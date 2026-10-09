from datetime import datetime, timedelta, timezone
import io
import json
import time
import uuid

from PIL import Image
import pytest

from pinterest_fixtures import Fixture, MediaResponse
from studio_lake.archive_rebuild import build_archive, verify_archive, compare_reference
from studio_lake.canonical import canonical, utc
from studio_lake.online_storage import connect
from studio_lake.pinterest.schedules import Schedules
from studio_lake.updates.sites import UpdateError


def again(fixture, *, mode="revalidate", hours=24, language="zh-TW"):
    spec = fixture.service.job(fixture.job["id"])["definition"]
    spec["media"]["reuse"] = dict(mode=mode, max_age_hours=hours)
    spec["access"]["language"] = language
    fixture.job = fixture.service.create(dict(request_key=str(uuid.uuid4()), definition=spec))
    return fixture.run()


def test_revalidation_304_keeps_fresh_observation_and_explicit_reuse_evidence(tmp_path):
    fixture = Fixture(tmp_path)
    assert fixture.run()["state"] == "completed"
    fixture.pin_transform = lambda p: p.update(title="changed statistics", repin_count=99)
    def get(url, **kwargs):
        fixture.media_calls.append((url, kwargs))
        assert "If-None-Match" in kwargs["headers"]
        response = MediaResponse(b"")
        response.status_code, response.headers = 304, {"ETag": kwargs["headers"]["If-None-Match"]}
        return response
    fixture.get = get
    result = again(fixture)
    assert result["state"] == "completed" and result["download_bytes"] == 0
    assert result["metrics"]["http_validated"] == 1 and len(fixture.media_calls) == 2
    db = connect(tmp_path / "index/online.sqlite")
    try:
        assert db.execute("SELECT count(*) FROM objects").fetchone()[0] == 1
        assert db.execute("SELECT count(*) FROM pin_observations").fetchone()[0] == 2
        assert db.execute("SELECT count(DISTINCT content_revision) FROM media_manifests").fetchone()[0] == 1
        assert db.execute("SELECT count(DISTINCT media_id) FROM assets").fetchone()[0] == 2
        evidence = db.execute("SELECT details_json FROM acquisitions WHERE evidence='http_validated'").fetchone()[0]
        assert json.loads(evidence)["previous_acquisition_id"]
    finally:
        db.close()


def test_conditional_200_is_consumed_once_and_preserves_changed_bytes(tmp_path):
    fixture = Fixture(tmp_path)
    fixture.run()
    output = io.BytesIO()
    Image.new("RGB", (8, 6), "blue").save(output, format="PNG")
    fixture.data = output.getvalue()
    result = again(fixture)
    assert result["state"] == "completed" and len(fixture.media_calls) == 2
    assert "If-None-Match" in fixture.media_calls[-1][1]["headers"]
    assert result["metrics"]["new_byte_objects"] == 1
    assert fixture.service.library(fixture.lake["library_id"]).verify(deep=True)["objects"] == 2


def test_historical_reuse_does_not_refresh_age_or_cross_access_scope(tmp_path):
    fixture = Fixture(tmp_path)
    fixture.run()
    with fixture.state.db() as db:
        old = json.loads(db.execute("SELECT value_json FROM pinterest_reuse").fetchone()[0])["source_checked_at"]
    assert again(fixture, mode="historical")["state"] == "completed"
    assert len(fixture.media_calls) == 1
    with fixture.state.db() as db:
        saved = json.loads(db.execute("SELECT value_json FROM pinterest_reuse").fetchone()[0])
        assert saved["source_checked_at"] == old and saved["evidence"] == "historical_reuse"
        saved["source_checked_at"] = utc((datetime.now(timezone.utc) - timedelta(hours=48)).isoformat())
        db.execute("UPDATE pinterest_reuse SET value_json=?", (canonical(saved),))
    assert again(fixture, mode="historical")["state"] == "completed"
    assert len(fixture.media_calls) == 2
    assert again(fixture, mode="historical", language="en-US")["state"] == "completed"
    assert len(fixture.media_calls) == 3


