use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use studio_application::lake_updates::LakeUpdateBackend;
use studio_domain::{Error, Result, lake_updates::*};

pub struct Backend {
    config_path: PathBuf,
    runtime: Mutex<Option<LakeUpdateRuntime>>,
    worker: Mutex<Option<Child>>,
    bundle: Mutex<Option<PathBuf>>,
    stopped: AtomicBool,
}
fn lock_error<T>(_: std::sync::PoisonError<T>) -> Error {
    Error::new("UPDATE_UNAVAILABLE", "数据湖更新控制器不可用")
}
impl Backend {
    pub async fn supervise(self: std::sync::Arc<Self>) {
        while !self.stopped.load(Ordering::Acquire) {
            if self.configured() {
                let backend = self.clone();
                let _ = tokio::task::spawn_blocking(move || backend.ensure_worker()).await;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
    pub fn new(root: PathBuf) -> Self {
        let config_path = root.join("lake-update-runtime.json");
        let runtime = fs::read(&config_path)
            .ok()
            .and_then(|v| serde_json::from_slice::<Value>(&v).ok())
            .and_then(|mut v| {
                v.as_object_mut()?.remove("store_root");
                serde_json::from_value(v).ok()
            });
        Self {
            config_path,
            runtime: Mutex::new(runtime),
            worker: Mutex::new(None),
            bundle: Mutex::new(None),
            stopped: AtomicBool::new(false),
        }
    }
    fn configuration(&self) -> Result<LakeUpdateRuntime> {
        self.runtime
            .lock()
            .map_err(lock_error)?
            .clone()
            .ok_or_else(|| Error::new("UPDATE_UNAVAILABLE", "请先配置数据湖更新运行环境"))
    }
    fn command(&self, runtime: &LakeUpdateRuntime, mode: &str) -> Result<Command> {
        let mut bundle = self.bundle.lock().map_err(lock_error)?;
        if bundle.is_none() {
            *bundle = Some(crate::lake_worker_bundle::install(
                self.config_path.parent().unwrap(),
            )?);
        }
        let worker_root = bundle.as_ref().unwrap();
        let mut command = Command::new(&runtime.python);
        command
            .arg("-I")
            .arg("-X")
            .arg("utf8")
            .arg(worker_root.join("worker.py"))
            .arg("--root")
            .arg(&runtime.state_root)
            .arg("--mode")
            .arg(mode)
            .current_dir(worker_root)
            .env_remove("PYTHONPATH")
            .env("PYTHONIOENCODING", "utf-8");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        Ok(command)
    }
    pub fn ensure_worker(&self) -> Result<()> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(Error::new("UPDATE_UNAVAILABLE", "更新控制器正在停止"));
        }
        let runtime = self.configuration()?;
        let mut slot = self.worker.lock().map_err(lock_error)?;
        if let Some(child) = slot.as_mut()
            && child.try_wait().map_err(Error::io)?.is_none()
        {
            return Ok(());
        }
        let child = self
            .command(&runtime, "serve")?
            .arg("--watch-stdin")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| Error::new("UPDATE_UNAVAILABLE", "无法启动数据湖更新运行器"))?;
        *slot = Some(child);
        Ok(())
    }
}
impl LakeUpdateBackend for Backend {
    fn configured(&self) -> bool {
        self.runtime.lock().is_ok_and(|v| v.is_some())
    }
    fn configure(&self, mut runtime: LakeUpdateRuntime) -> Result<()> {
        if !runtime.python.is_absolute()
            || !runtime.state_root.is_absolute()
            || !runtime.python.is_file()
        {
            return Err(Error::invalid(
                "更新运行环境需要有效的 Python 与状态目录绝对路径；运行器随 Studio 提供",
            ));
        }
        runtime.python = runtime.python.canonicalize().map_err(Error::io)?;
        fs::create_dir_all(&runtime.state_root).map_err(Error::io)?;
        runtime.state_root = runtime.state_root.canonicalize().map_err(Error::io)?;
        if runtime.state_root == runtime.python
            || runtime
                .state_root
                .starts_with(self.config_path.parent().unwrap().join("lake-worker"))
        {
            return Err(Error::invalid("状态目录不能覆盖程序文件"));
        }
        {
            let mut stored = self.runtime.lock().map_err(lock_error)?;
            if stored.as_ref().is_some_and(|old| old != &runtime) {
                return Err(Error::new(
                    "UPDATE_CONFLICT",
                    "已有更新运行环境；迁移状态目录前需要停止并核对任务",
                ));
            }
            // A controller belongs to one app registry; multiple projects use the same registry.
            let owner_path = runtime.state_root.join("studio-owner.json");
            let owner = json!({"registry": self.config_path.parent()});
            if owner_path.exists() {
                let old: Value = serde_json::from_slice(&fs::read(&owner_path).map_err(Error::io)?)
                    .map_err(Error::io)?;
                if old != owner {
                    return Err(Error::new(
                        "UPDATE_CONFLICT",
                        "状态目录已经属于另一个 Studio 应用目录",
                    ));
                }
            } else {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&owner_path)
                    .map_err(Error::io)?;
                file.write_all(&serde_json::to_vec(&owner).map_err(Error::io)?)
                    .map_err(Error::io)?;
                file.sync_all().map_err(Error::io)?;
            }
            studio_storage::atomic_json(&self.config_path, &runtime)?;
            *stored = Some(runtime);
        }
        self.ensure_worker()
    }
    fn execute(&self, operation: LakeUpdateOperation, arguments: Value) -> Result<Value> {
        let runtime = self.configuration()?;
        self.ensure_worker()?;
        let request = serde_json::to_vec(
            &json!({"protocol_version":1,"command":operation.name(),"arguments":arguments}),
        )
        .map_err(Error::io)?;
        if request.len() > 2 * 1024 * 1024 {
            return Err(Error::invalid("更新请求过大，请使用分页范围"));
        }
        let mut child = self
            .command(&runtime, "rpc")?
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| Error::new("UPDATE_UNAVAILABLE", "无法启动更新控制请求"))?;
        let mut input = child
            .stdin
            .take()
            .ok_or_else(|| Error::new("UPDATE_UNAVAILABLE", "更新输入通道不可用"))?;
        input
            .write_all(&request)
            .map_err(|_| Error::new("UPDATE_UNAVAILABLE", "更新输入通道中断"))?;
        drop(input);
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::new("UPDATE_UNAVAILABLE", "更新输出通道不可用"))?;
        let reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout
                .take(8 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });
        let deadline = Instant::now() + Duration::from_secs(90);
        while child.try_wait().map_err(Error::io)?.is_none() {
            if Instant::now() >= deadline || self.stopped.load(Ordering::Acquire) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(Error::new(
                    "UPDATE_UNAVAILABLE",
                    "更新控制请求超时；后台任务仍由持久检查点管理",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let bytes = reader
            .join()
            .map_err(|_| Error::new("UPDATE_PROTOCOL", "更新输出读取失败"))?
            .map_err(|_| Error::new("UPDATE_PROTOCOL", "更新输出读取失败"))?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(Error::new("UPDATE_PROTOCOL", "更新响应超出有界大小"));
        }
        let reply: Value = serde_json::from_slice(&bytes)
            .map_err(|_| Error::new("UPDATE_PROTOCOL", "更新运行器响应格式无效"))?;
        if reply["protocol_version"] != 1 {
            return Err(Error::new("UPDATE_PROTOCOL", "更新协议版本不匹配"));
        }
        if reply["ok"] != true {
            let code = match reply["error"]["code"].as_str().unwrap_or_default() {
                "INVALID_INPUT" => "INVALID_INPUT",
                "NOT_FOUND" => "NOT_FOUND",
                "IDEMPOTENCY_CONFLICT" => "IDEMPOTENCY_CONFLICT",
                "REVISION_CONFLICT" => "REVISION_CONFLICT",
                "SOURCE_ID_MISMATCH" => "SOURCE_ID_MISMATCH",
                "SOURCE_CHANGED" => "SOURCE_CHANGED",
                "SOURCE_LOCATION_CONFLICT" => "SOURCE_LOCATION_CONFLICT",
                "UPDATE_CONFLICT" => "UPDATE_CONFLICT",
                "UPDATE_UNSUPPORTED" => "UPDATE_UNSUPPORTED",
                "UPDATE_CREDENTIAL_REQUIRED" => "UPDATE_CREDENTIAL_REQUIRED",
                "UPDATE_NETWORK" => "UPDATE_NETWORK",
                "UPDATE_REMOTE_ERROR" => "UPDATE_REMOTE_ERROR",
                "UPDATE_BASELINE_REQUIRED" => "UPDATE_BASELINE_REQUIRED",
                "UPDATE_RESPONSE_INVALID" => "UPDATE_RESPONSE_INVALID",
                _ => "UPDATE_UNAVAILABLE",
            };
            return Err(Error::new(
                code,
                reply["error"]["message"]
                    .as_str()
                    .unwrap_or("数据湖更新请求失败"),
            ));
        }
        Ok(reply["result"].clone())
    }
    fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Ok(mut slot) = self.worker.lock()
            && let Some(mut child) = slot.take()
        {
            drop(child.stdin.take());
            let deadline = Instant::now() + Duration::from_secs(8);
            while child.try_wait().is_ok_and(|status| status.is_none()) && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        self.shutdown();
    }
}
