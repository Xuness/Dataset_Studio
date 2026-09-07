//! RAII binding to the pinned DuckDB 1.5.4 C API. No archive runtime dependency.
use std::{
    ffi::{CStr, CString, c_char, c_void},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use studio_domain::{Error, Result};
type Handle = *mut c_void;
#[repr(C)]
struct RawResult {
    columns: u64,
    rows: u64,
    changed: u64,
    deprecated_columns: Handle,
    error: *mut c_char,
    internal: Handle,
}
macro_rules! api {
    ($($name:ident: $ty:ty),+ $(,)?) => {
        struct Api { $($name:$ty,)+ _library:libloading::Library }
        impl Api { unsafe fn load(path:&Path)->Result<Self> {
            let library=unsafe {libloading::Library::new(path)}.map_err(|e|Error::new("METADATA_RUNTIME_UNAVAILABLE",format!("元数据运行库不可用：{e}。请运行 tooling/setup-duckdb.ps1")))?;
            Ok(Self { $($name: unsafe {*library.get::<$ty>(concat!("duckdb_",stringify!($name)).as_bytes()).map_err(Error::io)?},)+ _library:library })
        }}
    }
}
api! {
    library_version:unsafe extern "C" fn()->*const c_char,
    create_config:unsafe extern "C" fn(*mut Handle)->u32,
    set_config:unsafe extern "C" fn(Handle,*const c_char,*const c_char)->u32,
    destroy_config:unsafe extern "C" fn(*mut Handle),
    open_ext:unsafe extern "C" fn(*const c_char,*mut Handle,Handle,*mut *mut c_char)->u32,
    close:unsafe extern "C" fn(*mut Handle),
    connect:unsafe extern "C" fn(Handle,*mut Handle)->u32,
    disconnect:unsafe extern "C" fn(*mut Handle),
    interrupt:unsafe extern "C" fn(Handle),
    query:unsafe extern "C" fn(Handle,*const c_char,*mut RawResult)->u32,
    destroy_result:unsafe extern "C" fn(*mut RawResult),
    row_count:unsafe extern "C" fn(*mut RawResult)->u64,
    column_count:unsafe extern "C" fn(*mut RawResult)->u64,
    value_is_null:unsafe extern "C" fn(*mut RawResult,u64,u64)->bool,
    value_varchar:unsafe extern "C" fn(*mut RawResult,u64,u64)->*mut c_char,
    free:unsafe extern "C" fn(Handle),
}
struct QueryResult<'a> {
    raw: RawResult,
    api: &'a Api,
}
impl Drop for QueryResult<'_> {
    fn drop(&mut self) {
        unsafe { (self.api.destroy_result)(&mut self.raw) }
    }
}
pub(crate) struct Session {
    api: Arc<Api>,
    db: Handle,
    connection: Handle,
    stop: mpsc::Sender<()>,
    watchdog: Option<thread::JoinHandle<()>>,
    expired: Arc<AtomicBool>,
}
/// Retain the DLL, while releasing all database handles at the end of a request.
pub(crate) struct Runtime {
    path: PathBuf,
    api: Mutex<Option<Arc<Api>>>,
}
impl Runtime {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            api: Mutex::new(None),
        }
    }
    pub fn open(&self, path: &Path) -> Result<Session> {
        let started = Instant::now();
        let api = {
            let mut loaded = self
                .api
                .lock()
                .map_err(|_| Error::new("INTERNAL_ERROR", "元数据运行库状态锁不可用"))?;
            if loaded.is_none() {
                *loaded = Some(Arc::new(unsafe { Api::load(&self.path) }?));
            }
            loaded.as_ref().expect("loaded library").clone()
        };
        Session::with_api(
            api,
            path,
            true,
            Duration::from_secs(8).saturating_sub(started.elapsed()),
        )
    }
}
impl Session {
    pub fn open(dll: &Path, path: &Path) -> Result<Self> {
        Self::configured(dll, path, true, Duration::from_secs(8))
    }
    fn configured(dll: &Path, path: &Path, readonly: bool, budget: Duration) -> Result<Self> {
        Self::with_api(Arc::new(unsafe { Api::load(dll) }?), path, readonly, budget)
    }
    fn with_api(api: Arc<Api>, path: &Path, readonly: bool, budget: Duration) -> Result<Self> {
        let started = Instant::now();
        let version = unsafe { CStr::from_ptr((api.library_version)()) }.to_string_lossy();
        if version != "v1.5.4" {
            return Err(Error::new(
                "METADATA_RUNTIME_UNSUPPORTED",
                format!("需要 DuckDB v1.5.4，当前为 {version}"),
            ));
        }
        let path = CString::new(path.to_string_lossy().as_bytes()).map_err(Error::io)?;
        let mut config = std::ptr::null_mut();
        unsafe {
            if (api.create_config)(&mut config) != 0 {
                return Err(Error::new("SOURCE_FORMAT_ERROR", "无法创建 DuckDB 配置"));
            }
            for (key, value) in [
                (
                    "access_mode",
                    if readonly { "READ_ONLY" } else { "READ_WRITE" },
                ),
                ("threads", "1"),
                ("memory_limit", "256MB"),
                ("temp_directory", ""),
                ("enable_external_access", "false"),
                ("autoinstall_known_extensions", "false"),
                ("autoload_known_extensions", "false"),
            ] {
                let key = CString::new(key).expect("constant");
                let value = CString::new(value).expect("constant");
                if (api.set_config)(config, key.as_ptr(), value.as_ptr()) != 0 {
                    (api.destroy_config)(&mut config);
                    return Err(Error::new(
                        "METADATA_RUNTIME_UNSUPPORTED",
                        "DuckDB 配置不兼容",
                    ));
                }
            }
            let mut db = std::ptr::null_mut();
            let mut message = std::ptr::null_mut();
            let status = (api.open_ext)(path.as_ptr(), &mut db, config, &mut message);
            (api.destroy_config)(&mut config);
            if status != 0 {
                let message = if message.is_null() {
                    "无法打开分析索引".into()
                } else {
                    let s = CStr::from_ptr(message).to_string_lossy().into_owned();
                    (api.free)(message.cast());
                    s
                };
                if !db.is_null() {
                    (api.close)(&mut db);
                }
                let lower = message.to_lowercase();
                let code = if lower.contains("could not set lock")
                    || lower.contains("conflicting lock")
                    || lower.contains("database is locked")
                    || lower.contains("lock on file")
                    || lower.contains("file is already open")
                    || lower.contains("different configuration")
                {
                    "SOURCE_BUSY"
                } else if lower.contains("cannot open file") {
                    "SOURCE_UNAVAILABLE"
                } else {
                    "SOURCE_FORMAT_ERROR"
                };
                return Err(Error::new(code, message));
            }
            if !message.is_null() {
                (api.free)(message.cast());
            }
            let mut connection = std::ptr::null_mut();
            if (api.connect)(db, &mut connection) != 0 {
                (api.close)(&mut db);
                return Err(Error::new("SOURCE_FORMAT_ERROR", "无法连接分析索引"));
            }
            if started.elapsed() >= budget {
                (api.disconnect)(&mut connection);
                (api.close)(&mut db);
                return Err(Error::new(
                    "SOURCE_TIMEOUT",
                    "打开元数据索引已超过读取预算，请稍后重试",
                ));
            }
            let (stop, rx) = mpsc::channel();
            let expired = Arc::new(AtomicBool::new(false));
            let timed_out = expired.clone();
            let pointer = connection as usize;
            let interrupt = api.interrupt;
            let remaining = budget.saturating_sub(started.elapsed());
            let watchdog = thread::spawn(move || {
                if matches!(
                    rx.recv_timeout(remaining),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ) {
                    timed_out.store(true, Ordering::Release);
                    // DuckDB permits interruption from another thread. Drop joins before
                    // disconnecting or unloading this function pointer.
                    interrupt(pointer as Handle);
                }
            });
            let session = Self {
                api,
                db,
                connection,
                stop,
                watchdog: Some(watchdog),
                expired,
            };
            session.query("SET TimeZone='UTC'")?;
            Ok(session)
        }
    }
    pub fn query(&self, sql: &str) -> Result<Vec<Vec<Option<String>>>> {
        if self.expired.load(Ordering::Acquire) {
            return Err(Error::new(
                "SOURCE_TIMEOUT",
                "元数据读取超过时间限制，请稍后重试",
            ));
        }
        let query = CString::new(sql).map_err(Error::io)?;
        unsafe {
            let mut result = QueryResult {
                raw: std::mem::zeroed(),
                api: &self.api,
            };
            let status = (self.api.query)(self.connection, query.as_ptr(), &mut result.raw);
            if self.expired.load(Ordering::Acquire) {
                return Err(Error::new(
                    "SOURCE_TIMEOUT",
                    "元数据读取超过时间限制，请稍后重试",
                ));
            }
            if status != 0 {
                let text = if result.raw.error.is_null() {
                    "分析索引查询失败".into()
                } else {
                    CStr::from_ptr(result.raw.error)
                        .to_string_lossy()
                        .into_owned()
                };
                let code = if text.contains("Out of Memory") {
                    "SOURCE_RESOURCE_LIMIT"
                } else {
                    "SOURCE_FORMAT_ERROR"
                };
                return Err(Error::new(code, text));
            }
            let rows = (self.api.row_count)(&mut result.raw);
            let columns = (self.api.column_count)(&mut result.raw);
            if rows > 101 || columns > 64 {
                return Err(Error::new("METADATA_LIMIT", "元数据结果超出读取边界"));
            }
            let mut output = Vec::new();
            let mut size = 0usize;
            for row in 0..rows {
                let mut values = Vec::new();
                for column in 0..columns {
                    if (self.api.value_is_null)(&mut result.raw, column, row) {
                        values.push(None);
                        continue;
                    }
                    let ptr = (self.api.value_varchar)(&mut result.raw, column, row);
                    if ptr.is_null() {
                        return Err(Error::new("SOURCE_RESOURCE_LIMIT", "元数据值分配失败"));
                    }
                    let bytes = CStr::from_ptr(ptr).to_bytes();
                    size += bytes.len();
                    let value = if size <= 2 * 1024 * 1024 {
                        Some(String::from_utf8_lossy(bytes).into_owned())
                    } else {
                        None
                    };
                    (self.api.free)(ptr.cast());
                    if value.is_none() {
                        return Err(Error::new("METADATA_LIMIT", "元数据响应超过 2 MiB"));
                    }
                    values.push(value);
                }
                output.push(values);
            }
            Ok(output)
        }
    }
    #[cfg(test)]
    pub fn fixture(dll: &Path, path: &Path) -> Result<Self> {
        Self::configured(dll, path, false, Duration::from_secs(8))
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.watchdog.take() {
            let _ = thread.join();
        }
        unsafe {
            (self.api.disconnect)(&mut self.connection);
            (self.api.close)(&mut self.db);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_budget_interrupts_native_execution_and_handles_are_released() {
        let dll = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/duckdb/duckdb.dll");
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("budget.duckdb");
        drop(Session::fixture(&dll, &path).unwrap());
        let db = Session::configured(&dll, &path, true, Duration::from_millis(100)).unwrap();
        assert_eq!(
            db.query("SELECT SUM(i*j) FROM range(1000000000) a(i),range(1000000000) b(j)")
                .unwrap_err()
                .code,
            "SOURCE_TIMEOUT"
        );
        drop(db);
        std::fs::remove_file(&path).unwrap();
    }
}
