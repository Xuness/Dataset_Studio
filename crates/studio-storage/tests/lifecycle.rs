use fs2::FileExt;
use rusqlite::Connection;
use std::{fs, fs::OpenOptions};
use studio_application::ProjectRepository;
use studio_domain::{AssetKey, ProjectState, Source, new_id};
use studio_storage::SqliteStore;

#[test]
fn listing_never_opens_or_migrates_and_close_waits_for_inflight_requests() {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("runtime");
    let store = SqliteStore::new(runtime.clone()).unwrap();
    let project = store.create("可释放", None).unwrap();
    let pin = store.request_lease(&project.id).unwrap();
    assert_eq!(
        store.close(&project.id).unwrap().state,
        ProjectState::Draining
    );
    assert_eq!(
        store.request_lease(&project.id).err().unwrap().code,
        "PROJECT_CLOSED"
    );
    let other = SqliteStore::new(root.path().join("other")).unwrap();
    assert_eq!(
        other.open(project.directory.clone()).unwrap_err().code,
        "PROJECT_BUSY"
    );
    drop(pin);
    store.reap_closed().unwrap();
    assert_eq!(store.list().unwrap()[0].state, ProjectState::Closed);
    other.open(project.directory.clone()).unwrap();
    assert_eq!(store.list().unwrap().len(), 1);
    drop(other);
    drop(store);
    // Downgrade only the fixture's user_version after dropping all newer tables.
    let db = Connection::open(project.directory.join("project.sqlite")).unwrap();
    db.execute_batch("DROP TABLE evaluation_source_refs; DROP TABLE evaluation_stage_refs; DROP TABLE collection_scopes; DROP TABLE job_scopes; DROP TABLE result_references; DROP TABLE selection_exclusions; DROP TABLE selection_base; DROP VIEW result_members; DROP TABLE query_member_data; DROP TABLE query_results; DROP TABLE query_families; DROP TABLE query_definitions; PRAGMA user_version=2;").unwrap();
    drop(db);
    let before = fs::read(project.directory.join("project.sqlite")).unwrap();
    let store = SqliteStore::new(runtime).unwrap();
    assert_eq!(store.list().unwrap().len(), 1);
    assert!(store.owned_projects().unwrap().is_empty());
    assert_eq!(
        before,
        fs::read(project.directory.join("project.sqlite")).unwrap()
    );
    assert!(!project.directory.join(".backups").exists());
}

#[test]
fn recovery_isolates_missing_corrupt_future_and_busy_neighbors() {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("runtime");
    let store = SqliteStore::new(runtime.clone()).unwrap();
    let projects = (0..5)
        .map(|i| store.create(&format!("项目 {i}"), None).unwrap())
        .collect::<Vec<_>>();
    let good = &projects[0];
    let sid = new_id();
    store
        .attach(
            &good.id,
            Source {
                id: sid.clone(),
                name: "来源".into(),
                kind: "demo".into(),
                index_root: None,
                media_root: None,
            },
        )
        .unwrap();
    let selection = store
        .change_selection(
            &good.id,
            0,
            &[AssetKey {
                source_id: sid,
                asset_id: "one".into(),
            }],
            &[],
            false,
        )
        .unwrap();
    let job = store
        .submit_job(&good.id, &new_id(), selection.revision, 0)
        .unwrap();
    store
        .update_job(&good.id, &job.id, "running", 0, None, None)
        .unwrap();
    assert_eq!(
        store.close(&good.id).unwrap().state,
        ProjectState::Background
    );
    drop(store);
    let registry = Connection::open(runtime.join("registry.sqlite")).unwrap();
    registry
        .execute("UPDATE projects SET background_pending=1", [])
        .unwrap();
    drop(registry);
    fs::rename(
        projects[1].directory.join("project.sqlite"),
        projects[1].directory.join("offline.sqlite"),
    )
    .unwrap();
    fs::write(
        projects[2].directory.join("project.sqlite"),
        b"broken database",
    )
    .unwrap();
    let future = Connection::open(projects[3].directory.join("project.sqlite")).unwrap();
    future.execute_batch("PRAGMA user_version=999;").unwrap();
    drop(future);
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(projects[4].directory.join(".project.lock"))
        .unwrap();
    lock.try_lock_exclusive().unwrap();
    // Give the busy fixture a queued task so recovery actually needs its lease.
    let db = Connection::open(projects[4].directory.join("project.sqlite")).unwrap();
    db.execute(
        "INSERT INTO jobs VALUES (?1,'core.manifest','queued',1,0,0,'1',NULL,NULL,?1,'x',0)",
        [new_id()],
    )
    .unwrap();
    drop(db);
    let store = SqliteStore::new(runtime).unwrap();
    assert_eq!(store.list().unwrap().len(), 5);
    assert_eq!(store.recover_jobs().unwrap().len(), 4);
    assert_eq!(store.scheduled_jobs(&good.id, false).unwrap()[0].id, job.id);
    assert_eq!(store.list().unwrap().len(), 5);
    assert_eq!(store.owned_projects().unwrap(), vec![good.id.clone()]);
    store.open_recent(&good.id).unwrap();
    assert!(store.view_is_open(&good.id));
    store.close(&good.id).unwrap();
    store
        .update_job(&good.id, &job.id, "succeeded", 1, None, None)
        .unwrap();
    store.reap_closed().unwrap();
    assert!(store.owned_projects().unwrap().is_empty());
}
