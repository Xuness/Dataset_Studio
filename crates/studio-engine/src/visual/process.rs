use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};
use studio_application::visual::VisualExecutor;
use studio_domain::{Error, Result, new_id, visual::VisualRuntimeConfig};

type Waiting = Arc<Mutex<HashMap<String, mpsc::SyncSender<Result<Value>>>>>;
pub struct Worker {
    child: Mutex<Child>,
    input: Mutex<ChildStdin>,
    waiting: Waiting,
    #[cfg(windows)]
    tree: Mutex<Option<super::process_tree::ProcessTree>>,
}
fn unavailable(e: impl std::fmt::Display) -> Error {
    Error::new("VISUAL_UNAVAILABLE", e.to_string())
}

impl Worker {
    pub fn start(root: &Path, config: &VisualRuntimeConfig) -> Result<Arc<Self>> {
        let bundle = super::bundle::install(root)?;
        fs::create_dir_all(root.join("logs")).map_err(Error::io)?;
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("logs/visual-worker.log"))
            .map_err(Error::io)?;
        let mut command = Command::new(&config.python);
        command
            .args(["-I", "-X", "utf8"])
            .arg(bundle.join("worker.py"))
            .arg("--root")
            .arg(&config.materials_root)
            .arg("--device")
            .arg(&config.device)
            .current_dir(&bundle)
            .env_remove("PYTHONPATH")
            .env("PYTHONIOENCODING", "utf-8")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(log);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000 | 0x00000004); // CREATE_NO_WINDOW | CREATE_SUSPENDED
        }
        let mut child = command.spawn().map_err(unavailable)?;
        #[cfg(windows)]
        let tree = super::process_tree::ProcessTree::prepare(&child)?;
        #[cfg(windows)]
        super::process_tree::ProcessTree::resume(child.id())?;
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let waiting: Waiting = Arc::new(Mutex::new(HashMap::new()));
        let slots = waiting.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                let mut bytes = Vec::new();
                let read = Read::take(&mut reader, (2 << 20) + 1).read_until(b'\n', &mut bytes);
                if read.is_err()
                    || bytes.is_empty()
                    || bytes.len() > 2 << 20
                    || !bytes.ends_with(b"\n")
                {
                    break;
                }
                let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
                    break;
                };
                let Some(id) = value["id"].as_str() else {
                    break;
                };
                if let Ok(mut pending) = slots.lock()
                    && let Some(sender) = pending.remove(id)
                {
                    let result = if let Some(message) = value["error"].as_str() {
                        Err(Error::new("VISUAL_EXECUTION", message))
                    } else {
                        Ok(value["result"].clone())
                    };
                    let _ = sender.send(result);
                }
            }
            if let Ok(mut pending) = slots.lock() {
                for (_, sender) in pending.drain() {
                    let _ = sender.send(Err(unavailable("视觉服务已退出；当前批次可恢复")));
                }
            }
        });
        let worker = Arc::new(Self {
            child: Mutex::new(child),
            input: Mutex::new(input),
            waiting,
            #[cfg(windows)]
            tree: Mutex::new(Some(tree)),
        });
        let hello = worker.request("ping", json!({}))?;
        if hello["protocol_version"] != 1 {
            return Err(Error::new("VISUAL_PROTOCOL", "视觉服务协议不匹配"));
        }
        #[cfg(windows)]
        {
            let pid = hello["pid"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| unavailable("Python PID 无效"))?;
            let parent = hello["parent_pid"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| unavailable("Python 父进程 PID 无效"))?;
            let child = worker.child.lock().map_err(unavailable)?;
            worker
                .tree
                .lock()
                .map_err(unavailable)?
                .as_mut()
                .expect("created process tree")
                .adopt(&child, pid, parent)?;
        }
        Ok(worker)
    }
    pub fn alive(&self) -> bool {
        self.child
            .lock()
            .is_ok_and(|mut c| c.try_wait().is_ok_and(|s| s.is_none()))
    }
    pub fn stop(&self) -> Result<()> {
        #[cfg(windows)]
        if let Some(tree) = self.tree.lock().map_err(unavailable)?.as_mut() {
            tree.stop()?;
        }
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
        Ok(())
    }
}
impl VisualExecutor for Worker {
    fn request(&self, operation: &str, args: Value) -> Result<Value> {
        if !self.alive() {
            return Err(unavailable("视觉服务未运行"));
        }
        let id = new_id();
        let mut bytes =
            serde_json::to_vec(&json!({"id":id,"op":operation,"args":args})).map_err(Error::io)?;
        if bytes.len() >= 2 << 20 {
            return Err(Error::invalid("视觉控制请求过大"));
        }
        bytes.push(b'\n');
        let (sender, receiver) = mpsc::sync_channel(1);
        {
            let mut waiting = self.waiting.lock().map_err(unavailable)?;
            if waiting.len() >= 16 {
                return Err(Error::new("VISUAL_BUSY", "视觉执行队列已满"));
            }
            waiting.insert(id.clone(), sender);
        }
        let sent = self
            .input
            .lock()
            .map_err(unavailable)
            .and_then(|mut input| {
                input
                    .write_all(&bytes)
                    .and_then(|_| input.flush())
                    .map_err(unavailable)
            });
        if let Err(error) = sent {
            self.waiting.lock().map_err(unavailable)?.remove(&id);
            return Err(error);
        }
        match receiver.recv_timeout(Duration::from_secs(if operation == "index" {
            24 * 60 * 60
        } else {
            15 * 60
        })) {
            Ok(result) => result,
            Err(_) => {
                // A timed-out writer must be dead before another writer starts.
                self.stop()?;
                Err(unavailable("视觉操作超时；未确认批次保留以供恢复"))
            }
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
