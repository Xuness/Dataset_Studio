"""Production review regressions: real transport boundaries, recovery and shared spool."""

from contextlib import contextmanager
import json

import pytest

from pinterest_fixtures import Fixture, MediaResponse
from studio_lake.pinterest import http, runner
from studio_lake.pinterest.lake import library
from studio_lake.updates.resources import Resources
from studio_lake.updates.sites import UpdateError
from test_pinterest_manifest import pin


class Clock:
    now = 0

    def monotonic(self):
        return self.now


def real_client(fixture, monkeypatch, *, delay=16, interrupt=False):
    clock = Clock()
    monkeypatch.setattr(runner.time, "monotonic", clock.monotonic)

    class Response(MediaResponse):
        def iter_content(self, size):
            clock.now += delay
            if interrupt:
                raise UpdateError("CANCELLED", "Interrupted after send")
            yield self.data

    class Session:
        def get(self, url, **kwargs):
            identity = json.loads(kwargs["params"]["data"])["options"]["id"]
            fixture.pin_calls.append(identity)
            return Response(json.dumps(dict(resource_response=dict(data=pin(identity)))).encode())

        def close(self):
            pass

    def factory(root, context, *, cancelled):
        return http.Client(root, context, cancelled=cancelled, session=Session())

    fixture.factory = factory
    return clock


def test_slow_first_chunk_finishes_after_slice_and_does_not_refetch(tmp_path, monkeypatch):
    fixture = Fixture(tmp_path, ("123", "124"), budget=dict(api_requests=1))
    real_client(fixture, monkeypatch)
    run = fixture.runner()
    run.run(fixture.job["id"])
    result = run.run(fixture.job["id"])
    assert fixture.pin_calls == ["123"]
    assert result["api_requests"] == 1 and result["state"] == "waiting_budget"
    assert len(fixture.media_calls) == 1
    assert fixture.service.library(fixture.lake["library_id"]).verify()["captures"] == 1


def test_resending_a_running_request_cannot_bypass_budget(tmp_path, monkeypatch):
    fixture = Fixture(tmp_path, budget=dict(api_requests=1))
    real_client(fixture, monkeypatch, interrupt=True)
    for _ in range(4):
        result = fixture.runner().run(fixture.job["id"])
    assert len(fixture.pin_calls) == result["api_requests"] == 1
    assert result["state"] == "waiting_budget"


def test_running_resends_have_a_finite_attempt_limit(tmp_path, monkeypatch):
    fixture = Fixture(tmp_path)
    real_client(fixture, monkeypatch, interrupt=True)
    for _ in range(6):
        result = fixture.runner().run(fixture.job["id"])
    assert len(fixture.pin_calls) == result["api_requests"] == 3
    assert result["state"] == "completed_with_gaps"


def test_waiting_in_the_rate_lane_does_not_spend_requests_or_attempts(tmp_path, monkeypatch):
    fixture = Fixture(tmp_path, budget=dict(api_requests=1))
    clock = real_client(fixture, monkeypatch)
    factory = fixture.factory

    def waiting_factory(*args, **kwargs):
        client = factory(*args, **kwargs)
        client.injected = False
        return client

    fixture.factory = waiting_factory

    @contextmanager
    def lane(root, site, cancelled, **kwargs):
        clock.now += 16
        assert cancelled()
        raise UpdateError("CANCELLED", "No rate admission yet")
        yield

    monkeypatch.setattr(http.rate, "admission", lane)
    for _ in range(4):
        result = fixture.runner().run(fixture.job["id"])
    assert not fixture.pin_calls and result["api_requests"] == 0
    with fixture.state.db() as db:
        assert db.execute("SELECT max(attempts) FROM pinterest_tasks WHERE kind='pin_detail'").fetchone()[0] == 0


