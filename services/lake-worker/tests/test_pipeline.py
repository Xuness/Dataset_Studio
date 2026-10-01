import json
import threading
import time
from concurrent.futures import ThreadPoolExecutor

import pytest

from conftest import png
from test_updates import setup, post, job, FakeSite, Images, ImageResponse
from studio_lake.updates import media, settings
from studio_lake.updates.archive import online
from studio_lake.updates.resources import Resources
from studio_lake.updates.runner import Runner
from studio_lake.updates.sites import Site, UpdateError
from studio_lake.updates.telemetry import Telemetry


def configure(state, **patch):
    current = settings.read(state)
    value = {
        **current["value"],
        "max_download_mib": 1,
        "spool_mib": 64,
        "reserve_mib": 0,
        "publish_interval_seconds": 0.1,
        **patch,
    }
    return settings.save(state, value, current["revision"])


def test_revisioned_tuning_reopens_and_does_not_change_existing_job(tmp_path):
    lib, state = setup(tmp_path, "yandere")
    task = job(state, lib, {"kind": "ids", "ids": [11]}, "original")
    before = task["definition"]
    saved = configure(state, encode_concurrency=3, download_mib_per_second=2.5)
    assert saved["revision"] == 1
    assert settings.read(type(state)(state.root))["value"]["encode_concurrency"] == 3
    assert state.job(task["id"])["definition"] == before
    with pytest.raises(UpdateError, match="changed"):
        settings.save(state, saved["value"], 0)
    for value in (
        {"encode_concurrency": True},
        {"scan_mode": "unknown"},
        {"spool_mib": 64},
        {"download_mib_per_second": float("nan")},
        {"sites": {"x": {}}},
    ):
        with pytest.raises(UpdateError):
            settings.validate(value)


def test_parallel_download_encode_overlap_and_monotonic_publication(tmp_path, monkeypatch):
    lib, state = setup(tmp_path, "yandere")
    data = png("red")
    rows = [post("yandere", i, data) for i in range(11, 29)]
    remote = FakeSite("yandere", rows)
    original_capabilities = remote.capabilities
    remote.capabilities = lambda: {**original_capabilities(), "page_size": 6}
    config = configure(state, encode_concurrency=2, publish_items=4)
    config["value"]["sites"]["yandere"]["download_concurrency"] = 3
    settings.save(state, config["value"], config["revision"])
    lock = threading.Lock()
    counts = {"down": 0, "encode": 0, "max_down": 0, "max_encode": 0, "overlap": False}
    overlap = threading.Event()

    class TimedResponse(ImageResponse):
        def __enter__(self):
            with lock:
                counts["down"] += 1
                counts["max_down"] = max(counts["max_down"], counts["down"])
                if counts["encode"]:
                    counts["overlap"] = True
                    overlap.set()
            return self

        def iter_content(self, _):
            time.sleep(0.08)
            yield self.data

        def __exit__(self, *_):
            with lock:
                counts["down"] -= 1

    class Transfers(Images):
        def get(self, *args, **kw):
            self.calls += 1
            return TimedResponse(self.data)

    real_encode = media.prepare_image

    def slow_encode(*args):
        with lock:
            counts["encode"] += 1
            counts["max_encode"] = max(counts["max_encode"], counts["encode"])
            counts["overlap"] |= counts["down"] > 0
            if counts["overlap"]:
                overlap.set()
        try:
            assert overlap.wait(3), "downloads must continue while an encoder is occupied"
            time.sleep(0.13)
            return real_encode(*args)
        finally:
            with lock:
                counts["encode"] -= 1

    monkeypatch.setattr(media, "prepare_image", slow_encode)
    task = job(state, lib, {"kind": "id_range", "start": 11, "end": 29}, "original")
    runner = Runner(state, {"yandere": remote}, image_http=Transfers(data))
    cursors = []
    real_publish = runner.publish_images

    def publish(*args):
        real_publish(*args)
        cursors.append(state.job(task["id"])["cursor"]["next_id"])

    monkeypatch.setattr(runner, "publish_images", publish)
    done = runner.run(task["id"])
    assert done["state"] == "completed", done
    assert done["counts"] == {"stored": 18}
    assert counts["max_down"] == 3 and counts["max_encode"] == 2 and counts["overlap"]
    assert cursors == sorted(cursors) and cursors[-1] == 29
    assert done["telemetry"]["downloaded_bytes"] == len(data) * 18
    assert done["telemetry"]["active_downloads"] == 0
    assert done["telemetry"]["download_rate_bps"] == 0
    assert not runner.resources.reservations
    with online(lib) as (db, _):
        assert next(db.execute("SELECT count(*) FROM assets"))[0] == 18
        assert next(db.execute("SELECT count(*) FROM objects"))[0] == 1


