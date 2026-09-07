use studio_application::ProjectRepository;
use studio_domain::{AssetKey, Source, new_id};
use studio_storage::SqliteStore;

fn source(id: &str) -> Source {
    Source {
        id: id.into(),
        name: "测试资料".into(),
        kind: "demo".into(),
        index_root: None,
        media_root: None,
    }
}
#[test]
fn selection_is_project_scoped_versioned_and_job_inputs_are_frozen() {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteStore::new(root.path().to_owned()).unwrap();
    let a = store.create("人物项目", None).unwrap();
    let b = store.create("构图项目", None).unwrap();
    let s1 = new_id();
    let s2 = new_id();
    store.attach(&a.id, source(&s1)).unwrap();
    store.attach(&a.id, source(&s2)).unwrap();
    store.attach(&b.id, source(&s1)).unwrap();
    let x = AssetKey {
        source_id: s1.clone(),
        asset_id: "same-local-id".into(),
    };
    let y = AssetKey {
        source_id: s2.clone(),
        asset_id: "same-local-id".into(),
    };
    let selected = store
        .change_selection(&a.id, 0, &[x.clone(), y.clone()], &[], false)
        .unwrap();
    assert_eq!(selected.count, 2);
    assert_eq!(store.selection(&b.id).unwrap().count, 0);
    assert_eq!(
        store
            .change_selection(&a.id, 0, &[], &[], true)
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    let collection = store.save_collection(&a.id, "保留集").unwrap();
    let key = new_id();
    let job = store
        .submit_job(&a.id, &key, selected.revision, 20)
        .unwrap();
    assert_eq!(
        store
            .submit_job(&a.id, &key, selected.revision, 20)
            .unwrap()
            .id,
        job.id
    );
    assert_eq!(
        store
            .submit_job(&a.id, &key, selected.revision, 30)
            .unwrap_err()
            .code,
        "IDEMPOTENCY_CONFLICT"
    );
    store
        .change_selection(&a.id, selected.revision, &[], &[], true)
        .unwrap();
    assert_eq!(store.selection(&a.id).unwrap().count, 0);
    assert_eq!(
        store
            .collection_keys(&a.id, &collection.id, None, 10)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(store.job_inputs(&a.id, &job.id, None).unwrap().len(), 2);
    assert_eq!(store.job(&b.id, &job.id).unwrap_err().code, "NOT_FOUND");
    let events = store.events(&a.id, 0).unwrap();
    assert!(
        events
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence)
    );
}
#[test]
fn project_reopens_in_chinese_path_and_exclusive_owner_is_enforced() {
    let root = tempfile::tempdir().unwrap();
    let projects = root.path().join("中文 项目");
    let store = SqliteStore::new(root.path().join("runtime-a")).unwrap();
    let project = store.create("长期研究", Some(projects)).unwrap();
    let other = SqliteStore::new(root.path().join("runtime-b")).unwrap();
    assert_eq!(
        other.open(project.directory.clone()).unwrap_err().code,
        "PROJECT_BUSY"
    );
    drop(store);
    let reopened = other.open(project.directory.clone()).unwrap();
    assert_eq!(reopened.id, project.id);
    assert_eq!(reopened.name, "长期研究");
}
#[test]
fn selection_pages_do_not_duplicate_or_skip_cross_source_members() {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteStore::new(root.path().to_owned()).unwrap();
    let project = store.create("分页", None).unwrap();
    let source_id = new_id();
    store.attach(&project.id, source(&source_id)).unwrap();
    let keys = (0..12)
        .map(|i| AssetKey {
            source_id: source_id.clone(),
            asset_id: format!("{i:03}"),
        })
        .collect::<Vec<_>>();
    store
        .change_selection(&project.id, 0, &keys, &[], false)
        .unwrap();
    let first = store.selection_keys(&project.id, None, 5).unwrap();
    let second = store.selection_keys(&project.id, first.last(), 5).unwrap();
    let third = store.selection_keys(&project.id, second.last(), 5).unwrap();
    let collected = [first, second, third].concat();
    assert_eq!(collected, keys);
}
