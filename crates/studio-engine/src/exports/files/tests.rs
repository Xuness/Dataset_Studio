use super::*;
use serde_json::json;

fn directory() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
    fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("export-files-")
        .tempdir_in(root)
        .unwrap()
}

#[test]
fn sidecars_select_matching_pairs_without_overwriting_unrelated_content() {
    let temp = directory();
    let folder = temp.path();
    let bytes = b"original image";
    let id = hex::encode(Sha256::digest(bytes));
    let tags = Sidecar {
        extension: "txt",
        bytes: Some(b"long hair, blue".to_vec()),
    };
    fs::write(folder.join("000001_image.txt"), b"unrelated text").unwrap();
    let target = locate(
        folder,
        "000001_image.png",
        bytes.len() as u64,
        &id,
        Some(&tags),
    )
    .unwrap();
    assert_eq!(target.name, "000001_image_2.png");
    assert!(!target.existing);
    let staged = StagedSidecar::prepare(folder, &target.name, &tags)
        .unwrap()
        .unwrap();
    assert!(!folder.join("000001_image_2.txt").exists());
    write_new_file(&folder.join(&target.name), bytes).unwrap();
    staged.publish(folder).unwrap();
    let resumed = locate(
        folder,
        "000001_image.png",
        bytes.len() as u64,
        &id,
        Some(&tags),
    )
    .unwrap();
    assert!(resumed.existing);
    assert_eq!(resumed.name, target.name);

    let changed = Sidecar {
        extension: "txt",
        bytes: Some(b"long_hair blue".to_vec()),
    };
    let renamed = locate(
        folder,
        "000001_image.png",
        bytes.len() as u64,
        &id,
        Some(&changed),
    )
    .unwrap();
    assert_eq!(renamed.name, "000001_image_3.png");
    assert_eq!(
        fs::read(folder.join("000001_image_2.txt")).unwrap(),
        b"long hair, blue"
    );
    assert_eq!(
        fs::read(folder.join("000001_image.txt")).unwrap(),
        b"unrelated text"
    );
}

#[test]
fn images_without_tags_do_not_pick_up_old_tag_files() {
    let temp = directory();
    let folder = temp.path();
    let bytes = b"original";
    let id = hex::encode(Sha256::digest(bytes));
    fs::write(folder.join("one.png"), bytes).unwrap();
    fs::write(folder.join("one.txt"), b"stale tags").unwrap();
    let tags = Sidecar {
        extension: "txt",
        bytes: None,
    };
    let target = locate(folder, "one.png", bytes.len() as u64, &id, Some(&tags)).unwrap();
    assert_eq!(target.name, "one_2.png");
    assert!(
        StagedSidecar::prepare(folder, &target.name, &tags)
            .unwrap()
            .is_none()
    );
}

#[test]
fn publication_conflicts_do_not_clobber_files_or_leave_pending_sidecars() {
    let temp = directory();
    let folder = temp.path();
    let tags = Sidecar {
        extension: "json",
        bytes: Some(b"{\"correct\":true}".to_vec()),
    };
    let staged = StagedSidecar::prepare(folder, "image.png", &tags)
        .unwrap()
        .unwrap();
    fs::write(folder.join("image.json"), b"created after planning").unwrap();
    assert_eq!(
        staged.publish(folder).unwrap_err().code,
        "EXPORT_WRITE_FAILED"
    );
    assert_eq!(
        fs::read(folder.join("image.json")).unwrap(),
        b"created after planning"
    );
    assert_eq!(
        write_new_file(&folder.join("image.json"), b"replacement")
            .unwrap_err()
            .code,
        "EXPORT_WRITE_FAILED"
    );
    assert_eq!(fs::read_dir(folder).unwrap().count(), 1);
    // Save-as is explicitly allowed to replace the user's chosen file.
    write_file(&folder.join("image.json"), b"chosen replacement").unwrap();
    assert_eq!(
        fs::read(folder.join("image.json")).unwrap(),
        b"chosen replacement"
    );
}

#[test]
fn unpublished_sidecar_is_removed_on_cancellation_or_image_failure() {
    let temp = directory();
    let tags = Sidecar {
        extension: "txt",
        bytes: Some(b"tag".to_vec()),
    };
    let staged = StagedSidecar::prepare(temp.path(), "image.png", &tags).unwrap();
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    drop(staged);
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}

#[test]
fn errors_are_readable_before_completion_and_cleared_only_on_success() {
    let temp = directory();
    let path = temp.path().join("export-errors.jsonl");
    fs::write(&path, b"old run\n").unwrap();
    let mut log = FailureLog::new(temp.path());
    log.record(&json!({"ordinal": 0, "metadata_error": "cannot write tags"}))
        .unwrap();
    log.flush().unwrap();
    let first: Value = serde_json::from_str(fs::read_to_string(&path).unwrap().trim()).unwrap();
    assert_eq!(first["metadata_error"], "cannot write tags");
    for ordinal in 1..33 {
        log.record(&json!({"ordinal": ordinal, "error": "missing original"}))
            .unwrap();
    }
    log.flush().unwrap();
    // A cancelled run drops its writer without finish; earlier batches survive.
    drop(log);
    assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 33);
    assert_eq!(FailureLog::new(temp.path()).finish().unwrap(), 0);
    assert!(!path.exists());
}