def test_sessions_are_thread_owned_reused_and_closed(tmp_path, monkeypatch):
    lib, state = setup(tmp_path, "danbooru")
    configure(state)
    data = png("blue")
    rows = [post("danbooru", i, data) for i in range(11, 29)]
    for row in rows:
        row["file_url"] = "https://cdn.donmai.us/fixture.png"
    remote = FakeSite("danbooru", rows)
    pools = []

    class Session:
        def __init__(self):
            self.headers, self.owners, self.closed = {}, [], False
            pools.append(self)

        def get(self, *_args, **_kw):
            self.owners.append(threading.get_ident())
            time.sleep(0.025)
            return ImageResponse(data)

        def close(self):
            self.closed = True

    monkeypatch.setattr(media.requests, "Session", Session)
    task = job(state, lib, {"kind": "id_range", "start": 11, "end": 29}, "original")
    done = Runner(state, {"danbooru": remote}).run(task["id"])
    assert done["state"] == "completed", done
    assert 1 < len(pools) <= 4
    assert sum(len(p.owners) for p in pools) == 18
    assert all(len(set(p.owners)) == 1 and p.closed for p in pools)
    assert any(len(p.owners) > 1 for p in pools)


def test_slow_metadata_request_does_not_block_image_publication(tmp_path, monkeypatch):
    lib, state = setup(tmp_path, "danbooru")
    configure(state)
    data = png("red")
    held, release, published = threading.Event(), threading.Event(), threading.Event()

    class SlowSite(FakeSite):
        def capabilities(self):
            return {**super().capabilities(), "page_size": 3}

        def request(self, params, cancelled=lambda: False, resource="posts"):
            if resource == "posts" and "id:14..16" in params.get("tags", ""):
                held.set()
                assert release.wait(10)
            return super().request(params, cancelled, resource)

    remote = SlowSite("danbooru", [post("danbooru", i, data) for i in range(11, 17)])
    task = job(state, lib, {"kind": "id_range", "start": 11, "end": 17}, "original")
    runner = Runner(state, {"danbooru": remote}, image_http=Images(data))
    original = runner.publish_images

    def publish(*args):
        original(*args)
        if state.job(task["id"])["counts"].get("stored", 0) >= 3:
            published.set()

    monkeypatch.setattr(runner, "publish_images", publish)
    with ThreadPoolExecutor(max_workers=1) as pool:
        future = pool.submit(runner.run, task["id"])
        try:
            assert held.wait(10)
            assert published.wait(5), "an unfinished API response must not hold the image publisher"
            assert state.job(task["id"])["counts"]["stored"] == 3
        finally:
            release.set()
        done = future.result(timeout=15)
    assert done["state"] == "completed" and done["counts"] == {"stored": 6}, done


def test_pause_keeps_download_receipts_and_resumes_without_http(tmp_path, monkeypatch):
    from studio_lake.updates import pipeline

    lib, state = setup(tmp_path, "danbooru")
    configure(state, buffer_images=4, encode_concurrency=1)
    data = png("red")
    remote = FakeSite("danbooru", [post("danbooru", i, data) for i in range(11, 15)])
    images = Images(data)
    task = job(state, lib, {"kind": "ids", "ids": list(range(11, 15))}, "original")
    runner = Runner(state, {"danbooru": remote}, image_http=images)
    entered = threading.Event()

    def hold_encode(_lib, _job, _item, _download, _resources, cancelled, _progress):
        entered.set()
        until = time.monotonic() + 10
        while not cancelled() and time.monotonic() < until:
            time.sleep(0.02)
        raise UpdateError("CANCELLED", "fixture pause")

    with monkeypatch.context() as patch, ThreadPoolExecutor(max_workers=1) as pool:
        patch.setattr(pipeline, "encode_download", hold_encode)
        future = pool.submit(runner.run, task["id"])
        assert entered.wait(10)
        directory = lib.cache / "updates" / task["id"]
        until = time.monotonic() + 10
        while len(list(directory.glob("*.download.json"))) < 4 and time.monotonic() < until:
            time.sleep(0.02)
        state.action(task["id"], "pause")
        assert future.result(timeout=15)["state"] == "paused"
    assert len(list(directory.glob("*.downloaded"))) == 4
    assert not runner.resources.reservations
    calls = images.calls
    state.action(task["id"], "resume")
    done = Runner(state, {"danbooru": remote}, image_http=images).run(task["id"])
    assert done["state"] == "completed" and done["counts"] == {"stored": 4}, done
    assert images.calls == calls == 4
    assert not list(directory.glob("*.downloaded")) and not list(directory.glob("*.ready"))


