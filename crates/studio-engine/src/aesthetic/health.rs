use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Mutex,
};
use studio_domain::{Error, Result};

#[derive(Default)]
struct Fault {
    volume: String,
    code: String,
    pending: HashSet<String>,
}
#[derive(Default)]
struct State {
    shutdown: bool,
    faults: HashMap<String, Fault>,
}
#[derive(Default)]
pub(super) struct DispatchGate(Mutex<State>);

pub(super) fn storage_failure(error: &Error) -> bool {
    matches!(
        error.code,
        "EVALUATION_BUSY"
            | "EVALUATION_STORAGE_FULL"
            | "EVALUATION_STORAGE_IO"
            | "EVALUATION_CORRUPT"
            | "EVALUATION_WRITER_EXITED"
            | "DATABASE_ERROR"
            | "IO_ERROR"
    )
}
fn affects(fault_pid: &str, fault: &Fault, pid: &str, volume: &str) -> bool {
    fault_pid == pid
        || matches!(fault.code.as_str(), "EVALUATION_STORAGE_IO" | "IO_ERROR")
        || (fault.code == "EVALUATION_STORAGE_FULL" && fault.volume == volume)
}
fn check(state: &State, pid: &str, volume: &str) -> Result<()> {
    if state.shutdown {
        return Err(Error::new("CANCELLED", "引擎正在关闭"));
    }
    if let Some((_, fault)) = state
        .faults
        .iter()
        .find(|(key, f)| affects(key, f, pid, volume))
    {
        return Err(Error::new(
            "EVALUATION_STORAGE_UNHEALTHY",
            format!("评审存储暂停准入：{}；在途返回正在保留/排空", fault.code),
        ));
    }
    Ok(())
}
fn record(state: &mut State, pid: &str, volume: &str, attempt: Option<&str>, error: &Error) {
    let fault = state.faults.entry(pid.into()).or_default();
    fault.volume = volume.into();
    // Never downgrade a shared storage fault to a local queue failure before a probe succeeds.
    if !matches!(
        fault.code.as_str(),
        "EVALUATION_STORAGE_IO" | "IO_ERROR" | "EVALUATION_CORRUPT" | "EVALUATION_WRITER_EXITED"
    ) && (fault.code != "EVALUATION_STORAGE_FULL"
        || matches!(error.code, "EVALUATION_STORAGE_IO" | "IO_ERROR"))
    {
        fault.code = error.code.into();
    }
    if let Some(attempt) = attempt {
        fault.pending.insert(attempt.into());
    }
}
impl DispatchGate {
    /// Holds the same mutex used by fault closure/shutdown through the durable
    /// attempt commit. Once this succeeds the request is in-flight and must drain.
    pub fn commit(
        &self,
        pid: &str,
        volume: &str,
        action: impl FnOnce() -> Result<bool>,
    ) -> Result<bool> {
        let mut state = self.0.lock().map_err(|_| Error::io("评审准入锁不可用"))?;
        check(&state, pid, volume)?;
        let result = action();
        if let Err(e) = &result
            && storage_failure(e)
        {
            record(&mut state, pid, volume, None, e);
        }
        result
    }
    pub fn check(&self, pid: &str, volume: &str) -> Result<()> {
        check(
            &*self.0.lock().map_err(|_| Error::io("评审准入锁不可用"))?,
            pid,
            volume,
        )
    }
    pub fn fault(&self, pid: &str, volume: &str, attempt: Option<&str>, error: &Error) {
        if let Ok(mut state) = self.0.lock() {
            record(&mut state, pid, volume, attempt, error);
        }
    }
    /// Called only after both the outcome commit and a fresh write probe succeed.
    pub fn recovered(&self, pid: &str, attempt: Option<&str>) {
        if let Ok(mut state) = self.0.lock()
            && let Some(fault) = state.faults.get_mut(pid)
        {
            if let Some(attempt) = attempt {
                fault.pending.remove(attempt);
            }
            if fault.pending.is_empty()
                && !matches!(
                    fault.code.as_str(),
                    "EVALUATION_CORRUPT" | "EVALUATION_WRITER_EXITED"
                )
            {
                state.faults.remove(pid);
            }
        }
    }
    pub fn shutdown(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.shutdown = true;
        }
    }
    pub fn is_shutdown(&self) -> bool {
        self.0.lock().map_or(true, |v| v.shutdown)
    }
    pub fn metrics(&self, pid: &str, volume: &str) -> (String, Option<String>, u64) {
        let Ok(state) = self.0.lock() else {
            return (
                "manual_recovery_required".into(),
                Some("INTERNAL_ERROR".into()),
                0,
            );
        };
        let pending = state.faults.get(pid).map_or(0, |v| v.pending.len() as u64);
        if state.shutdown {
            return ("shutdown".into(), None, pending);
        }
        match state
            .faults
            .iter()
            .find(|(key, f)| affects(key, f, pid, volume))
        {
            Some((_, fault)) => (
                if matches!(
                    fault.code.as_str(),
                    "EVALUATION_CORRUPT" | "EVALUATION_WRITER_EXITED" | "DATABASE_ERROR"
                ) {
                    "manual_recovery_required"
                } else {
                    "storage_backpressure"
                }
                .into(),
                Some(fault.code.clone()),
                pending,
            ),
            None => ("healthy".into(), None, pending),
        }
    }
}

