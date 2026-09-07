use serde_json::json;
use studio_application::{DraftRepository, ProjectRepository};
use studio_domain::SaveDraft;
use studio_storage::SqliteStore;

#[test]
fn drafts_are_versioned_scoped_and_persistent_without_changing_selection() {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteStore::new(root.path().into()).unwrap();
    let p = store.create("草稿甲", None).unwrap();
    let other = store.create("草稿乙", None).unwrap();
    let request = SaveDraft {
        schema_version: 1,
        expected_revision: 0,
        value: json!({"scope":"selection","text":"尚未提交"}),
    };
    let saved = store
        .save_draft(&p.id, "core.query", "default", request.clone())
        .unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(
        store
            .save_draft(&p.id, "core.query", "default", request)
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    assert!(
        store
            .draft(&other.id, "core.query", "default")
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .draft(&p.id, "core.query", "second")
            .unwrap()
            .is_none()
    );
    assert_eq!(store.selection(&p.id).unwrap().revision, 0);
    store
        .save_preference(
            "layout",
            SaveDraft {
                schema_version: 1,
                expected_revision: 0,
                value: json!({"properties":false}),
            },
        )
        .unwrap();
    store.close(&p.id).unwrap();
    drop(store);
    let store = SqliteStore::new(root.path().into()).unwrap();
    assert!(store.owned_projects().unwrap().is_empty());
    assert_eq!(
        store.preference("layout").unwrap().unwrap().value,
        json!({"properties":false})
    );
    store.open_recent(&p.id).unwrap();
    assert_eq!(
        store
            .draft(&p.id, "core.query", "default")
            .unwrap()
            .unwrap()
            .value,
        saved.value
    );
    assert!(store.jobs(&p.id).unwrap().is_empty());
}

#[test]
fn future_draft_and_legacy_payloads_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteStore::new(root.path().into()).unwrap();
    let p = store.create("旧草稿", None).unwrap();
    let db = rusqlite::Connection::open(p.directory.join("project.sqlite")).unwrap();
    db.execute(
        "INSERT INTO drafts VALUES ('old.tool','{\"legacy\":true}')",
        [],
    )
    .unwrap();
    let old = store.draft(&p.id, "old.tool", "default").unwrap().unwrap();
    assert_eq!(old.schema_version, 0);
    assert_eq!(old.value, json!({"legacy":true}));
    store
        .save_draft(
            &p.id,
            "future.tool",
            "default",
            SaveDraft {
                schema_version: 9,
                expected_revision: 0,
                value: json!({"future":true}),
            },
        )
        .unwrap();
    assert_eq!(
        store
            .save_draft(
                &p.id,
                "future.tool",
                "default",
                SaveDraft {
                    schema_version: 1,
                    expected_revision: 1,
                    value: json!({})
                }
            )
            .unwrap_err()
            .code,
        "DRAFT_VERSION_UNSUPPORTED"
    );
    assert_eq!(
        store
            .draft(&p.id, "future.tool", "default")
            .unwrap()
            .unwrap()
            .value,
        json!({"future":true})
    );
    assert!(
        store
            .save_draft(
                &p.id,
                "large",
                "default",
                SaveDraft {
                    schema_version: 1,
                    expected_revision: 0,
                    value: json!("x".repeat(65537))
                }
            )
            .is_err()
    );
}