def test_filtered_local_scans_count_budget_and_finish_at_last_page(tmp_path):
    lib, state = setup(tmp_path, "danbooru")
    configure(state)
    data = png("red")
    remote = FakeSite("danbooru", [post("danbooru", i, data) for i in range(11, 14)])
    cap = remote.capabilities
    remote.capabilities = lambda: {**cap(), "page_size": 2}
    runner = Runner(state, {"danbooru": remote}, image_http=Images(data))
    initial = job(state, lib, {"kind": "id_range", "start": 11, "end": 14}, "original")
    assert runner.run(initial["id"])["state"] == "completed"
    calls = len(remote.calls)
    missing = job(state, lib, {"kind": "local", "missing_media": True}, "original", page_budget=1)
    first = runner.run(missing["id"])
    assert first["state"] == "paused" and first["cursor"]["pages"] == 1
    state.action(missing["id"], "resume")
    done = runner.run(missing["id"])
    assert done["state"] == "completed" and done["cursor"]["pages"] == 2, done
    assert len(remote.calls) == calls and done["counts"] == {}


def test_media_commit_crash_reconciles_before_reusing_old_ready_files(tmp_path, monkeypatch):
    import studio_lake.updates.runner as module

    lib, state = setup(tmp_path, "danbooru")
    configure(state)
    data = png("red")
    remote, images = FakeSite("danbooru", [post("danbooru", 11, data)]), Images(data)
    task = job(state, lib, {"kind": "ids", "ids": [11]}, "original")
    original = module.reconcile

    def crash(control, library, identity):
        with library.journal() as db:
            media_committed = any(
                json.loads(r[0])["source"].get("update_role") == "media"
                for r in db.execute("SELECT manifest_json FROM commits")
            )
        if media_committed:
            raise RuntimeError("fixture crash before media queue reconcile")
        original(control, library, identity)

    with monkeypatch.context() as patch:
        patch.setattr(module, "reconcile", crash)
        done = Runner(state, {"danbooru": remote}, image_http=images).run(task["id"])
        assert done["state"] == "needs_review", done
    directory = lib.cache / "updates" / task["id"]
    assert len(list(directory.glob("*.ready"))) == 1
    state.action(task["id"], "resume")
    done = Runner(state, {"danbooru": remote}, image_http=images).run(task["id"])
    assert done["state"] == "completed" and done["counts"] == {"stored": 1}
    assert images.calls == 1 and not list(directory.glob("*.ready"))
    assert state.items(task["id"])["items"][0]["attempts"] == 1


def test_metadata_first_and_final_local_page_are_not_skipped(tmp_path):
    lib, state = setup(tmp_path, "danbooru")
    configure(state, scan_mode="metadata_first")
    data = png("red")
    rows = [post("danbooru", i, data) for i in range(11, 17)]
    remote = FakeSite("danbooru", rows)
    capabilities = remote.capabilities
    remote.capabilities = lambda: {**capabilities(), "page_size": 2}
    task = job(state, lib, {"kind": "id_range", "start": 11, "end": 17}, "original", page_budget=3)

    class AfterScan(Images):
        def get(self, *args, **kw):
            assert state.job(task["id"])["cursor"]["metadata_complete"]
            return super().get(*args, **kw)

    runner = Runner(state, {"danbooru": remote}, image_http=AfterScan(data))
    assert runner.run(task["id"])["state"] == "completed"
    for row in rows:
        row["score"] = 123
    refresh = job(state, lib, {"kind": "local"})
    done = runner.run(refresh["id"])
    assert done["state"] == "completed" and done["counts"] == {"metadata": 6}, done
    with online(lib) as (db, _):
        assert (
            next(
                db.execute(
                    "SELECT count(*) FROM post_versions p JOIN observations o USING(row_id) WHERE p.valid_until IS NULL AND o.score=123"
                )
            )[0]
            == 6
        )


