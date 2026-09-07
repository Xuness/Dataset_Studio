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
#[derive(Clone, Copy)]
struct RawResult {
    columns: u64,
    rows: u64,
    changed: u64,
    deprecated_columns: Handle,
    error: *mut c_char,
    internal: Handle,
}
#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct RawString {
    bytes: [u8; 16],
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
    prepare:unsafe extern "C" fn(Handle,*const c_char,*mut Handle)->u32,
    prepare_error:unsafe extern "C" fn(Handle)->*const c_char,
    destroy_prepare:unsafe extern "C" fn(*mut Handle),
    execute_prepared_streaming:unsafe extern "C" fn(Handle,*mut RawResult)->u32,
    result_error:unsafe extern "C" fn(*mut RawResult)->*const c_char,
    fetch_chunk:unsafe extern "C" fn(RawResult)->Handle,
    destroy_data_chunk:unsafe extern "C" fn(*mut Handle),
    data_chunk_get_size:unsafe extern "C" fn(Handle)->u64,
    data_chunk_get_column_count:unsafe extern "C" fn(Handle)->u64,
    data_chunk_get_vector:unsafe extern "C" fn(Handle,u64)->Handle,
    vector_get_data:unsafe extern "C" fn(Handle)->Handle,
    vector_get_validity:unsafe extern "C" fn(Handle)->*mut u64,
    validity_row_is_valid:unsafe extern "C" fn(*mut u64,u64)->bool,
    string_t_length:unsafe extern "C" fn(RawString)->u32,
    string_t_data:unsafe extern "C" fn(*mut RawString)->*const c_char,
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
    cancelled: Option<Arc<AtomicBool>>,
}
/// Retain the DLL, while releasing all database handles at the end of a request.
pub(crate) struct Runtime {
    path: PathBuf,
    api: Mutex<Option<Arc<Api>>>,
}
impl Default for Runtime {
    fn default() -> Self {
        let dll = std::env::var_os("STUDIO_DUCKDB_DLL")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let local = std::env::current_exe()
                    .unwrap_or_default()
                    .with_file_name("duckdb.dll");
                if local.is_file() || !cfg!(debug_assertions) {
                    local
                } else {
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/duckdb/duckdb.dll")
                }
            });
        Self::new(dll)
    }
}
impl Runtime {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            api: Mutex::new(None),
        }
    }
    pub fn open(&self, path: &Path) -> Result<Session> {
        self.open_with(path, Duration::from_secs(8), None)
    }
    pub fn open_query(&self, path: &Path, cancelled: Arc<AtomicBool>) -> Result<Session> {
        self.open_with(path, Duration::from_secs(600), Some(cancelled))
    }
    fn open_with(
        &self,
        path: &Path,
        budget: Duration,
        cancelled: Option<Arc<AtomicBool>>,
    ) -> Result<Session> {
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
            budget.saturating_sub(started.elapsed()),
            cancelled,
        )
    }
}
impl Session {
    pub fn open(dll: &Path, path: &Path) -> Result<Self> {
        Self::configured(dll, path, true, Duration::from_secs(8))
    }
    fn configured(dll: &Path, path: &Path, readonly: bool, budget: Duration) -> Result<Self> {
        Self::with_api(
            Arc::new(unsafe { Api::load(dll) }?),
            path,
            readonly,
            budget,
            None,
        )
    }
    fn with_api(
        api: Arc<Api>,
        path: &Path,
        readonly: bool,
        budget: Duration,
        cancelled: Option<Arc<AtomicBool>>,
    ) -> Result<Self> {
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
            let cancellation = cancelled.clone();
            let watchdog = thread::spawn(move || {
                let deadline = Instant::now() + remaining;
                loop {
                    if !matches!(
                        rx.recv_timeout(Duration::from_millis(40)),
                        Err(mpsc::RecvTimeoutError::Timeout)
                    ) {
                        break;
                    }
                    let cancel = cancellation
                        .as_ref()
                        .is_some_and(|c| c.load(Ordering::Acquire));
                    if cancel || Instant::now() >= deadline {
                        timed_out.store(!cancel, Ordering::Release);
                        // Drop joins before disconnecting or unloading the function pointer.
                        interrupt(pointer as Handle);
                        break;
                    }
                }
            });
            let session = Self {
                api,
                db,
                connection,
                stop,
                watchdog: Some(watchdog),
                expired,
                cancelled,
            };
            session.query("SET TimeZone='UTC'")?;
            Ok(session)
        }
    }
    pub fn query(&self, sql: &str) -> Result<Vec<Vec<Option<String>>>> {
        self.check_cancelled()?;
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
    fn check_cancelled(&self) -> Result<()> {
        if self
            .cancelled
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Acquire))
        {
            return Err(Error::new("CANCELLED", "查询构建已取消"));
        }
        if self.expired.load(Ordering::Acquire) {
            return Err(Error::new("SOURCE_TIMEOUT", "查询超过读取预算"));
        }
        Ok(())
    }
    /// A single VARCHAR identity column, copied at most 512 rows at a time.
    /// The streaming entrypoint is pinned to 1.5.4; no materialized row API is mixed in.
    pub fn stream_ids(
        &self,
        sql: &str,
        sink: &mut dyn FnMut(&[String]) -> Result<()>,
    ) -> Result<()> {
        self.check_cancelled()?;
        struct Statement<'a> {
            value: Handle,
            api: &'a Api,
        }
        impl Drop for Statement<'_> {
            fn drop(&mut self) {
                unsafe {
                    (self.api.destroy_prepare)(&mut self.value);
                }
            }
        }
        struct Chunk<'a> {
            value: Handle,
            api: &'a Api,
        }
        impl Drop for Chunk<'_> {
            fn drop(&mut self) {
                unsafe {
                    (self.api.destroy_data_chunk)(&mut self.value);
                }
            }
        }
        let sql = CString::new(sql).map_err(Error::io)?;
        unsafe {
            let mut statement = Statement {
                value: std::ptr::null_mut(),
                api: &self.api,
            };
            if (self.api.prepare)(self.connection, sql.as_ptr(), &mut statement.value) != 0 {
                return Err(native_error((self.api.prepare_error)(statement.value)));
            }
            let mut result = QueryResult {
                raw: std::mem::zeroed(),
                api: &self.api,
            };
            let status = (self.api.execute_prepared_streaming)(statement.value, &mut result.raw);
            self.check_cancelled()?;
            if status != 0 {
                return Err(native_error((self.api.result_error)(&mut result.raw)));
            }
            loop {
                self.check_cancelled()?;
                let chunk = Chunk {
                    value: (self.api.fetch_chunk)(result.raw),
                    api: &self.api,
                };
                self.check_cancelled()?;
                if chunk.value.is_null() {
                    let error = (self.api.result_error)(&mut result.raw);
                    if !error.is_null() {
                        return Err(native_error(error));
                    }
                    break;
                }
                if (self.api.data_chunk_get_column_count)(chunk.value) != 1 {
                    return Err(Error::new("QUERY_PROTOCOL_ERROR", "查询必须仅返回存储身份"));
                }
                let rows = (self.api.data_chunk_get_size)(chunk.value);
                let vector = (self.api.data_chunk_get_vector)(chunk.value, 0);
                let validity = (self.api.vector_get_validity)(vector);
                let data = (self.api.vector_get_data)(vector).cast::<RawString>();
                let mut batch = Vec::with_capacity(512);
                for row in 0..rows {
                    if !validity.is_null() && !(self.api.validity_row_is_valid)(validity, row) {
                        return Err(Error::new("SOURCE_FORMAT_ERROR", "查询结果包含空身份"));
                    }
                    let mut value = *data.add(row as usize);
                    let length = (self.api.string_t_length)(value) as usize;
                    if length > 128 {
                        return Err(Error::new("SOURCE_FORMAT_ERROR", "存储身份过长"));
                    }
                    let bytes = std::slice::from_raw_parts(
                        (self.api.string_t_data)(&mut value).cast::<u8>(),
                        length,
                    );
                    batch.push(std::str::from_utf8(bytes).map_err(Error::io)?.to_owned());
                    if batch.len() == 512 {
                        sink(&batch)?;
                        batch.clear();
                        self.check_cancelled()?;
                    }
                }
                if !batch.is_empty() {
                    sink(&batch)?;
                }
            }
        }
        Ok(())
    }
    #[cfg(test)]
    pub fn fixture(dll: &Path, path: &Path) -> Result<Self> {
        Self::configured(dll, path, false, Duration::from_secs(8))
    }
}
unsafe fn native_error(pointer: *const c_char) -> Error {
    let message = if pointer.is_null() {
        "分析索引查询失败".into()
    } else {
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned()
    };
    let code = if message.to_lowercase().contains("out of memory") {
        "SOURCE_RESOURCE_LIMIT"
    } else {
        "SOURCE_FORMAT_ERROR"
    };
    Error::new(code, message)
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
    fn streaming_batches_cover_many_chunks_and_cancel_releases_native_handles() {
        let dll = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/duckdb/duckdb.dll");
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("stream.duckdb");
        drop(Session::fixture(&dll, &path).unwrap());
        let runtime = Runtime::new(dll);
        let cancel = Arc::new(AtomicBool::new(false));
        let db = runtime.open_query(&path, cancel.clone()).unwrap();
        let mut count = 0;
        db.stream_ids(
            "SELECT CAST(i AS VARCHAR) FROM range(25000) t(i)",
            &mut |rows| {
                assert!(rows.len() <= 512);
                for row in rows {
                    assert_eq!(row, &count.to_string());
                    count += 1;
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(count, 25000);
        let error = db
            .stream_ids(
                "SELECT CAST(i AS VARCHAR) FROM range(1000000) t(i)",
                &mut |_| {
                    cancel.store(true, Ordering::Release);
                    Ok(())
                },
            )
            .unwrap_err();
        assert_eq!(error.code, "CANCELLED");
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
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