@pytest.mark.parametrize("limit", ["pixels", "bytes", "memory", "decoder_pixels"])
def test_permanent_media_limit_is_a_gap_and_later_pins_can_finish(tmp_path, monkeypatch, limit):
    from PIL import Image
    import io

    buffer = io.BytesIO()
    Image.new("RGB", (1000, 1000), "red").save(buffer, "PNG", compress_level=0)
    large = buffer.getvalue()
    if limit == "decoder_pixels":
        # The shared image executor changes Pillow's process-wide default; our own guard still wins.
        monkeypatch.setattr(Image, "MAX_IMAGE_PIXELS", 1_000_000_000)
        import struct
        import zlib
        header = struct.pack(">II", 25000, 25000) + large[24:29]
        large = large[:16] + header + struct.pack(">I", zlib.crc32(b"IHDR" + header)) + large[33:]
    fixture = Fixture(tmp_path, ("123", "124"), pin_transform=lambda p: p["images"]["orig"].update(
        width=1000 if p["id"] == "123" else 8, height=1000 if p["id"] == "123" else 6,
        url=p["images"]["orig"]["url"] + "?pin=" + p["id"]))
    get = fixture.get

    def images(url, **kwargs):
        if "pin=123" in url:
            fixture.media_calls.append((url, kwargs))
            return MediaResponse(large)
        return get(url, **kwargs)

    fixture.get = images
    resources = Resources(reserve_bytes=0)
    if limit == "pixels":
        resources.config["max_image_pixels"] = 500_000
    elif limit == "bytes":
        resources.overrides["max_download_bytes"] = 2 * 1024**2
    elif limit == "memory":
        resources.config["decode_memory_mib"] = 8
    run = runner.Runner(fixture.state, resources=resources, client_factory=fixture.factory, image_http=fixture)
    result = run.run(fixture.job["id"], time_slice=60)
    assert result["state"] == "completed_with_gaps"
    assert fixture.pin_calls == ["123", "124"]
    assert fixture.service.library(fixture.lake["library_id"]).verify(deep=True)["objects"] == 1
    assert any(v["reason"] == "UPDATE_RESOURCE_LIMIT" for v in fixture.service.page(dict(job_id=result["id"]), "items")["items"])
    assert not list((tmp_path / "index/pinterest_downloads").rglob("*.downloaded"))


def test_rejected_media_does_not_accumulate_until_spool_deadlock(tmp_path):
    fixture = Fixture(tmp_path, tuple(str(100 + n) for n in range(8)), data=b"x" * (8 * 1024**2),
        pin_transform=lambda p: p["images"]["orig"].update(url=p["images"]["orig"]["url"] + "?pin=" + p["id"]))
    resources = Resources(max_download_bytes=16 * 1024**2, spool_bytes=64 * 1024**2, reserve_bytes=0)
    run = runner.Runner(fixture.state, resources=resources, client_factory=fixture.factory, image_http=fixture)
    result = run.run(fixture.job["id"], time_slice=60)
    assert result["state"] == "completed_with_gaps" and len(fixture.media_calls) == 8
    assert not list((tmp_path / "index/pinterest_downloads").rglob("*.downloaded"))
    assert resources.snapshot()["staging_bytes"] < 1024**2
    fixture.service.action(dict(job_id=result["id"], action="retry", expected_revision=result["revision"]))
    assert run.run(fixture.job["id"], time_slice=60)["state"] == "completed_with_gaps"
    assert len(fixture.media_calls) == 16 and resources.snapshot()["staging_bytes"] < 1024**2


def test_spool_reservation_covers_tar_and_other_lakes_cannot_overcommit(tmp_path, monkeypatch):
    fixture = Fixture(tmp_path)
    resources = Resources(max_download_bytes=16 * 1024**2, spool_bytes=64 * 1024**2, reserve_bytes=0)
    original = library.Batch.seal
    checked = []

    def seal(batch):
        if batch.tar:
            batch.pack_writer.flush()
            snapshot = resources.snapshot()
            assert resources.reservations, "Download lease must survive into sealing"
            actual_pack = batch.pack_staging_path.stat().st_size
            assert actual_pack > 0
            assert snapshot["staging_bytes"] >= actual_pack
            other = tmp_path / "another-lake/downloads/job"
            lease = resources.try_reserve(other, "other", resources.spool_bytes - snapshot["staging_reserved_bytes"])
            assert lease is not None
            try:
                result = original(batch)
                assert resources.snapshot()["staging_reserved_bytes"] <= resources.spool_bytes
                checked.append(True)
                return result
            finally:
                lease.release()
        return original(batch)

    monkeypatch.setattr(library.Batch, "seal", seal)
    run = runner.Runner(fixture.state, resources=resources, client_factory=fixture.factory, image_http=fixture)
    assert run.run(fixture.job["id"], time_slice=60)["state"] == "completed"
    assert checked and not resources.reservations


def test_explicit_retry_reclaims_old_failure_generation_before_admission(tmp_path):
    from studio_lake.pinterest.downloader import download_key
    fixture = Fixture(tmp_path, data=b"invalid")
    result = fixture.run()
    with fixture.state.db() as db:
        task = dict(db.execute("SELECT * FROM pinterest_tasks WHERE job_id=? AND kind='media_download'", (result["id"],)).fetchone())
    directory = tmp_path / "index/pinterest_downloads" / result["id"]
    directory.mkdir(parents=True, exist_ok=True)
    key = download_key(result["id"], task, json.loads(task["input_json"]))
    # An older worker left this rejected generation behind. New download admission
    # would deadlock unless the retry's durable cleanup runs first.
    old = directory / (key + ".downloaded")
    old.write_bytes(b"x" * (60 * 1024**2))
    fixture.service.action(dict(job_id=result["id"], action="retry", expected_revision=result["revision"]))
    resources = Resources(max_download_bytes=16 * 1024**2, spool_bytes=64 * 1024**2, reserve_bytes=0)
    run = runner.Runner(fixture.state, resources=resources, client_factory=fixture.factory, image_http=fixture)
    assert run.run(result["id"], time_slice=60)["state"] == "completed_with_gaps"
    assert not old.exists() and len(fixture.media_calls) == 2