def test_shared_encoder_memory_admission_and_live_limit_changes(tmp_path):
    resources = Resources()
    config = settings.validate({"encode_concurrency": 1, "decode_memory_mib": 64})
    resources.configure(config)
    entered, release = threading.Event(), threading.Event()

    def work():
        with resources.encoding(48 * settings.MIB, lambda: False):
            entered.set()
            assert release.wait(5)

    with ThreadPoolExecutor(max_workers=2) as pool:
        first = pool.submit(work)
        assert entered.wait(3)
        second_entered = threading.Event()

        def second():
            with resources.encoding(48 * settings.MIB, lambda: False):
                second_entered.set()

        second_future = pool.submit(second)
        resources.configure({**config, "encode_concurrency": 2})
        assert not second_entered.wait(0.1), "memory bound must still apply"
        resources.configure({**config, "encode_concurrency": 2, "decode_memory_mib": 128})
        assert second_entered.wait(3)
        release.set()
        first.result(timeout=5)
        second_future.result(timeout=5)
    assert resources.encode_jobs == resources.decode_bytes == 0


def test_spool_admission_and_release_do_not_overbook(tmp_path):
    # Production paths are updates/<job>; do not count sibling pytest cases as other jobs.
    tmp_path = tmp_path / "updates" / "job"
    resources = Resources(max_download_bytes=1024, spool_bytes=8192, reserve_bytes=0)
    a = resources.try_reserve(tmp_path, "a")
    b = resources.try_reserve(tmp_path, "b")
    assert a and b and resources.try_reserve(tmp_path, "c") is None
    a.release()
    c = resources.try_reserve(tmp_path, "a")
    a.release()
    assert c and resources.try_reserve(tmp_path, "d") is None
    b.release()
    c.release()
    with pytest.raises(UpdateError, match="reserved"):
        resources.check_staged_size(tmp_path, "none", 9000)


def test_download_slots_are_shared_per_site_and_lowering_does_not_cancel():
    resources = Resources()
    slots = [resources.try_download("yandere") for _ in range(4)]
    assert all(slots) and resources.try_download("yandere") is None
    other = resources.try_download("gelbooru")
    assert other is not None
    cfg = settings.validate({"sites": {"yandere": {"download_concurrency": 1}}})
    resources.configure(cfg)
    for slot in slots[:3]:
        slot.release()
    assert resources.try_download("yandere") is None
    slots[3].release()
    new = resources.try_download("yandere")
    slots[3].release()
    assert new is not None and resources.try_download("yandere") is None
    new.release()
    other.release()
    assert resources.download_jobs == {"yandere": 0, "gelbooru": 0}


def test_shared_site_observers_are_isolated_between_job_threads():
    http = Images(b"[]")
    site = Site("yandere", http=http, delay=0)
    barrier = threading.Barrier(2)

    def request(identity):
        seen = []
        site.observer = lambda **values: seen.append((identity, values))
        barrier.wait(timeout=5)
        assert site.request({"limit": 1}).status == 200
        site.observer = None
        return seen

    with ThreadPoolExecutor(max_workers=2) as pool:
        a, b = pool.submit(request, "a"), pool.submit(request, "b")
        assert a.result(timeout=10) == [("a", {"api_requests_delta": 1})]
        assert b.result(timeout=10) == [("b", {"api_requests_delta": 1})]
    assert site.observer is None


def test_disk_reservations_share_remaining_free_space(tmp_path, monkeypatch):
    from types import SimpleNamespace
    import studio_lake.updates.resources as module

    monkeypatch.setattr(module.shutil, "disk_usage", lambda _: SimpleNamespace(free=10 * 1024))
    resource = Resources(max_download_bytes=1024, spool_bytes=64 * 1024, reserve_bytes=0)
    first = resource.try_reserve(tmp_path / "a" / "job", "one")
    second = resource.try_reserve(tmp_path / "b" / "job", "two")
    assert first and second
    assert resource.try_reserve(tmp_path / "c" / "job", "three") is None
    first.release()
    third = resource.try_reserve(tmp_path / "c" / "job", "three")
    assert third
    second.release()
    third.release()