def test_invalid_304_cannot_bind_an_asset(tmp_path):
    fixture = Fixture(tmp_path)
    fixture.run()
    def get(*args, **kwargs):
        response = MediaResponse(b"", etag='"changed"')
        response.status_code = 304
        return response
    fixture.get = get
    result = again(fixture)
    assert result["state"] == "completed_with_gaps"
    assert any(v["reason"] == "cdn_etag_changed_on_304" for v in fixture.service.page(dict(job_id=result["id"]), "items")["items"])


def test_published_staging_is_reclaimed_before_a_budget_limited_job_finishes(tmp_path):
    fixture = Fixture(tmp_path, ("123", "124"), budget=dict(api_requests=1, detail_requests=1))
    result = fixture.run()
    assert result["state"] == "waiting_budget"
    for name in ("pinterest_receipts", "pinterest_downloads"):
        path = tmp_path / "index" / name / result["id"]
        assert not list(path.glob("*"))
    fixture.service.action(dict(job_id=result["id"], expected_revision=result["revision"], action="continue"))
    assert fixture.run()["state"] == "completed"
    assert len(fixture.media_calls) == 1


def schedule_args(fixture, at):
    return dict(id=str(uuid.uuid4()), request_key=str(uuid.uuid4()), expected_revision=0,
        definition=fixture.spec, every_seconds=60, first_run_at=datetime.fromtimestamp(at, timezone.utc).isoformat(), enabled=True)


def test_schedules_coalesce_downtime_and_block_all_unfinished_lake_work(tmp_path):
    fixture = Fixture(tmp_path)
    schedules = Schedules(fixture.service)
    at = time.time()
    args = schedule_args(fixture, at - 600)
    saved = schedules.save(args)
    assert schedules.save(args) == saved
    other = schedules.save(schedule_args(fixture, at - 600))
    schedules.tick(at)
    assert all(v["last_job"] is None for v in schedules.list({})["items"])
    fixture.run()
    schedules.tick(at)
    schedules.tick(at)
    rows = schedules.list({})["items"]
    launched = [v for v in rows if v["last_job"]]
    assert len(launched) == 1
    identity = launched[0]["last_job"]
    job = fixture.service.job(identity)
    assert datetime.fromisoformat(launched[0]["next_run_at"]).timestamp() > at
    fixture.service.action(dict(job_id=identity, action="pause", expected_revision=job["revision"]))
    schedules.tick(at + 86400)
    assert len([v for v in schedules.list({})["items"] if v["last_job"]]) == 1
    with pytest.raises(UpdateError, match="changed"):
        schedules.remove(dict(id=other["id"], request_key=str(uuid.uuid4()), expected_revision=other["revision"] + 1))


def test_archive_preparation_resumes_and_compares_a_fixed_published_prefix(tmp_path):
    fixture = Fixture(tmp_path)
    fixture.run()
    output = tmp_path / "prepare"
    class Crash(BaseException):
        pass
    def stop(stage, seq):
        if stage == "published" and seq == 2:
            raise Crash()
    with pytest.raises(Crash):
        build_archive(tmp_path / "media", output, "pinterest", reference_index=tmp_path / "index", stop=stop)
    assert not (output / "ONLINE.json").exists()
    plan = build_archive(tmp_path / "media", output, "pinterest", reference_index=tmp_path / "index")
    assert plan["state"] == "built"
    again(fixture, mode="historical")
    verified = verify_archive(output)
    assert verified["raw_roundtrip_verified"]
    assert compare_reference(output)["equal"]
    db = connect(tmp_path / "index/online.sqlite")
    try:
        assert db.execute("SELECT count(*) FROM leases WHERE purpose='archive-validation'").fetchone()[0] == 0
    finally:
        db.close()