def test_slow_media_without_validator_finishes_once_after_slice(tmp_path, monkeypatch):
    fixture = Fixture(tmp_path)
    clock = real_client(fixture, monkeypatch, delay=0)
    original = fixture.get
    def get(url, **kwargs):
        response = original(url, **kwargs)
        response.headers.pop("ETag")
        def chunks(size):
            clock.now += 16
            yield fixture.data
        response.iter_content = chunks
        return response
    fixture.get = get
    run = fixture.runner()
    run.run(fixture.job["id"])
    assert run.run(fixture.job["id"])["state"] == "completed"
    assert len(fixture.media_calls) == 1


def test_wall_budget_is_checked_when_a_running_request_resends(tmp_path, monkeypatch):
    fixture = Fixture(tmp_path, budget=dict(wall_seconds=10))
    real_client(fixture, monkeypatch, interrupt=True)
    run = fixture.runner()
    run.run(fixture.job["id"])
    result = run.run(fixture.job["id"])
    assert result["state"] == "waiting_budget" and len(fixture.pin_calls) == 1
    assert result["error_code"] == "PINTEREST_RUN_BUDGET"


def test_recovered_download_and_tar_are_one_atomic_reservation(tmp_path):
    from studio_lake.pinterest.staging import Spool
    fixture = Fixture(tmp_path)
    lib = fixture.service.library(fixture.lake["library_id"])
    raw = lib.cache / "pinterest_downloads" / fixture.job["id"] / ("a" * 64 + ".downloaded")
    tar = lib.cache / "pack_staging" / (fixture.job["id"] + ".tar.tmp")
    for path in (raw, tar):
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("wb") as file:
            file.truncate(20 * 1024**2)
    resources = Resources(max_download_bytes=32 * 1024**2, spool_bytes=64 * 1024**2, reserve_bytes=0)
    other = resources.try_reserve(tmp_path / "other/job", "other", 20 * 1024**2)
    assert other is not None
    try:
        with Spool(lib, fixture.job["id"], resources) as spool:
            spool.reserve("a" * 64, fixture.job["id"])
            value = resources.snapshot()
            assert value["staging_bytes"] == 40 * 1024**2
            assert 60 * 1024**2 <= value["staging_reserved_bytes"] < 64 * 1024**2
        assert resources.snapshot()["staging_bytes"] == 40 * 1024**2
    finally:
        other.release()


def test_crash_after_acceptance_replays_cleanup_before_new_space_admission(tmp_path, monkeypatch):
    from studio_lake import archive_io
    fixture = Fixture(tmp_path, ("123", "124"), data=b"x" * (8 * 1024**2),
        pin_transform=lambda p: p["images"]["orig"].update(url=p["images"]["orig"]["url"] + "?pin=" + p["id"]))
    resources = Resources(max_download_bytes=16 * 1024**2, spool_bytes=64 * 1024**2, reserve_bytes=0)
    run = runner.Runner(fixture.state, resources=resources, client_factory=fixture.factory, image_http=fixture)
    class Crash(BaseException):
        pass
    def crash(name):
        if name == "pinterest_after_commit" and fixture.media_calls:
            raise Crash()
    monkeypatch.setattr(archive_io, "failpoint", crash)
    with pytest.raises(Crash):
        run.run(fixture.job["id"], time_slice=60)
    assert resources.snapshot()["staging_bytes"] >= 8 * 1024**2
    snapshot = resources.snapshot()
    assert resources.try_reserve(tmp_path / "other/job", "too-large", 46 * 1024**2) is None
    other = resources.try_reserve(tmp_path / "other/job", "other", resources.spool_bytes - snapshot["staging_reserved_bytes"] - 1024**2)
    assert other is not None
    monkeypatch.setattr(archive_io, "failpoint", lambda _: None)
    try:
        assert run.run(fixture.job["id"], time_slice=60)["state"] == "completed_with_gaps"
        assert len(fixture.media_calls) == 2 and resources.snapshot()["staging_bytes"] < 1024**2
    finally:
        other.release()
