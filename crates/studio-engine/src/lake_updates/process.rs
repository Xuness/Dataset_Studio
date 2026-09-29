//! Bounded, private worker transport. Never persist raw Python output or exception text.
use super::*;
use std::io::{BufRead, BufReader};
use std::sync::mpsc;

pub(super) fn unavailable(message: &str) -> Error {
    Error::new("UPDATE_UNAVAILABLE", message)
}
pub(super) fn reply(bytes: &[u8]) -> Result<Value> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| {
        Error::new(
            "UPDATE_PROTOCOL",
            "更新运行器响应格式无效；请检查 Python 依赖",
        )
    })?;
    if value["protocol_version"] != 1 {
        return Err(Error::new("UPDATE_PROTOCOL", "更新协议版本不匹配"));
    }
    if value["ok"] != true {
        let code = match value["error"]["code"].as_str().unwrap_or_default() {
            "INVALID_INPUT" => "INVALID_INPUT",
            "NOT_FOUND" => "NOT_FOUND",
            "IDEMPOTENCY_CONFLICT" => "IDEMPOTENCY_CONFLICT",
            "REVISION_CONFLICT" => "REVISION_CONFLICT",
            "SOURCE_ID_MISMATCH" => "SOURCE_ID_MISMATCH",
            "SOURCE_CHANGED" => "SOURCE_CHANGED",
            "SOURCE_LOCATION_CONFLICT" => "SOURCE_LOCATION_CONFLICT",
            "UPDATE_CONFLICT" => "UPDATE_CONFLICT",
            "UPDATE_PROTOCOL" => "UPDATE_PROTOCOL",
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
            value["error"]["message"]
                .as_str()
                .unwrap_or("数据湖更新请求失败"),
        ));
    }
    Ok(value["result"].clone())
}
pub(super) fn handshake(bytes: &[u8]) -> Result<()> {
    // Do not surface arbitrary startup output, even if it resembles a worker error.
    let value = reply(bytes).map_err(|_| {
        Error::new(
            "UPDATE_PROTOCOL",
            "运行环境握手失败；请检查依赖、状态目录和运行器版本",
        )
    })?;
    if value["worker_version"] != "0.2.0" || value["runtime_check"] != 1 {
        return Err(Error::new("UPDATE_PROTOCOL", "运行器版本不兼容"));
    }
    Ok(())
}
pub(super) fn request(
    mut command: Command,
    input: &[u8],
    stopped: &AtomicBool,
    timeout: Duration,
) -> Result<Vec<u8>> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| unavailable("无法启动 Python；请更换解释器或修复运行依赖"))?;
    let result = (|| {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input)
            .map_err(|_| unavailable("更新输入通道中断"))?;
        let stdout = child.stdout.take().unwrap();
        let reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout
                .take(8 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });
        let deadline = Instant::now() + timeout;
        while child.try_wait().map_err(Error::io)?.is_none() {
            if Instant::now() >= deadline || stopped.load(Ordering::Acquire) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(unavailable(
                    "更新控制请求超时或正在停止；已保存的任务保持可恢复",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let bytes = reader
            .join()
            .map_err(|_| unavailable("更新输出读取失败"))?
            .map_err(Error::io)?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(Error::new("UPDATE_PROTOCOL", "更新响应超出有界大小"));
        }
        Ok(bytes)
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}
pub(super) fn prepared(mut command: Command, stopped: &AtomicBool) -> Result<Child> {
    let mut child = command
        .arg("--watch-stdin")
        .arg("--wait-activate")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| unavailable("无法启动数据湖更新运行器"))?;
    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = BufReader::new(stdout.take(65537))
            .read_until(b'\n', &mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    let result = (|| {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if stopped.load(Ordering::Acquire) || Instant::now() >= deadline {
                return Err(unavailable("运行器启动超时或正在停止"));
            }
            match receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(bytes) => {
                    handshake(&bytes.map_err(Error::io)?)?;
                    if child.try_wait().map_err(Error::io)?.is_some() {
                        return Err(unavailable("运行器在启动后退出"));
                    }
                    return Ok(());
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(unavailable("运行器启动通道中断"));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = reader.join();
    result.map(|_| child)
}
pub(super) fn activate(child: &mut Child) -> Result<()> {
    child
        .stdin
        .as_mut()
        .ok_or_else(|| unavailable("运行器启动通道不可用"))?
        .write_all(b"1")
        .map_err(|_| unavailable("运行器在启动后退出"))
}
pub(super) fn stop(slot: &mut Option<Child>, force: bool) -> Result<()> {
    if let Some(child) = slot.as_mut() {
        drop(child.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(8);
        while child.try_wait().map_err(Error::io)?.is_none() {
            if Instant::now() >= deadline {
                if !force {
                    return Err(Error::new(
                        "UPDATE_CONFLICT",
                        "运行器仍在停止；请等待当前批次退出后重试",
                    ));
                }
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    *slot = None;
    Ok(())
}