def test_reuse_and_posts_without_urls_do_not_require_download_spool(tmp_path):
    lib, state = setup(tmp_path, "danbooru")
    configure(state)
    data = png("red")
    records = [post("danbooru", 11, data), post("danbooru", 12, data)]
    remote, images = FakeSite("danbooru", records), Images(data)
    runner = Runner(state, {"danbooru": remote}, image_http=images)
    first = job(state, lib, {"kind": "ids", "ids": [11]}, "original")
    assert runner.run(first["id"])["counts"] == {"stored": 1}
    configure(state, spool_mib=64, max_download_mib=16)
    records[1]["is_deleted"] = True
    records[1].pop("file_url")
    second = state.create(
        {
            "library_id": lib.info["library_id"],
            "range": {"kind": "ids", "ids": [11, 12]},
            "media": {
                "profile": "custom",
                "existing": "keep",
                "encoding": {
                    "version": 1,
                    "format": "png",
                    "max_edge": None,
                    "animation": "preserve",
                    "alpha": "preserve",
                },
            },
        },
        "reuse-small-spool",
    )
    done = Runner(state, {"danbooru": remote}, image_http=images).run(second["id"])
    assert done["state"] == "completed_with_exclusions", done
    assert done["counts"] == {"reused": 1, "unavailable": 1} and images.calls == 1


@pytest.mark.parametrize("space_remains", [True, False])
def test_publishing_frees_spool_before_rechecking_disk_pressure(tmp_path, monkeypatch, space_remains):
    from types import SimpleNamespace
    import studio_lake.updates.resources as module

    lib, state = setup(tmp_path, "danbooru")
    configure(state, publish_items=32, publish_interval_seconds=60)
    data = png("blue")
    remote = FakeSite("danbooru", [post("danbooru", pid, data) for pid in range(11, 15)])
    images = Images(data)
    # Only one image can hold a staging lease; releasing it must unblock the next.
    resources = Resources(max_download_bytes=16 * 1024, spool_bytes=100 * 1024, reserve_bytes=0)
    runner = Runner(state, {"danbooru": remote}, resources=resources, image_http=images)
    task = job(state, lib, {"kind": "id_range", "start": 11, "end": 15}, "original")
    published = []
    original = runner.publish_images

    def publish(*args):
        original(*args)
        published.extend(r["post_id"] for r in args[2])

    monkeypatch.setattr(runner, "publish_images", publish)
    monkeypatch.setattr(module.shutil, "disk_usage", lambda _: SimpleNamespace(
        free=1024 * 1024 if space_remains or not published else 0,
    ))
    done = runner.run(task["id"])
    if not space_remains:
        assert done["state"] == "waiting_space" and done["counts"] == {"stored": 1, "pending": 3}, done
        assert published == [11] and not resources.reservations
        space_remains = True
        state.action(task["id"], "resume")
        done = runner.run(task["id"])
    assert done["state"] == "completed" and done["counts"] == {"stored": 4}, done
    assert published == [11, 12, 13, 14] and images.calls == 4
    assert not resources.reservations


def test_disk_full_waits_and_preserves_metadata_checkpoint(tmp_path, monkeypatch):
    from pathlib import Path
    import errno

    lib, state = setup(tmp_path, "danbooru")
    configure(state)
    data = png("red")
    remote, images = FakeSite("danbooru", [post("danbooru", 11, data)]), Images(data)
    task = job(state, lib, {"kind": "ids", "ids": [11]}, "original")
    original = Path.open

    def full(path, *args, **kwargs):
        if path.suffix == ".partial" and args and args[0] == "wb":
            raise OSError(errno.ENOSPC, "fixture disk full")
        return original(path, *args, **kwargs)

    with monkeypatch.context() as patch:
        patch.setattr(Path, "open", full)
        held = Runner(state, {"danbooru": remote}, image_http=images).run(task["id"])
    assert held["state"] == "waiting_space" and held["counts"] == {"pending": 1}, held
    assert held["cursor"]["metadata_complete"]
    state.action(task["id"], "resume")
    done = Runner(state, {"danbooru": remote}, image_http=images).run(task["id"])
    assert done["state"] == "completed" and done["counts"] == {"stored": 1}, done
    assert len(remote.calls) == 1


def test_media_rate_lane_has_shared_cooldown_and_cancellable_wait(tmp_path):
    from studio_lake.updates.rate import wait_start, cooldown

    wait_start(tmp_path, "yandere", lambda: False, lambda: 50)
    cooldown(tmp_path, "yandere", 0.15, image=True)
    before = time.monotonic()
    wait_start(tmp_path, "yandere", lambda: False, lambda: None)
    assert time.monotonic() - before >= 0.12
    cooldown(tmp_path, "yandere", 60, image=True)
    with pytest.raises(UpdateError) as error:
        wait_start(tmp_path, "yandere", lambda: True, lambda: 50)
    assert error.value.code == "CANCELLED"


