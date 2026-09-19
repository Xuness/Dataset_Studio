//! Fault ports are inert in ordinary builds. Integration binaries explicitly
//! opt into test-faults and an isolated test directory; there is no HTTP hook.
pub fn check(point: &str, key: &str) -> studio_domain::Result<()> {
    #[cfg(feature = "test-faults")]
    {
        use studio_domain::Error;
        let Some(root) = std::env::var_os("STUDIO_TEST_FAULT_DIR") else {
            return Ok(());
        };
        if !point
            .bytes()
            .chain(key.bytes())
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(Error::invalid("无效的测试故障名称"));
        }
        let path = std::path::PathBuf::from(root).join(format!("{point}-{key}"));
        let mode = match std::fs::read_to_string(&path) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(Error::io(e)),
        };
        std::fs::write(path.with_extension("hit"), mode.as_bytes()).map_err(Error::io)?;
        match mode.trim() {
            "hold" => {
                while path.exists() {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
            // Exit without Rust unwinding or application shutdown, avoiding a Windows crash dialog.
            "crash" => std::process::exit(97),
            "exit" => panic!("injected writer exit"),
            "busy_once" => {
                std::fs::remove_file(path).map_err(Error::io)?;
                return Err(Error::new("EVALUATION_BUSY", "injected SQLite BUSY"));
            }
            "busy" => return Err(Error::new("EVALUATION_BUSY", "injected SQLite BUSY")),
            "lost_ack_once" => {
                std::fs::remove_file(path).map_err(Error::io)?;
                return Err(Error::new(
                    "EVALUATION_STORAGE_IO",
                    "injected lost commit acknowledgement",
                ));
            }
            "full" => {
                return Err(Error::new(
                    "EVALUATION_STORAGE_FULL",
                    "injected SQLite FULL",
                ));
            }
            "io" => return Err(Error::new("EVALUATION_STORAGE_IO", "injected SQLite IOERR")),
            "corrupt" => return Err(Error::new("EVALUATION_CORRUPT", "injected SQLite CORRUPT")),
            _ => return Err(Error::invalid("未知测试故障模式")),
        }
    }
    #[cfg(not(feature = "test-faults"))]
    let _ = (point, key);
    Ok(())
}
