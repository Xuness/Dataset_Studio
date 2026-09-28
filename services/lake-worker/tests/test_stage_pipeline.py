"""Independent stage progress, bounded admission, indexed reads and durable retries."""

from contextlib import contextmanager
from concurrent.futures import ThreadPoolExecutor
import json
import threading
import time

import pytest

from conftest import png
from test_updates import setup, post, job, FakeSite, Images, ImageResponse
from test_pipeline import configure
from studio_lake.updates import media, pipeline, settings
from studio_lake.updates.candidates import eligible
from studio_lake.updates.resources import Resources
from studio_lake.updates.runner import Runner
from studio_lake.updates.sites import UpdateError
from studio_lake.updates.staging import estimate, OVERHEAD


def until(predicate, timeout=5):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.02)
    assert predicate(), "stage did not advance before timeout"


def test_slow_publisher_does_not_stop_download_admission_or_encoding(tmp_path, monkeypatch):
    lib, state = setup(tmp_path, "danbooru")
    configure(state, buffer_images=16, publish_items=1, encode_concurrency=2)
    data = png("blue")
    remote = FakeSite("danbooru", [post("danbooru", i, data) for i in range(11, 43)])
    images = Images(data)
    task = job(state, lib, {"kind": "id_range", "start": 11, "end": 43}, "original")
    runner = Runner(state, {"danbooru": remote}, image_http=images)
    held, release = threading.Event(), threading.Event()
    encoded, calls_at_hold = [], []
    original_publish, original_encode = runner.publish_images, media.prepare_image

    def publish(*args):
        if not held.is_set():
            calls_at_hold.append(images.calls)
            held.set()
            assert release.wait(10)
        original_publish(*args)

    def encode(*args):
        result = original_encode(*args)
        encoded.append(1)
        return result

    monkeypatch.setattr(runner, "publish_images", publish)
    monkeypatch.setattr(media, "prepare_image", encode)
    with ThreadPoolExecutor(max_workers=1) as executor:
        future = executor.submit(runner.run, task["id"])
        try:
            assert held.wait(5)
            until(lambda: images.calls >= calls_at_hold[0] + 4 and len(encoded) >= 8)
            assert images.calls <= 16, "unpublished stages must still obey the in-flight bound"
            assert len(runner.resources.reservations) <= 16
            until(lambda: state.job(task["id"])["telemetry"].get("publishing_images") == 1)
        finally:
            release.set()
        done = future.result(timeout=15)
    assert done["state"] == "completed" and done["counts"] == {"stored": 32}, done
    assert images.calls == 32 and not runner.resources.reservations


@pytest.mark.parametrize("code", ["UPDATE_NETWORK", "UPDATE_REMOTE_ERROR"])
def test_api_retry_keeps_existing_media_pipeline_running(tmp_path, monkeypatch, code):
    lib, state = setup(tmp_path, "danbooru")
    configure(state, publish_items=1)
    monkeypatch.setattr(pipeline, "API_RETRY_SECONDS", 1)
    data = png("red")
    failed = threading.Event()

    class UnstableSite(FakeSite):
        def capabilities(self):
            return {**super().capabilities(), "page_size": 2}

        def request(self, params, cancelled=lambda: False, resource="posts"):
            if resource == "posts" and "id:13..14" in params.get("tags", "") and not failed.is_set():
                failed.set()
                raise UpdateError(code, "fixture temporary API failure", retry_after=2)
            return super().request(params, cancelled, resource)

    remote = UnstableSite("danbooru", [post("danbooru", i, data) for i in range(11, 15)])
    task = job(state, lib, {"kind": "id_range", "start": 11, "end": 15}, "original")
    images = Images(data)
    runner = Runner(state, {"danbooru": remote}, image_http=images)
    with ThreadPoolExecutor(max_workers=1) as executor:
        future = executor.submit(runner.run, task["id"])
        try:
            assert failed.wait(5)
            until(lambda: state.job(task["id"])["counts"].get("stored") == 2
                  and state.job(task["id"])["telemetry"].get("metadata_error_code") == code)
            current = state.job(task["id"])
            assert current["state"] == "running"
            assert current["telemetry"]["metadata_error_code"] == code
            assert current["telemetry"]["metadata_retry_at"] > time.time()
            assert not current["cursor"]["metadata_complete"]
        except BaseException:
            runner.stop.set()
            raise
        done = future.result(timeout=10)
    assert done["state"] == "completed" and done["counts"] == {"stored": 4}, done
    assert done["telemetry"]["metadata_retries"] == 1
    assert done["telemetry"]["metadata_retry_at"] is None
    assert done["telemetry"]["metadata_error_code"] is None
    assert images.calls == 4