def test_parallel_telemetry_aggregates_without_last_file_overwrite(tmp_path):
    lib, state = setup(tmp_path, "danbooru")
    task = job(state, lib, {"kind": "ids", "ids": [11, 12]}, "original")
    t = Telemetry(state, task)
    t.add(11, phase="downloading", queue_state="pending", downloaded_bytes_delta=100, current_bytes=100)
    t.add(12, phase="downloading", queue_state="pending", downloaded_bytes_delta=200, current_bytes=200)
    t.flush(force=True)
    sample = state.job(task["id"])["telemetry"]
    assert sample["downloaded_bytes"] == 300 and sample["active_downloads"] == 2
    assert {f["post_id"] for f in sample["files"]} == {11, 12}
    t.add(11, phase="processing_image", encode_seconds_delta=2)
    t.flush(force=True)
    sample = state.job(task["id"])["telemetry"]
    assert sample["active_downloads"] == sample["active_encodes"] == 1
    assert sample["timings_seconds"]["encode"] == 2
    t.flush(stopping="paused", force=True)
    sample = state.job(task["id"])["telemetry"]
    assert sample["download_rate_bps"] == sample["publish_rate_images_per_second"] == 0
    assert sample["files"] == [] and sample["current_bytes"] is None


def test_download_concurrency_can_be_tuned_while_running(tmp_path):
    lib, state = setup(tmp_path, "danbooru")
    configure(state, sites={"danbooru": {"download_concurrency": 1}})
    data, gate, first, three = png("blue"), threading.Event(), threading.Event(), threading.Event()
    lock = threading.Lock()
    started = []

    class HeldResponse(ImageResponse):
        def iter_content(self, _):
            with lock:
                started.append(time.monotonic())
                first.set()
                if len(started) >= 3:
                    three.set()
            assert gate.wait(10)
            yield self.data

    class HeldImages(Images):
        def get(self, *args, **kw):
            self.calls += 1
            return HeldResponse(self.data)

    remote = FakeSite("danbooru", [post("danbooru", i, data) for i in range(11, 17)])
    task = job(state, lib, {"kind": "ids", "ids": list(range(11, 17))}, "original")
    runner = Runner(state, {"danbooru": remote}, image_http=HeldImages(data))
    with ThreadPoolExecutor(max_workers=1) as pool:
        future = pool.submit(runner.run, task["id"])
        try:
            assert first.wait(10)
            before = settings.read(state)
            before["value"]["sites"]["danbooru"]["download_concurrency"] = 3
            higher = settings.save(state, before["value"], before["revision"])
            assert three.wait(5), "new transfer slots must appear without restarting the job"
            higher["value"]["sites"]["danbooru"]["download_concurrency"] = 1
            lower = settings.save(state, higher["value"], higher["revision"])
            until = time.monotonic() + 5
            while runner.settings_revision != lower["revision"] and time.monotonic() < until:
                time.sleep(0.02)
            assert runner.settings_revision == lower["revision"]
        finally:
            gate.set()
        done = future.result(timeout=15)
    assert done["state"] == "completed" and done["counts"] == {"stored": 6}, done


def test_http_429_persists_media_cooldown(tmp_path):
    lib, state = setup(tmp_path, "danbooru")
    configure(state)
    data = png("red")
    remote = FakeSite("danbooru", [post("danbooru", 11, data)])
    task = job(state, lib, {"kind": "ids", "ids": [11]})
    assert Runner(state, {"danbooru": remote}).run(task["id"])["state"] == "completed"
    task["definition"]["media"]["profile"] = "original"
    with state.db() as db:
        item = dict(db.execute("SELECT * FROM items WHERE job_id=?", (task["id"],)).fetchone())

    class Limited(Images):
        def get(self, *args, **kw):
            r = ImageResponse(b"")
            r.status_code, r.headers = 429, {"Retry-After": "2"}
            return r

    remote.rate_root = state.root
    before = time.time()
    result = media.download(
        lib,
        task,
        item,
        remote,
        Resources(reserve_bytes=0),
        lambda: False,
        media.ImageSessions("danbooru", Limited(data)),
    )
    assert result["state"] == "failed" and result["reason"] == "image_http_429"
    marker = json.loads((state.root / "rate-image-danbooru.json").read_text("utf-8"))
    assert marker["cooldown_until"] >= before + 2
