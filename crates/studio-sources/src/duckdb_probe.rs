//! A deliberately narrow, read-only compatibility probe. Catalog browsing does not require this DLL.
use std::{
    ffi::{CStr, CString, c_char, c_void},
    path::Path,
};
use studio_domain::*;
#[repr(C)]
struct QueryResult {
    columns: u64,
    rows: u64,
    changed: u64,
    deprecated_columns: *mut c_void,
    error: *mut c_char,
    internal: *mut c_void,
}
pub fn probe(dll: &Path, database: &Path) -> Result<String> {
    // DuckDB's stable C API owns all handles. Every successful allocation is closed before unloading.
    unsafe {
        let lib = libloading::Library::new(dll).map_err(Error::io)?;
        let create: libloading::Symbol<unsafe extern "C" fn(*mut *mut c_void) -> u32> =
            lib.get(b"duckdb_create_config").map_err(Error::io)?;
        let set: libloading::Symbol<
            unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> u32,
        > = lib.get(b"duckdb_set_config").map_err(Error::io)?;
        let destroy: libloading::Symbol<unsafe extern "C" fn(*mut *mut c_void)> =
            lib.get(b"duckdb_destroy_config").map_err(Error::io)?;
        let open: libloading::Symbol<
            unsafe extern "C" fn(
                *const c_char,
                *mut *mut c_void,
                *mut c_void,
                *mut *mut c_char,
            ) -> u32,
        > = lib.get(b"duckdb_open_ext").map_err(Error::io)?;
        let close: libloading::Symbol<unsafe extern "C" fn(*mut *mut c_void)> =
            lib.get(b"duckdb_close").map_err(Error::io)?;
        let connect: libloading::Symbol<
            unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> u32,
        > = lib.get(b"duckdb_connect").map_err(Error::io)?;
        let disconnect: libloading::Symbol<unsafe extern "C" fn(*mut *mut c_void)> =
            lib.get(b"duckdb_disconnect").map_err(Error::io)?;
        let query: libloading::Symbol<
            unsafe extern "C" fn(*mut c_void, *const c_char, *mut QueryResult) -> u32,
        > = lib.get(b"duckdb_query").map_err(Error::io)?;
        let result_destroy: libloading::Symbol<unsafe extern "C" fn(*mut QueryResult)> =
            lib.get(b"duckdb_destroy_result").map_err(Error::io)?;
        let value: libloading::Symbol<
            unsafe extern "C" fn(*mut QueryResult, u64, u64) -> *mut c_char,
        > = lib.get(b"duckdb_value_varchar").map_err(Error::io)?;
        let free: libloading::Symbol<unsafe extern "C" fn(*mut c_void)> =
            lib.get(b"duckdb_free").map_err(Error::io)?;
        let path = CString::new(database.to_string_lossy().as_bytes()).map_err(Error::io)?;
        let mut config = std::ptr::null_mut();
        if create(&mut config) != 0 {
            return Err(Error::new("DUCKDB_ERROR", "无法创建 DuckDB 配置"));
        }
        for (k, v) in [
            (c"access_mode", c"READ_ONLY"),
            (c"threads", c"1"),
            (c"memory_limit", c"256MB"),
        ] {
            if set(config, k.as_ptr(), v.as_ptr()) != 0 {
                destroy(&mut config);
                return Err(Error::new("DUCKDB_ERROR", "DuckDB 配置不兼容"));
            }
        }
        let mut db = std::ptr::null_mut();
        let mut message = std::ptr::null_mut();
        let status = open(path.as_ptr(), &mut db, config, &mut message);
        destroy(&mut config);
        if status != 0 {
            let text = if message.is_null() {
                "无法打开分析索引".into()
            } else {
                let s = CStr::from_ptr(message).to_string_lossy().into_owned();
                free(message.cast());
                s
            };
            return Err(Error::new("SOURCE_BUSY", text));
        }
        let mut connection = std::ptr::null_mut();
        if connect(db, &mut connection) != 0 {
            close(&mut db);
            return Err(Error::new("DUCKDB_ERROR", "无法连接分析索引"));
        }
        let mut result: QueryResult = std::mem::zeroed();
        let status = query(
            connection,
            c"SELECT version() || ' / applied=' || CAST(MAX(seq) AS VARCHAR) FROM applied".as_ptr(),
            &mut result,
        );
        let response = if status == 0 {
            let ptr = value(&mut result, 0, 0);
            if ptr.is_null() {
                Err(Error::new("SOURCE_FORMAT_ERROR", "分析索引缺少水位"))
            } else {
                let text = CStr::from_ptr(ptr).to_string_lossy().into_owned();
                free(ptr.cast());
                Ok(text)
            }
        } else {
            Err(Error::new("SOURCE_FORMAT_ERROR", "分析索引查询失败"))
        };
        result_destroy(&mut result);
        disconnect(&mut connection);
        close(&mut db);
        response
    }
}