def test_candidate_read_cost_does_not_grow_with_completed_history(tmp_path, monkeypatch):
    lib, state = setup(tmp_path, "danbooru")
    task = job(state, lib, {"kind": "id_range", "start": 1, "end": 20000})
    with state.db() as db:
        db.executemany("INSERT INTO items(job_id,post_id,record_json,state) VALUES(?,?,'{}','stored')",
                       ((task["id"], i) for i in range(1, 10001)))
        for pid, status, retry, attempts in [
            (10001, "pending", 0, 0), (10002, "failed", time.time() + 3600, 1),
            (10003, "pending_metadata", 0, 0), (10004, "failed", 0, 1),
            (10005, "pending", 0, 8), (10006, "failed", 0, 1), (10007, "pending", 0, 0),
        ]:
            db.execute("INSERT INTO items(job_id,post_id,record_json,state,retry_at,attempts) VALUES(?,?,'{}',?,?,?)",
                       (task["id"], pid, status, retry, attempts))
    original = state.db
    steps = []

    @contextmanager
    def counted():
        with original() as db:
            db.raw.set_progress_handler(lambda: steps.append(100) or False, 100)
            try:
                yield db
            finally:
                db.raw.set_progress_handler(None)

    monkeypatch.setattr(state, "db", counted)
    assert [row["post_id"] for row in eligible(state, task["id"], 3)] == [10001, 10003, 10004]
    assert sum(steps) < 5000, "candidate selection must not walk 10000 completed rows"


def test_sixteen_downloads_fit_realistic_staging_budget(tmp_path):
    lib, state = setup(tmp_path, "danbooru")
    config = configure(state, spool_mib=8192, max_download_mib=256, buffer_images=32,
                       sites={"danbooru": {"download_concurrency": 16}})
    assert config["value"]["max_download_mib"] == 256
    data = png("blue")
    records = [post("danbooru", i, data) for i in range(11, 27)]
    for record in records:
        record["file_size"] = len(data)
    release, all_started = threading.Event(), threading.Event()
    lock = threading.Lock()
    count = 0

    class HeldResponse(ImageResponse):
        headers = {"Content-Length": str(len(data))}

        def iter_content(self, _):
            nonlocal count
            with lock:
                count += 1
                if count == 16:
                    all_started.set()
            assert release.wait(10)
            yield self.data

    class HeldImages(Images):
        def get(self, *args, **kwargs):
            self.calls += 1
            return HeldResponse(self.data)

    task = job(state, lib, {"kind": "id_range", "start": 11, "end": 27}, "webp-2048-q95")
    runner = Runner(state, {"danbooru": FakeSite("danbooru", records)}, image_http=HeldImages(data))
    with ThreadPoolExecutor(max_workers=1) as executor:
        future = executor.submit(runner.run, task["id"])
        try:
            assert all_started.wait(5), "16 small images must not reserve 16 GiB"
            snapshot = runner.resources.snapshot()
            assert snapshot["staging_reserved_bytes"] < 16 * settings.MIB
        finally:
            release.set()
        done = future.result(timeout=15)
    assert done["state"] == "completed" and done["counts"] == {"stored": 16}, done


def test_stage_budget_shrinks_and_reserves_the_pack_copy(tmp_path):
    resources = Resources(reserve_bytes=0)
    directory = tmp_path / "updates" / "job"
    policy = {"profile": "custom", "encoding": {"max_edge": 2048}}
    plan = estimate(policy, {"file_size": 6 * settings.MIB, "width": 5000, "height": 4000}, resources)
    assert plan.peak() == 64 * settings.MIB + OVERHEAD
    lease = resources.try_reserve(directory, "item", plan.peak())
    resources.plan(directory, "item", plan)
    ready_size = 512 * 1024
    (directory / "item.ready").write_bytes(b"x" * ready_size)
    resources.stage_size(directory, "item", 2 * ready_size + OVERHEAD)
    snapshot = resources.snapshot()
    assert snapshot["staging_bytes"] == ready_size
    assert snapshot["staging_reserved_bytes"] == 2 * ready_size + OVERHEAD
    lease.release()
    assert resources.snapshot()["staging_reserved_bytes"] == ready_size
    assert not resources.plans


