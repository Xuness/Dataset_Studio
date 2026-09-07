use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use studio_application::ReadResources;
use studio_domain::*;
use studio_resources::*;
fn coordinator(queue: usize) -> ReadCoordinator {
    ReadCoordinator::new(vec![ReadBudget {
        class: ReadClass::Media,
        concurrency: 1,
        queue_limit: queue,
        bytes: 100,
    }])
}
fn request(priority: ReadPriority) -> ReadRequest {
    ReadRequest {
        class: ReadClass::Media,
        priority,
        bytes: 60,
    }
}
fn until(mut condition: impl FnMut() -> bool) {
    let start = Instant::now();
    while !condition() {
        assert!(start.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn interaction_priority_has_bounded_fifo_fairness() {
    let c = coordinator(32);
    let held = c
        .acquire(request(ReadPriority::Interactive), &AtomicBool::new(false))
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let mut workers = Vec::new();
    for (name, priority) in [
        ("prefetch", ReadPriority::Prefetch),
        ("background", ReadPriority::Background),
    ] {
        let (c, tx) = (c.clone(), tx.clone());
        workers.push(thread::spawn(move || {
            let _permit = c
                .acquire(request(priority), &AtomicBool::new(false))
                .unwrap();
            tx.send(name).unwrap();
        }));
    }
    until(|| c.metrics()[0].queued == 2);
    for _ in 0..8 {
        let (c, tx) = (c.clone(), tx.clone());
        workers.push(thread::spawn(move || {
            let _permit = c
                .acquire(request(ReadPriority::Interactive), &AtomicBool::new(false))
                .unwrap();
            tx.send("interactive").unwrap();
        }));
    }
    until(|| c.metrics()[0].queued == 10);
    drop(held);
    let order = (0..10)
        .map(|_| rx.recv_timeout(Duration::from_secs(5)).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(order[0], "interactive");
    assert!(order.iter().position(|v| *v == "background").unwrap() <= 8);
    assert!(order.iter().position(|v| *v == "prefetch").unwrap() <= 8);
    for worker in workers {
        worker.join().unwrap();
    }
    let m = &c.metrics()[0];
    assert_eq!(m.started, 11);
    assert_eq!(m.completed, 11);
    assert_eq!(m.peak_reserved_bytes, 60);
}
#[test]
fn cancellation_queue_and_byte_budgets_prevent_work_start() {
    let c = coordinator(1);
    let held = c
        .acquire(request(ReadPriority::Interactive), &AtomicBool::new(false))
        .unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let (worker_c, worker_cancel) = (c.clone(), cancelled.clone());
    let waiting = thread::spawn(move || {
        worker_c
            .acquire(request(ReadPriority::Background), &worker_cancel)
            .err()
            .unwrap()
            .code
    });
    until(|| c.metrics()[0].queued == 1);
    assert_eq!(
        c.acquire(request(ReadPriority::Interactive), &AtomicBool::new(false))
            .err()
            .unwrap()
            .code,
        "READ_BUDGET_EXCEEDED"
    );
    cancelled.store(true, Ordering::Release);
    assert_eq!(waiting.join().unwrap(), "CANCELLED");
    assert_eq!(c.metrics()[0].started, 1);
    assert_eq!(c.metrics()[0].cancelled_waiting, 1);
    drop(held);
    assert_eq!(
        c.acquire(
            ReadRequest {
                bytes: 101,
                ..request(ReadPriority::Interactive)
            },
            &AtomicBool::new(false)
        )
        .err()
        .unwrap()
        .code,
        "READ_BUDGET_EXCEEDED"
    );
}
fn key(n: u32) -> String {
    format!("{n:064x}")
}
#[test]
fn cache_survives_restart_verifies_bytes_and_preserves_pins() {
    let temp = tempfile::tempdir().unwrap();
    let cache = PreviewCache::open(temp.path()).unwrap();
    let pin = cache.put(&key(1), b"valid preview").unwrap().unwrap();
    cache.set_quota(1).unwrap();
    assert_eq!(cache.metrics().entries, 1);
    assert_eq!(cache.clear().unwrap().entries, 1);
    drop(pin);
    assert_eq!(cache.metrics().entries, 0);
    cache.set_quota(100).unwrap();
    drop(cache.put(&key(2), b"valid preview").unwrap());
    let offline = cache.get(&key(2), false).unwrap().unwrap();
    assert!(offline.verified_ms > 0);
    drop(offline);
    drop(cache);
    let cache = PreviewCache::open(temp.path()).unwrap();
    assert_eq!(cache.metrics().entries, 1);
    assert_eq!(cache.metrics().quota_bytes, 100);
    assert_eq!(
        cache.get(&key(2), true).unwrap().unwrap().bytes,
        b"valid preview"
    );
    std::fs::write(
        temp.path().join("objects").join(format!("{}.jpg", key(2))),
        b"changed bytes",
    )
    .unwrap();
    assert!(cache.get(&key(2), true).unwrap().is_none());
    assert_eq!(cache.metrics().corrupt, 1);
    assert_eq!(cache.metrics().entries, 0);
}
#[test]
fn cache_maintenance_is_bounded_and_only_owned_material_is_removed() {
    let temp = tempfile::tempdir().unwrap();
    let cache = PreviewCache::open(temp.path()).unwrap();
    drop(cache);
    for n in 1..=300 {
        std::fs::write(
            temp.path().join("objects").join(format!("{}.jpg", key(n))),
            b"orphan",
        )
        .unwrap();
    }
    let keep = temp.path().join("objects/user-data.txt");
    std::fs::write(&keep, b"keep").unwrap();
    let cache = PreviewCache::open(temp.path()).unwrap();
    assert!(cache.metrics().maintenance_removed <= 128);
    assert!(cache.metrics().maintenance_pending);
    for _ in 0..5 {
        cache.maintain(128).unwrap();
    }
    assert_eq!(cache.metrics().maintenance_removed, 300);
    assert_eq!(std::fs::read(keep).unwrap(), b"keep");
    let unrelated = tempfile::tempdir().unwrap();
    std::fs::write(unrelated.path().join("project.db"), b"authority").unwrap();
    assert!(PreviewCache::open(unrelated.path()).is_err());
    assert_eq!(
        std::fs::read(unrelated.path().join("project.db")).unwrap(),
        b"authority"
    );
}
#[test]
fn keys_bind_content_parameters_and_source_identity_not_locations() {
    let mut source = Source {
        id: new_id(),
        name: "test".into(),
        kind: "danbooru".into(),
        index_root: None,
        media_root: None,
    };
    let a = preview_key(&source, "asset", "content-v1", 360);
    source.media_root = Some("another-drive".into());
    assert_eq!(a, preview_key(&source, "asset", "content-v1", 360));
    assert_ne!(a, preview_key(&source, "asset", "content-v2", 360));
    assert_ne!(a, preview_key(&source, "asset", "content-v1", 720));
    source.id = new_id();
    assert_ne!(a, preview_key(&source, "asset", "content-v1", 360));
}
#[test]
fn a_damaged_cache_index_is_isolated_without_touching_project_files() {
    let temp = tempfile::tempdir().unwrap();
    let cache = PreviewCache::open(temp.path()).unwrap();
    drop(cache.put(&key(1), b"cached").unwrap());
    cache.set_quota(0).unwrap();
    drop(cache);
    std::fs::write(temp.path().join("cache.sqlite"), b"damaged index").unwrap();
    let cache = PreviewCache::open(temp.path()).unwrap();
    assert!(cache.metrics().index_rebuilt);
    assert_eq!(cache.metrics().entries, 0);
    assert_eq!(cache.metrics().quota_bytes, 0);
    cache.set_quota(100).unwrap();
    drop(cache.put(&key(2), b"rebuilt preview").unwrap());
    assert_eq!(
        cache.get(&key(2), true).unwrap().unwrap().bytes,
        b"rebuilt preview"
    );
}
#[test]
fn clear_continues_in_bounded_batches_across_restart_and_suspends_new_writes() {
    let temp = tempfile::tempdir().unwrap();
    let cache = PreviewCache::open(temp.path()).unwrap();
    for n in 0..600 {
        drop(cache.put(&key(n), b"preview").unwrap());
    }
    let progress = cache.clear().unwrap();
    assert!(progress.clear_pending);
    assert!(progress.entries >= 344);
    assert!(cache.put(&key(999), b"new preview").unwrap().is_none());
    drop(cache);
    let cache = PreviewCache::open(temp.path()).unwrap();
    assert!(cache.metrics().clear_pending);
    for _ in 0..10 {
        cache.maintain(128).unwrap();
    }
    assert_eq!(cache.metrics().entries, 0);
    assert!(!cache.metrics().clear_pending);
    drop(cache.put(&key(999), b"new preview").unwrap());
    assert_eq!(cache.metrics().entries, 1);
}
#[test]
fn eviction_retains_the_recently_used_entry() {
    let temp = tempfile::tempdir().unwrap();
    let cache = PreviewCache::open(temp.path()).unwrap();
    drop(cache.put(&key(1), b"first").unwrap());
    drop(cache.put(&key(2), b"second").unwrap());
    thread::sleep(Duration::from_millis(10));
    drop(cache.get(&key(1), true).unwrap());
    cache.set_quota(6).unwrap();
    assert!(cache.get(&key(1), true).unwrap().is_some());
    assert!(cache.get(&key(2), true).unwrap().is_none());
}