/// FULL is a volume fault. IOERR has uncertain scope and conservatively stops
/// all ledgers until the affected writer is healthy; BUSY/corruption stay local.
pub(super) fn storage_volume(path: &Path) -> Result<String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            GetVolumeNameForVolumeMountPointW, GetVolumePathNameW,
        };
        let path = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let mut mount = vec![0u16; 32768];
        let mut volume = vec![0u16; 1024];
        // Buffers are owned, writable and NUL-terminated; lengths are in UTF-16 units.
        unsafe {
            if GetVolumePathNameW(path.as_ptr(), mount.as_mut_ptr(), mount.len() as u32) == 0 {
                return Err(Error::io(std::io::Error::last_os_error()));
            }
            if GetVolumeNameForVolumeMountPointW(
                mount.as_ptr(),
                volume.as_mut_ptr(),
                volume.len() as u32,
            ) == 0
            {
                // Network volumes may not expose a volume GUID. The canonical share root is stable.
                return Ok(String::from_utf16_lossy(
                    &mount[..mount.iter().position(|v| *v == 0).unwrap_or(mount.len())],
                )
                .to_lowercase());
            }
        }
        Ok(String::from_utf16_lossy(
            &volume[..volume.iter().position(|v| *v == 0).unwrap_or(volume.len())],
        )
        .to_lowercase())
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(std::fs::metadata(path)
            .map_err(Error::io)?
            .dev()
            .to_string())
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = path;
        Ok("shared_storage".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn faults_have_explicit_domains_and_wait_for_every_retained_outcome() {
        let gate = DispatchGate::default();
        gate.fault(
            "a",
            "disk1",
            Some("first"),
            &Error::new("EVALUATION_BUSY", "busy"),
        );
        assert!(gate.check("b", "disk1").is_ok());
        gate.fault(
            "a",
            "disk1",
            Some("second"),
            &Error::new("EVALUATION_STORAGE_FULL", "full"),
        );
        assert!(gate.check("b", "disk1").is_err());
        assert!(gate.check("c", "disk2").is_ok());
        gate.recovered("a", Some("first"));
        assert!(gate.check("b", "disk1").is_err());
        gate.recovered("a", Some("second"));
        assert!(gate.check("a", "disk1").is_ok());
        gate.fault(
            "a",
            "disk1",
            None,
            &Error::new("EVALUATION_STORAGE_IO", "io"),
        );
        assert!(gate.check("c", "disk2").is_err());
        gate.recovered("a", None);
        gate.shutdown();
        gate.recovered("a", None);
        assert!(gate.check("a", "disk1").is_err());
    }
    #[test]
    fn fault_closure_and_send_commit_are_serialized() {
        let gate = std::sync::Arc::new(DispatchGate::default());
        let (started, ready) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let sender = gate.clone();
        let send = std::thread::spawn(move || {
            sender.commit("p", "disk", || {
                started.send(()).unwrap();
                wait.recv().unwrap();
                Ok(true)
            })
        });
        ready.recv().unwrap();
        let closer = gate.clone();
        let close = std::thread::spawn(move || {
            closer.fault("p", "disk", None, &Error::new("EVALUATION_BUSY", "busy"))
        });
        release.send(()).unwrap();
        assert!(send.join().unwrap().unwrap());
        close.join().unwrap();
        let mut reached = false;
        assert!(
            gate.commit("p", "disk", || {
                reached = true;
                Ok(true)
            })
            .is_err()
        );
        assert!(!reached);
    }
}