def test_size_correction_refuses_overbooking_without_losing_the_lease(tmp_path):
    resources = Resources(max_download_bytes=16 * settings.MIB, spool_bytes=24 * settings.MIB, reserve_bytes=0)
    directory = tmp_path / "updates" / "job"
    plan = estimate({"profile": "original"}, {"file_size": settings.MIB}, resources)
    a = resources.try_reserve(directory, "a", plan.peak())
    b = resources.try_reserve(directory, "b", plan.peak())
    resources.plan(directory, "a", plan)
    before = dict(resources.reservations)
    with pytest.raises(UpdateError) as error:
        resources.download_size(directory, "a", 11 * settings.MIB)
    assert error.value.code == "UPDATE_SPACE" and resources.reservations == before
    b.release()
    resources.download_size(directory, "a", 11 * settings.MIB)
    a.release()


def test_unknown_length_download_grows_in_chunks_and_shrinks_at_eof(tmp_path):
    from test_download_resume import HTTP, Response, URL
    from studio_lake.updates.media import ImageSessions
    from studio_lake.updates.transfer import fetch
    from types import SimpleNamespace

    directory = tmp_path / "updates" / "job"
    resources = Resources(max_download_bytes=16 * settings.MIB, spool_bytes=64 * settings.MIB, reserve_bytes=0)
    plan = estimate({"profile": "original"}, {}, resources)
    lease = resources.try_reserve(directory, "item", plan.peak())
    resources.plan(directory, "item", plan)
    sizes = []
    data = b"x" * (10 * settings.MIB + 7)
    response = Response(data=data, hook=lambda: sizes.append(resources.reservations[(directory, "item")]))
    result = fetch(directory, "item", URL, "original", {"post_id": 11},
                   SimpleNamespace(name="yandere", rate_root=None), resources, lambda: False,
                   ImageSessions("yandere", HTTP([response])), lambda **_: None)
    assert result["state"] == "downloaded"
    assert min(sizes) == 16 * settings.MIB + OVERHEAD
    assert max(sizes) == 24 * settings.MIB + OVERHEAD
    assert resources.reservations[(directory, "item")] == 2 * len(data) + OVERHEAD
    assert json.loads((directory / "item.download.json").read_text())["download_bytes"] == len(data)
    lease.release()


def test_header_space_wait_does_not_consume_attempts_or_stop_other_images(tmp_path):
    from urllib.parse import urlsplit

    lib, state = setup(tmp_path, "danbooru")
    configure(state, publish_items=32, publish_interval_seconds=60)
    large = png("red") + b"x" * (10 * settings.MIB)
    small = png("blue")
    records = [post("danbooru", 11, large), post("danbooru", 12, small)]
    bodies = {11: large, 12: small}
    for record in records:
        record.update(file_size=len(bodies[record["id"]]), file_url=f"https://example.invalid/{record['id']}.png")
    calls = []
    second = threading.Event()

    class Stream(ImageResponse):
        def iter_content(self, size):
            if self.data is large:
                assert second.wait(5)
            for start in range(0, len(self.data), size):
                yield self.data[start:start + size]

    class Transfers:
        def get(self, url, **kwargs):
            pid = int(urlsplit(url).path.removesuffix(".png").strip("/"))
            calls.append(pid)
            if pid == 12:
                second.set()
            return Stream(bodies[pid])

    task = job(state, lib, {"kind": "id_range", "start": 11, "end": 13}, "original")
    resources = Resources(max_download_bytes=16 * settings.MIB, spool_bytes=32 * settings.MIB, reserve_bytes=0)
    done = Runner(state, {"danbooru": FakeSite("danbooru", records)},
                  resources=resources, image_http=Transfers()).run(task["id"])
    assert done["state"] == "completed" and done["counts"] == {"stored": 2}, done
    assert calls.count(11) == 1 and calls.count(12) >= 2
    assert all(row["attempts"] == 1 for row in state.items(task["id"])["items"])
    assert not resources.reservations
