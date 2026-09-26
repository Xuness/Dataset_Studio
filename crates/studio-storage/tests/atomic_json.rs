#[cfg(windows)]
#[test]
fn checkpoint_replacement_retries_a_short_windows_reader() {
    use std::{fs, os::windows::fs::OpenOptionsExt, path::Path, time::Duration};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
    fs::create_dir_all(&root).unwrap();
    let temp = tempfile::Builder::new()
        .prefix("atomic-checkpoint-")
        .tempdir_in(root)
        .unwrap();
    let path = temp.path().join("checkpoint.json");
    studio_storage::atomic_json(&path, &serde_json::json!({"version":1})).unwrap();
    // This handle permits reads and writes, but temporarily denies replacement.
    let reader = fs::OpenOptions::new()
        .read(true)
        .share_mode(3)
        .open(&path)
        .unwrap();
    let target = path.clone();
    let update = std::thread::spawn(move || {
        studio_storage::atomic_json(&target, &serde_json::json!({"version":2}))
    });
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(&path).unwrap()).unwrap()["version"],
        1
    );
    drop(reader);
    update.join().unwrap().unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(&path).unwrap()).unwrap()["version"],
        2
    );
}
