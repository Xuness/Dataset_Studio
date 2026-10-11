//! A Windows venv python.exe is a redirector which starts another python.exe.
//! Own both before sending work, so killing just the redirector cannot leave a
//! writer alive after the engine releases the material directory's lock.
use std::process::Child;
use studio_domain::{Error, Result};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectExtendedLimitInformation, SetInformationJobObject,
        },
        Threading::{
            OpenProcess, OpenThread, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
            PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, ResumeThread, THREAD_SUSPEND_RESUME,
            WaitForSingleObject,
        },
    },
};

pub(super) struct ProcessTree {
    job: usize,
    real: usize,
}
fn error(stage: &str) -> Error {
    Error::new(
        "VISUAL_UNAVAILABLE",
        format!(
            "视觉进程托管失败（{stage}）：{}",
            std::io::Error::last_os_error()
        ),
    )
}
impl ProcessTree {
    pub fn prepare(child: &Child) -> Result<Self> {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(error("create job"));
            }
            let guard = Self {
                job: job as usize,
                real: 0,
            };
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
            {
                return Err(error("set limits"));
            }
            // std::process::Child may expose a handle without SET_QUOTA. Open
            // our known child explicitly with the rights needed for assignment.
            let launcher = OpenProcess(
                PROCESS_SET_QUOTA
                    | PROCESS_TERMINATE
                    | PROCESS_SYNCHRONIZE
                    | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                child.id(),
            );
            if launcher.is_null() {
                return Err(error("open launcher"));
            }
            let assigned = AssignProcessToJobObject(job, launcher);
            let assignment_error = (assigned == 0).then(|| error("assign launcher"));
            CloseHandle(launcher);
            if let Some(error) = assignment_error {
                return Err(error);
            }
            Ok(guard)
        }
    }
    /// The redirector must inherit our job before it creates its own nested job
    /// for Python. Assigning both after startup forms incompatible job siblings.
    pub fn resume(pid: u32) -> Result<()> {
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(error("thread snapshot"));
            }
            let mut entry: THREADENTRY32 = std::mem::zeroed();
            entry.dwSize = std::mem::size_of_val(&entry) as u32;
            let mut thread_ids = Vec::new();
            let mut found = Thread32First(snapshot, &mut entry);
            while found != 0 {
                if entry.th32OwnerProcessID == pid {
                    thread_ids.push(entry.th32ThreadID);
                }
                found = Thread32Next(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
            if thread_ids.len() != 1 {
                return Err(Error::new(
                    "VISUAL_UNAVAILABLE",
                    "无法唯一定位尚未启动的 Python 主线程",
                ));
            }
            let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, thread_ids[0]);
            if thread.is_null() {
                return Err(error("open startup thread"));
            }
            let previous = ResumeThread(thread);
            let resume_error = (previous == u32::MAX).then(|| error("resume startup thread"));
            CloseHandle(thread);
            if let Some(error) = resume_error {
                return Err(error);
            }
            if previous != 1 {
                return Err(Error::new("VISUAL_UNAVAILABLE", "解释器启动挂起计数不一致"));
            }
        }
        Ok(())
    }
    pub fn adopt(&mut self, child: &Child, pid: u32, parent: u32) -> Result<()> {
        if pid != child.id() && parent != child.id() {
            return Err(Error::new(
                "VISUAL_PROTOCOL",
                "Python 进程不是所启动解释器的子进程",
            ));
        }
        unsafe {
            let real = OpenProcess(
                PROCESS_TERMINATE | PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                pid,
            );
            if real.is_null() {
                return Err(error("open python"));
            }
            self.real = real as usize;
            let mut inside = 0;
            if IsProcessInJob(real, self.job as HANDLE, &mut inside) == 0 {
                return Err(error("query python job"));
            }
            if inside == 0 {
                return Err(Error::new(
                    "VISUAL_PROTOCOL",
                    "Python 子进程没有继承引擎托管关系",
                ));
            }
        }
        Ok(())
    }
    pub fn stop(&mut self) -> Result<()> {
        unsafe {
            if self.job != 0 {
                CloseHandle(self.job as HANDLE);
                self.job = 0;
            }
            if self.real != 0 && WaitForSingleObject(self.real as HANDLE, 30000) != WAIT_OBJECT_0 {
                return Err(Error::new(
                    "VISUAL_BUSY",
                    "视觉 Python 进程仍在退出，暂不能启动新写入者",
                ));
            }
        }
        Ok(())
    }
}
impl Drop for ProcessTree {
    fn drop(&mut self) {
        let _ = self.stop();
        if self.real != 0 {
            unsafe {
                CloseHandle(self.real as HANDLE);
            }
        }
    }
}
