use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use studio_application::lake_updates::LakeUpdateBackend;
use studio_domain::{Error, Result, lake_updates::*};
mod process;
#[cfg(test)]
mod tests;

pub struct Backend {
    config_path: PathBuf,
    // Configuration/shutdown excludes RPCs and supervisor launches, without
    // serializing unrelated RPCs (in particular pause/cancel).
    gate: RwLock<()>,
    runtime: Mutex<Option<LakeUpdateRuntime>>,
    worker: Mutex<Option<Child>>,
    bundle: Mutex<Option<PathBuf>>,
    health: Mutex<LakeUpdateHealth>,
    stopped: AtomicBool,
}
fn lock_error<T>(_: std::sync::PoisonError<T>) -> Error {
    Error::new("UPDATE_UNAVAILABLE", "数据湖更新控制器不可用")
}
fn millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
impl Backend {
    pub async fn supervise(
        self: std::sync::Arc<Self>,
        store: std::sync::Arc<studio_storage::SqliteStore>,
    ) {
        while !self.stopped.load(Ordering::Acquire) {
            if self.configured() {
                let backend = self.clone();
                let store = store.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    backend.ensure_worker()?;
                    crate::lake_locations::recover(&store, backend.as_ref())
                })
                .await;
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
        let mut health = LakeUpdateHealth {
            state: "unconfigured".into(),
            ..Default::default()
        };
        if runtime.is_some() {
            health.state = "starting".into();
        } else if config_path.exists() {
            health.state = "failed".into();
            health.error_code = Some("UPDATE_CONFIGURATION".into());
            health.message = Some("已保存的运行环境配置损坏；请重新配置原状态目录".into());
        }
        Self {
            config_path,
            gate: RwLock::new(()),
            runtime: Mutex::new(runtime),
            worker: Mutex::new(None),
            bundle: Mutex::new(None),
            health: Mutex::new(health),
            stopped: AtomicBool::new(false),
        }
    }
    fn configuration(&self) -> Result<LakeUpdateRuntime> {
        self.runtime
            .lock()
            .map_err(lock_error)?
            .clone()
            .ok_or_else(|| process::unavailable("请先配置数据湖更新运行环境"))
    }
    fn command(&self, runtime: &LakeUpdateRuntime, mode: &str) -> Result<Command> {
        let mut bundle = self.bundle.lock().map_err(lock_error)?;
        if bundle.is_none() {
            *bundle = Some(crate::lake_worker_bundle::install(
                self.config_path.parent().unwrap(),
            )?);
        }
        let root = bundle.as_ref().unwrap();
        let mut command = Command::new(&runtime.python);
        command
            .arg("-I")
            .arg("-X")
            .arg("utf8")
            .arg(root.join("worker.py"))
            .arg("--root")
            .arg(&runtime.state_root)
            .arg("--mode")
            .arg(mode)
            .current_dir(root)
            .env_remove("PYTHONPATH")
            .env("PYTHONIOENCODING", "utf-8");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        Ok(command)
    }
    fn failure(&self, error: &Error) {
        if let Ok(mut health) = self.health.lock() {
            health.state = "failed".into();
            health.failures = health.failures.saturating_add(1);
            health.error_code = Some(error.code.into());
            // Never copy arbitrary Python/OS output into status or startup logs.
            health.message =
                Some("更新运行环境不可用；请检查解释器、依赖及状态目录后重新验证".into());
            health.next_retry_ms =
                Some(millis() + (5_000u64 << health.failures.min(6)).min(300_000));
            let path = self.config_path.with_file_name("lake-update-health.json");
            let mut events = fs::read(&path)
                .ok()
                .and_then(|v| serde_json::from_slice::<Vec<Value>>(&v).ok())
                .unwrap_or_default();
            events.push(json!({"at_ms":millis(),"code":error.code,"failures":health.failures}));
            if events.len() > 32 {
                events.drain(..events.len() - 32);
            }
            let _ = studio_storage::atomic_json(&path, &events);
        }
    }
    fn healthy(&self) {
        if let Ok(mut health) = self.health.lock() {
            *health = LakeUpdateHealth {
                state: "ready".into(),
                ..Default::default()
            };
        }
    }
    fn ensure_locked(&self) -> Result<()> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(process::unavailable("更新控制器正在停止"));
        }
        let runtime = self.configuration()?;
        let mut slot = self.worker.lock().map_err(lock_error)?;
        if let Some(child) = slot.as_mut() {
            if child.try_wait().map_err(Error::io)?.is_none() {
                return if child.stdin.is_some() {
                    Ok(())
                } else {
                    Err(Error::new("UPDATE_CONFLICT", "运行器仍在停止"))
                };
            }
            *slot = None;
            self.failure(&process::unavailable("运行器已退出"));
        }
        if self
            .health
            .lock()
            .map_err(lock_error)?
            .next_retry_ms
            .is_some_and(|t| t > millis())
        {
            return Err(process::unavailable(
                "运行器重试等待中；可在设置中立即重新验证",
            ));
        }
        let result = (|| {
            let mut child = process::prepared(self.command(&runtime, "serve")?, &self.stopped)?;
            if let Err(error) = process::activate(&mut child) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
            *slot = Some(child);
            self.healthy();
            Ok(())
        })();
        if let Err(error) = &result {
            self.failure(error);
        }
        result
    }
    pub fn ensure_worker(&self) -> Result<()> {
        let _gate = self.gate.read().map_err(lock_error)?;
        self.ensure_locked()
    }
    fn configure_locked(&self, mut runtime: LakeUpdateRuntime) -> Result<()> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(process::unavailable("更新控制器正在停止"));
        }
        if !runtime.python.is_absolute()
            || !runtime.state_root.is_absolute()
            || !runtime.python.is_file()
        {
            return Err(Error::invalid("需要有效的 Python 与状态目录绝对路径"));
        }
        runtime.python = runtime.python.canonicalize().map_err(Error::io)?;
        // Failed candidates never open state, acquire ownership, or stop the old worker.
        process::handshake(&process::request(
            self.command(&runtime, "check")?,
            &[],
            &self.stopped,
            Duration::from_secs(15),
        )?)?;
        fs::create_dir_all(&runtime.state_root).map_err(Error::io)?;
        runtime.state_root = runtime.state_root.canonicalize().map_err(Error::io)?;
        let app = self
            .config_path
            .parent()
            .unwrap()
            .canonicalize()
            .map_err(Error::io)?;
        if runtime.state_root.starts_with(app.join("lake-worker"))
            || runtime.python.starts_with(&runtime.state_root)
        {
            return Err(Error::invalid("状态目录不能覆盖运行环境程序文件"));
        }
        let previous = self.runtime.lock().map_err(lock_error)?.clone();
        if previous
            .as_ref()
            .is_some_and(|old| old.state_root != runtime.state_root)
        {
            return Err(Error::new(
                "UPDATE_CONFLICT",
                "更换解释器必须保留原状态目录；状态目录迁移需要独立处理",
            ));
        }
        let owner_path = runtime.state_root.join("studio-owner.json");
        let owner = json!({"registry": app});
        let new_owner = !owner_path.exists();
        if !new_owner {
            let saved: Value = serde_json::from_slice(&fs::read(&owner_path).map_err(Error::io)?)
                .map_err(Error::io)?;
            let matches = saved["registry"]
                .as_str()
                .is_some_and(|p| PathBuf::from(p).canonicalize().is_ok_and(|p| p == app));
            if !matches {
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
        let mut slot = self.worker.lock().map_err(lock_error)?;
        let result = (|| {
            process::stop(&mut slot, false)?;
            let mut candidate = process::prepared(self.command(&runtime, "serve")?, &self.stopped)?;
            let installed: Result<()> = (|| {
                studio_storage::atomic_json(&self.config_path, &runtime)?;
                process::activate(&mut candidate)?;
                Ok(())
            })();
            if let Err(error) = installed {
                let _ = candidate.kill();
                let _ = candidate.wait();
                if let Some(old) = &previous {
                    studio_storage::atomic_json(&self.config_path, old)?;
                } else if self.config_path.exists() {
                    fs::remove_file(&self.config_path).map_err(Error::io)?;
                }
                return Err(error);
            }
            *self.runtime.lock().map_err(lock_error)? = Some(runtime);
            *slot = Some(candidate);
            self.healthy();
            Ok(())
        })();
        if result.is_err() {
            // A previously saved runtime still owns its state if this request
            // had to repair a missing marker before failing.
            if new_owner && previous.is_none() {
                let _ = fs::remove_file(owner_path);
            }
            if slot.is_none()
                && let Some(old) = previous
                && let Ok(mut child) =
                    process::prepared(self.command(&old, "serve")?, &self.stopped)
            {
                if process::activate(&mut child).is_ok() {
                    *slot = Some(child);
                } else {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        }
        result
    }
}
impl LakeUpdateBackend for Backend {
    fn configured(&self) -> bool {
        self.runtime.lock().is_ok_and(|v| v.is_some())
    }
    fn health(&self) -> LakeUpdateHealth {
        let mut value = self.health.lock().map(|v| v.clone()).unwrap_or_default();
        if let Ok(runtime) = self.configuration() {
            value.python = Some(runtime.python);
            value.state_root = Some(runtime.state_root);
        }
        if self.stopped.load(Ordering::Acquire) {
            value.state = "stopping".into();
        }
        value
    }
    fn configure(&self, runtime: LakeUpdateRuntime) -> Result<()> {
        let _gate = self.gate.write().map_err(lock_error)?;
        let result = self.configure_locked(runtime);
        if let Err(error) = &result {
            self.failure(error);
            let running = self.worker.lock().is_ok_and(|mut slot| {
                slot.as_mut().is_some_and(|child| {
                    child.stdin.is_some() && child.try_wait().is_ok_and(|s| s.is_none())
                })
            });
            if (running || !self.configured())
                && let Ok(mut health) = self.health.lock()
            {
                health.next_retry_ms = None;
                health.message =
                    Some("候选运行环境未启用；原配置保持不变，请修改后重新验证".into());
                if running {
                    health.state = "ready".into();
                }
            }
        }
        result
    }
    fn execute(&self, operation: LakeUpdateOperation, arguments: Value) -> Result<Value> {
        let _gate = self.gate.read().map_err(lock_error)?;
        self.ensure_locked()?;
        let request = serde_json::to_vec(
            &json!({"protocol_version":1,"command":operation.name(),"arguments":arguments}),
        )
        .map_err(Error::io)?;
        if request.len() > 2 * 1024 * 1024 {
            return Err(Error::invalid("更新请求过大，请使用分页范围"));
        }
        process::reply(&process::request(
            self.command(&self.configuration()?, "rpc")?,
            &request,
            &self.stopped,
            Duration::from_secs(90),
        )?)
    }
    fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Ok(_gate) = self.gate.write()
            && let Ok(mut slot) = self.worker.lock()
        {
            let _ = process::stop(&mut slot, true);
        }
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        self.shutdown();
    }
}
