use super::*;
use studio_application::{ProjectRepository, SourceAdapter};
use studio_resources::ReadCoordinator;
use studio_sources::SourceRouter;

async fn until(mut condition: impl FnMut() -> bool) {
    let start = Instant::now();
    while !condition() {
        assert!(start.elapsed() < Duration::from_secs(10));
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
fn setup() -> (
    tempfile::TempDir,
    Arc<SqliteStore>,
    String,
    Arc<PreviewService>,
    ReadCoordinator,
) {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::new(root.path().join("app")).unwrap());
    let project = store.create("读取测试", None).unwrap();
    store
        .attach(
            &project.id,
            Source {
                id: studio_sources::DEMO_ID.into(),
                name: "fixture".into(),
                kind: "demo".into(),
                index_root: None,
                media_root: None,
            },
        )
        .unwrap();
    let coordinator = ReadCoordinator::default();
    let cache = PreviewCache::open(&root.path().join("cache")).unwrap();
    let resources: Arc<dyn ReadResources> = Arc::new(coordinator.clone());
    let sources = crate::sources::SourceService::new(
        studio_sources::registry(root.path().join("query-temp"), None, None).unwrap(),
        resources.clone(),
    );
    let service = PreviewService::new(resources, cache, sources);
    (root, store, project.id, service, coordinator)
}
fn submit(
    service: Arc<PreviewService>,
    store: Arc<SqliteStore>,
    pid: String,
    id: String,
    asset: &str,
) -> tokio::task::JoinHandle<Result<Arc<Preview>>> {
    let asset = asset.to_owned();
    tokio::spawn(async move {
        let ticket = service.ticket(&pid, &id)?;
        service
            .get(
                store,
                &ticket,
                pid,
                AssetKey {
                    source_id: studio_sources::DEMO_ID.into(),
                    asset_id: asset,
                },
                PreviewOptions {
                    edge: 360,
                    priority: ReadPriority::Interactive,
                    max_source_bytes: 64 << 20,
                },
            )
            .await
    })
}
#[tokio::test]
async fn shared_subscriber_cancel_preserves_other_and_last_cancel_prevents_source_read() {
    let (_root, store, pid, service, coordinator) = setup();
    let held = coordinator
        .acquire(
            ReadRequest {
                class: ReadClass::Media,
                priority: ReadPriority::Interactive,
                bytes: 64 << 20,
            },
            &AtomicBool::new(false),
        )
        .unwrap();
    let runner = tokio::spawn(service.clone().run());
    let id1 = new_id();
    let id2 = new_id();
    let a = submit(
        service.clone(),
        store.clone(),
        pid.clone(),
        id1.clone(),
        "sample-0001",
    );
    let b = submit(
        service.clone(),
        store.clone(),
        pid.clone(),
        id2.clone(),
        "sample-0001",
    );
    until(|| service.metrics().shared == 1).await;
    service.cancel(&pid, &id1).unwrap();
    assert_eq!(a.await.unwrap().err().unwrap().code, "CANCELLED");
    assert_eq!(service.metrics().cancelled_last, 0);
    drop(held);
    let response = b.await.unwrap().unwrap();
    assert!(!response.media.bytes.is_empty());
    drop(response);
    assert_eq!(service.metrics().generated, 1);
    let held = coordinator
        .acquire(
            ReadRequest {
                class: ReadClass::Media,
                priority: ReadPriority::Interactive,
                bytes: 64 << 20,
            },
            &AtomicBool::new(false),
        )
        .unwrap();
    let id3 = new_id();
    let c = submit(
        service.clone(),
        store.clone(),
        pid.clone(),
        id3.clone(),
        "sample-0002",
    );
    until(|| {
        coordinator
            .metrics()
            .iter()
            .find(|m| m.budget.class == ReadClass::Media)
            .unwrap()
            .queued
            == 1
    })
    .await;
    service.cancel(&pid, &id3).unwrap();
    assert_eq!(c.await.unwrap().err().unwrap().code, "CANCELLED");
    until(|| {
        coordinator
            .metrics()
            .iter()
            .find(|m| m.budget.class == ReadClass::Media)
            .unwrap()
            .cancelled_waiting
            == 1
    })
    .await;
    drop(held);
    assert_eq!(service.metrics().generated, 1);
    let early = new_id();
    service.cancel(&pid, &early).unwrap();
    assert_eq!(
        service.ticket(&pid, &early).err().unwrap().code,
        "CANCELLED"
    );
    service.shutdown();
    runner.await.unwrap();
}
#[tokio::test]
async fn cancelled_pack_request_performs_zero_payload_reads() {
    use sha2::{Digest, Sha256};
    let (root, store, pid, service, coordinator) = setup();
    let source = store.source(&pid, studio_sources::DEMO_ID).unwrap();
    let bytes = SourceRouter.read(&source, "sample-0001").unwrap().bytes;
    let asset_id = hex::encode(Sha256::digest(&bytes));
    let lake = root.path().join("indexed-pack-fixture");
    std::fs::create_dir_all(lake.join("indexes/g")).unwrap();
    let id = new_id();
    std::fs::write(
        lake.join("CURRENT.json"),
        serde_json::json!({"library_id":id,"generation":"g","index_version":1}).to_string(),
    )
    .unwrap();
    std::fs::write(lake.join("library.json"),serde_json::json!({"library_id":id,"format_version":1,"image_format":"uncompressed-pax-tar"}).to_string()).unwrap();
    std::fs::write(lake.join("pack.tar"), &bytes).unwrap();
    let db = rusqlite::Connection::open(lake.join("indexes/g/catalog.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE state(key TEXT PRIMARY KEY,value INTEGER);INSERT INTO state VALUES('seq',1);CREATE TABLE objects(sha256 TEXT PRIMARY KEY,pack_path TEXT,offset INTEGER,length INTEGER,stored_ext TEXT) WITHOUT ROWID;").unwrap();
    db.execute(
        "INSERT INTO objects VALUES(?1,'pack.tar',0,?2,'png')",
        rusqlite::params![asset_id, bytes.len() as i64],
    )
    .unwrap();
    drop(db);
    store
        .attach(
            &pid,
            Source {
                id: id.clone(),
                name: "pack".into(),
                kind: "danbooru".into(),
                index_root: Some(lake.clone()),
                media_root: Some(lake),
            },
        )
        .unwrap();
    let held = coordinator
        .acquire(
            ReadRequest {
                class: ReadClass::Media,
                priority: ReadPriority::Interactive,
                bytes: 64 << 20,
            },
            &AtomicBool::new(false),
        )
        .unwrap();
    let runner = tokio::spawn(service.clone().run());
    let request_id = new_id();
    let ticket = service.ticket(&pid, &request_id).unwrap();
    let worker_service = service.clone();
    let worker_pid = pid.clone();
    let requested = tokio::spawn(async move {
        worker_service
            .get(
                store,
                &ticket,
                worker_pid,
                AssetKey {
                    source_id: id,
                    asset_id,
                },
                PreviewOptions {
                    edge: 360,
                    priority: ReadPriority::Prefetch,
                    max_source_bytes: 64 << 20,
                },
            )
            .await
    });
    until(|| {
        coordinator
            .metrics()
            .iter()
            .find(|m| m.budget.class == ReadClass::Media)
            .unwrap()
            .queued
            == 1
    })
    .await;
    service.cancel(&pid, &request_id).unwrap();
    assert_eq!(requested.await.unwrap().err().unwrap().code, "CANCELLED");
    until(|| service.metrics().cancelled_finished == 1).await;
    let m = service.metrics();
    assert_eq!(m.source_bytes, 0);
    assert_eq!(m.pack_opens, 0);
    assert_eq!(m.generated, 0);
    assert_eq!(m.cancelled_before_read, 1);
    assert!(m.max_cancel_latency_ms < 500);
    println!(
        "cancelled_pack_fixture={}",
        serde_json::to_string(&m).unwrap()
    );
    drop(held);
    service.shutdown();
    runner.await.unwrap();
}
