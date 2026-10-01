use super::*;

fn root() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/test-runs")
        .join(format!("runtime-r3-{}", studio_domain::new_id()));
    fs::create_dir_all(&path).unwrap();
    path
}
#[test]
fn rejected_executable_does_not_install_configuration_or_owner() {
    let root = root();
    let python = root.join("invalid.exe");
    fs::write(&python, b"not an executable").unwrap();
    let backend = Backend::new(root.join("app"));
    let state_root = root.join("state");
    let error = backend
        .configure(LakeUpdateRuntime {
            python,
            state_root: state_root.clone(),
        })
        .unwrap_err();
    assert_eq!(error.code, "UPDATE_UNAVAILABLE", "{error:?}");
    assert!(!backend.configured());
    assert!(!backend.config_path.exists());
    assert!(!state_root.join("studio-owner.json").exists());
    assert_eq!(backend.health().state, "failed");
}
#[test]
fn broken_saved_configuration_keeps_health_and_repair_entry() {
    let root = root();
    fs::write(root.join("lake-update-runtime.json"), b"{broken").unwrap();
    let backend = Backend::new(root);
    assert!(!backend.configured());
    assert_eq!(
        backend.health().error_code.as_deref(),
        Some("UPDATE_CONFIGURATION")
    );
}
#[test]
fn startup_diagnostics_are_bounded_redacted_and_backed_off() {
    let root = root();
    let backend = Backend::new(root.clone());
    for _ in 0..70 {
        backend.failure(&Error::new(
            "UPDATE_PROTOCOL",
            "secret-api-key-must-not-appear",
        ));
    }
    let health = backend.health();
    assert_eq!(health.failures, 70);
    assert!(health.next_retry_ms.unwrap() > millis() + 290_000);
    let bytes = fs::read_to_string(root.join("lake-update-health.json")).unwrap();
    assert!(!bytes.contains("secret-api-key"));
    assert_eq!(
        serde_json::from_str::<Vec<Value>>(&bytes).unwrap().len(),
        32
    );
}
#[test]
fn startup_handshake_rejects_wrong_version_without_exposing_output() {
    for value in [
        json!({"protocol_version":2,"ok":true,"result":{}}),
        json!({"protocol_version":1,"ok":true,"result":{"worker_version":"old","runtime_check":1}}),
        json!({"protocol_version":1,"ok":false,"error":{"code":"UPDATE_PROTOCOL","message":"secret"}}),
    ] {
        let error = process::handshake(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert_eq!(error.code, "UPDATE_PROTOCOL");
        assert!(!error.to_string().contains("secret"));
    }
}
#[test]
fn stopping_controller_never_saves_a_replacement() {
    let root = root();
    let backend = Backend::new(root.clone());
    backend.shutdown();
    assert_eq!(
        backend
            .configure(LakeUpdateRuntime {
                python: root.join("python"),
                state_root: root.join("state")
            })
            .unwrap_err()
            .code,
        "UPDATE_UNAVAILABLE"
    );
    assert!(!backend.config_path.exists());
    assert_eq!(backend.health().state, "stopping");
}
#[test]
fn immediate_exit_is_not_a_ready_worker() {
    #[cfg(windows)]
    let mut command = Command::new("cmd.exe");
    #[cfg(not(windows))]
    let mut command = Command::new("sh");
    #[cfg(windows)]
    command.args(["/c", "exit", "1"]);
    #[cfg(not(windows))]
    command.args(["-c", "exit 1"]);
    let result = process::prepared(command, &AtomicBool::new(false));
    assert!(result.is_err());
}
