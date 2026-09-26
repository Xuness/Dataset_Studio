use super::*;
use std::{sync::mpsc, thread, time::Duration};

fn fixture() -> (tempfile::TempDir, PreviewCache) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
    fs::create_dir_all(&root).unwrap();
    let root = tempfile::Builder::new()
        .prefix("preview-concurrency-")
        .tempdir_in(root)
        .unwrap();
    let cache = PreviewCache::open(root.path()).unwrap();
    (root, cache)
}
fn key(n: u32) -> String {
    format!("{n:064x}")
}
const WAIT: Duration = Duration::from_secs(3);

#[test]
fn warm_read_and_maintenance_continue_during_an_unrelated_write() {
    let (_root, cache) = fixture();
    drop(cache.put(&key(1), b"warm").unwrap());
    thread::scope(|scope| {
        let (entered, paused) = mpsc::channel();
        let (resume, go) = mpsc::channel();
        let writer = cache.clone();
        let cold = scope.spawn(move || {
            writer.put_with(&key(2), b"cold", || {
                entered.send(()).unwrap();
                go.recv_timeout(WAIT * 2).unwrap();
            })
        });
        paused.recv_timeout(WAIT).unwrap();
        let partial = cache
            .inner
            .lock()
            .unwrap()
            .partials
            .iter()
            .next()
            .unwrap()
            .clone();
        // Explicitly rescan while the temporary file is active.
        cache.inner.lock().unwrap().scan = Some(fs::read_dir(partial.parent().unwrap()).unwrap());
        cache.maintain(128).unwrap();
        assert!(partial.exists());
        let (send, receive) = mpsc::channel();
        let reader = cache.clone();
        scope.spawn(move || send.send(reader.get(&key(1), true)).unwrap());
        let warm = receive.recv_timeout(WAIT);
        resume.send(()).unwrap();
        assert_eq!(
            warm.expect("warm hit waited for unrelated write")
                .unwrap()
                .unwrap()
                .bytes,
            b"warm"
        );
        drop(cold.join().unwrap().unwrap());
    });
    assert!(cache.inner.lock().unwrap().partials.is_empty());
    assert_eq!(cache.get(&key(2), true).unwrap().unwrap().bytes, b"cold");
}

#[test]
fn read_pin_precedes_file_io_and_clear_finishes_only_in_maintenance() {
    let (_root, cache) = fixture();
    drop(cache.put(&key(1), b"pinned").unwrap());
    thread::scope(|scope| {
        let (entered, paused) = mpsc::channel();
        let (resume, go) = mpsc::channel();
        let reader = cache.clone();
        let work = scope.spawn(move || {
            reader.get_with(&key(1), true, || {
                entered.send(()).unwrap();
                go.recv_timeout(WAIT * 2).unwrap();
            })
        });
        paused.recv_timeout(WAIT).unwrap();
        let cleared = cache.clear().unwrap();
        assert_eq!(cleared.entries, 1);
        assert_eq!(cleared.pinned, 1);
        assert!(cleared.clear_pending);
        assert!(cache.put(&key(1), b"replacement").unwrap().is_none());
        resume.send(()).unwrap();
        let cached = work.join().unwrap().unwrap().unwrap();
        assert_eq!(cached.bytes, b"pinned");
        drop(cached);
    });
    assert_eq!(cache.metrics().entries, 1);
    cache.maintain(128).unwrap();
    assert_eq!(cache.metrics().entries, 0);
    assert!(!cache.metrics().clear_pending);
}

#[test]
fn clear_and_quota_reduction_also_cover_inflight_writes_and_survive_restart() {
    for clear in [true, false] {
        let (root, cache) = fixture();
        thread::scope(|scope| {
            let (entered, paused) = mpsc::channel();
            let (resume, go) = mpsc::channel();
            let writer = cache.clone();
            let work = scope.spawn(move || {
                writer.put_with(&key(1), b"not retained", || {
                    entered.send(()).unwrap();
                    go.recv_timeout(WAIT * 2).unwrap();
                })
            });
            paused.recv_timeout(WAIT).unwrap();
            if clear {
                assert!(cache.clear().unwrap().clear_pending);
            } else {
                cache.set_quota(0).unwrap();
            }
            resume.send(()).unwrap();
            assert!(work.join().unwrap().unwrap().is_none());
        });
        cache.maintain(128).unwrap();
        assert_eq!(cache.metrics().entries, 0);
        assert!(cache.inner.lock().unwrap().busy.is_empty());
        assert_eq!(cache.inner.lock().unwrap().staged_bytes, 0);
        drop(cache);
        let cache = PreviewCache::open(root.path()).unwrap();
        assert_eq!(cache.metrics().entries, 0);
        assert!(!cache.metrics().clear_pending);
        assert_eq!(cache.metrics().quota_bytes == 0, !clear);
    }
}

#[test]
fn unlink_reserves_only_its_key_and_does_not_block_other_hits() {
    let (_root, cache) = fixture();
    drop(cache.put(&key(1), b"evict").unwrap());
    drop(cache.put(&key(2), b"keep").unwrap());
    thread::scope(|scope| {
        let (entered, paused) = mpsc::channel();
        let (resume, go) = mpsc::channel();
        let evictor = cache.clone();
        let removal = scope.spawn(move || {
            evictor.remove_with(&key(1), || {
                entered.send(()).unwrap();
                go.recv_timeout(WAIT * 2).unwrap();
            })
        });
        paused.recv_timeout(WAIT).unwrap();
        assert!(cache.get(&key(1), true).unwrap().is_none());
        assert!(cache.put(&key(1), b"replace").unwrap().is_none());
        let (send, receive) = mpsc::channel();
        let reader = cache.clone();
        scope.spawn(move || send.send(reader.get(&key(2), true)).unwrap());
        let warm = receive.recv_timeout(WAIT);
        resume.send(()).unwrap();
        assert_eq!(
            warm.expect("unrelated hit waited for unlink")
                .unwrap()
                .unwrap()
                .bytes,
            b"keep"
        );
        assert!(removal.join().unwrap().unwrap());
    });
    assert_eq!(cache.metrics().entries, 1);
}

#[test]
fn offline_read_cannot_revert_a_concurrent_online_verification_time() {
    let (_root, cache) = fixture();
    drop(cache.put(&key(1), b"shared").unwrap());
    cache
        .inner
        .lock()
        .unwrap()
        .db
        .execute("UPDATE entries SET verified_ms=1", [])
        .unwrap();
    thread::scope(|scope| {
        let (entered, paused) = mpsc::channel();
        let (resume, go) = mpsc::channel();
        let reader = cache.clone();
        let offline = scope.spawn(move || {
            reader.get_with(&key(1), false, || {
                entered.send(()).unwrap();
                go.recv_timeout(WAIT * 2).unwrap();
            })
        });
        paused.recv_timeout(WAIT).unwrap();
        let online = cache.get(&key(1), true).unwrap().unwrap().verified_ms;
        resume.send(()).unwrap();
        assert!(online > 1);
        assert_eq!(
            offline.join().unwrap().unwrap().unwrap().verified_ms,
            online
        );
    });
}
