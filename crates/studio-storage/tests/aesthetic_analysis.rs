use studio_application::{ManagementRepository, ProjectRepository};
use studio_domain::{aesthetic_analysis::*, *};
use studio_storage::SqliteStore;

#[test]
fn derived_workset_is_hidden_until_atomic_publish_and_resumes_from_committed_cursor() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
    std::fs::create_dir_all(&root).unwrap();
    let directory = tempfile::Builder::new()
        .prefix("aesthetic-worksets-")
        .tempdir_in(root)
        .unwrap();
    let store = SqliteStore::new(directory.path().join("state")).unwrap();
    let project = store.create("后端工作集", None).unwrap();
    let source = Source {
        id: new_id(),
        name: "fixture".into(),
        kind: "demo".into(),
        index_root: None,
        media_root: None,
    };
    store.attach(&project.id, source.clone()).unwrap();
    let id = new_id();
    let item = AestheticAnalysisJob {
        id: id.clone(),
        created_at: "1".into(),
        state: "running".into(),
        phase: "selecting".into(),
        progress: 0,
        total: 512,
        request: AestheticAnalysisCreate {
            idempotency_key: id,
            name: "派生结果".into(),
            spec: AestheticAnalysisSpec::Derive {
                snapshot_id: new_id(),
                filter: Default::default(),
                review_watermark: None,
            },
        },
        input: AestheticAnalysisInput {
            stage_id: new_id(),
            stage_config_hash: "frozen".into(),
            evidence_watermark: 17,
            observations: 17,
            candidates: 512,
            review_watermark: 3,
        },
        result: None,
        error: None,
    };
    let (cid, _, _, _) = store.begin_evaluation_workset(&project.id, &item).unwrap();
    let keys = |start| {
        (start..start + 256)
            .map(|n| AssetKey {
                source_id: source.id.clone(),
                asset_id: format!("{n:064x}"),
            })
            .collect()
    };
    store
        .append_evaluation_workset(&project.id, &item.id, 0, 256, keys(0))
        .unwrap();
    assert!(store.collections(&project.id).unwrap().is_empty());
    assert_eq!(
        store
            .collection_keys(&project.id, &cid, None, 32)
            .unwrap_err()
            .code,
        "RESULT_NOT_READY"
    );
    assert!(
        store
            .object_details(&project.id, ObjectKind::Workset, &cid)
            .is_err()
    );
    drop(store);
    let store = SqliteStore::new(directory.path().join("state")).unwrap();
    store.open(project.directory.clone()).unwrap();
    assert_eq!(
        store.begin_evaluation_workset(&project.id, &item).unwrap(),
        (cid.clone(), 256, 256, false)
    );
    assert!(
        store
            .publish_evaluation_workset(&project.id, &item)
            .is_err()
    );
    store
        .append_evaluation_workset(&project.id, &item.id, 256, 512, keys(256))
        .unwrap();
    let collection = store
        .publish_evaluation_workset(&project.id, &item)
        .unwrap();
    assert_eq!(collection.count, 512);
    assert_eq!(
        store
            .publish_evaluation_workset(&project.id, &item)
            .unwrap()
            .id,
        cid
    );
    assert_eq!(store.collections(&project.id).unwrap().len(), 1);
    assert_eq!(
        store
            .collection_keys(&project.id, &cid, None, 1000)
            .unwrap()
            .len(),
        512
    );
    let details = store
        .object_details(&project.id, ObjectKind::Workset, &cid)
        .unwrap();
    store
        .remove_object(
            &project.id,
            ObjectKind::Workset,
            &cid,
            details.object.revision,
        )
        .unwrap();
    assert_eq!(
        store
            .begin_evaluation_workset(&project.id, &item)
            .unwrap_err()
            .code,
        "OBJECT_REMOVED"
    );
}
