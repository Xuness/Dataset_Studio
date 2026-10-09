import hashlib
import uuid

import pytest

from pinterest_fixtures import Fixture
from studio_lake.library import Library, read_object
from studio_lake.online_storage import connect
from studio_lake.pinterest.lake.library import PinterestLibrary
from studio_lake.pinterest.lake.online import rebuild
from studio_lake.updates.sites import UpdateError


def test_specified_pins_share_acquisition_but_keep_all_media_and_rebuild(tmp_path):
    fixture = Fixture(tmp_path, ("858146903966145189", "1089097122423035272"))
    assert fixture.service.preview(dict(definition=fixture.spec))["network_requests"] == 0
    assert not fixture.pin_calls and not fixture.media_calls
    result = fixture.run()
    assert result["state"] == "completed", result
    assert len(fixture.pin_calls) == 2 and len(fixture.media_calls) == 1
    assert result["archive_seq"] == result["served_seq"] == 5
    assert result["download_bytes"] == len(fixture.data)
    lib = fixture.service.library(fixture.lake["library_id"])
    assert isinstance(Library(lib.config), PinterestLibrary)
    assert lib.verify(deep=True) == dict(batches=5, captures=2, objects=1)
    db = connect(lib.cache / "online.sqlite")
    try:
        assert db.execute("SELECT count(*) FROM assets").fetchone()[0] == 2
        assert db.execute("SELECT count(*) FROM acquisitions").fetchone()[0] == 1
        sha, pack, offset, length = db.execute("SELECT sha256,pack_path,offset,length FROM objects").fetchone()
        assert read_object(lib.root, pack, offset, length) == fixture.data
        assert sha == hashlib.sha256(fixture.data).hexdigest()
        assert db.execute("SELECT download_md5 FROM acquisitions").fetchone()[0] == hashlib.md5(fixture.data).hexdigest()
        assert set(r[0] for r in db.execute("SELECT pin_id FROM pins")) == {"858146903966145189", "1089097122423035272"}
    finally:
        db.close()
    rebuilt = rebuild(PinterestLibrary.archive(lib.root), tmp_path / "rebuilt")
    assert rebuilt["served_seq"] == 5
    db = connect(tmp_path / "rebuilt" / "online.sqlite")
    try:
        assert db.execute("SELECT count(*) FROM assets").fetchone()[0] == 2
    finally:
        db.close()


@pytest.mark.parametrize("case", ["html", "truncated", "dimensions", "etag", "story_video"])
def test_invalid_original_cannot_publish_an_object(tmp_path, case):
    args = {}
    if case == "html":
        args["data"] = b"<html>not an image</html>"
    elif case == "truncated":
        from pinterest_fixtures import png
        args["data"] = png()[:30]
    elif case == "dimensions":
        args["pin_transform"] = lambda p: p["images"]["orig"].update(width=100)
    elif case == "etag":
        args["etag"] = '"' + "0" * 32 + '"'
    else:
        args["pin_transform"] = lambda p: p.update(story_pin_data=dict(page_count=1, pages=[dict(blocks=[dict(block_type=3, video={"x": 1})])]))
    fixture = Fixture(tmp_path, **args)
    assert fixture.run()["state"] == "completed_with_gaps"
    db = connect(tmp_path / "index" / "online.sqlite")
    try:
        assert db.execute("SELECT count(*) FROM objects").fetchone()[0] == 0
        assert db.execute("SELECT count(*) FROM captures").fetchone()[0] == 1
    finally:
        db.close()


def test_idempotency_revision_and_budget_are_explicit(tmp_path):
    fixture = Fixture(tmp_path, ("123", "124"), budget=dict(api_requests=1, detail_requests=1))
    args = dict(request_key=str(uuid.uuid4()), definition=fixture.spec)
    first = fixture.service.create(args)
    assert first["id"] == fixture.job["id"] == fixture.service.create(args)["id"]
    with pytest.raises(UpdateError, match="another Pinterest command"):
        fixture.service.create({**args, "definition": {**fixture.spec, "seeds": [dict(kind="pin", id="125")]}})
    assert fixture.run()["state"] == "waiting_budget"
    assert len(fixture.pin_calls) == 1
    job = fixture.service.job(fixture.job["id"])
    paused = fixture.service.action(dict(job_id=job["id"], action="pause", expected_revision=job["revision"]))
    assert paused["desired_state"] == "paused"
    with pytest.raises(UpdateError, match="changed"):
        fixture.service.action(dict(job_id=job["id"], action="cancel", expected_revision=job["revision"]))
    assert fixture.run()["state"] == "paused"


def test_same_url_still_checks_each_pins_declared_dimensions(tmp_path):
    def transform(pin):
        if pin["id"] == "124":
            pin["images"]["orig"]["width"] = 12
    fixture = Fixture(tmp_path, ("123", "124"), pin_transform=transform)
    result = fixture.run()
    assert result["state"] == "completed_with_gaps"
    assert len(fixture.media_calls) == 1
    reasons = [v["reason"] for v in fixture.service.page(dict(job_id=result["id"]), "items")["items"]]
    assert "shared_url_dimensions_mismatch" in reasons


@pytest.mark.parametrize("point", ["pinterest_after_response", "pinterest_after_prepared", "pinterest_after_seal",
    "pinterest_after_rename", "pinterest_after_commit", "pinterest_after_publish", "pinterest_after_control_replay"])
@pytest.mark.parametrize("stage", ["pin", "media"])
def test_recovery_replays_saved_evidence_without_refetch_or_duplicate_publication(tmp_path, monkeypatch, point, stage):
    from studio_lake import archive_io
    from studio_lake.pinterest import runner
    from studio_lake.pinterest.lake import library, online

    fixture = Fixture(tmp_path)
    class Crash(BaseException):
        pass
    fired = []
    def fault(name):
        ready = fixture.pin_calls if stage == "pin" or point == "pinterest_after_response" else fixture.media_calls
        if name == point and ready and not fired:
            fired.append(name)
            raise Crash()
    for module in (runner, archive_io, library, online):
        monkeypatch.setattr(module, "failpoint", fault)
    with pytest.raises(Crash):
        fixture.run()
    result = fixture.run()
    assert result["state"] == "completed", result
    assert len(fixture.pin_calls) == 1 and len(fixture.media_calls) == 1
    assert result["archive_seq"] == result["served_seq"] == 3
    assert fixture.service.library(fixture.lake["library_id"]).verify(deep=True)["objects"] == 1


def test_new_definition_rejects_unsupported_source_semantics(tmp_path):
    fixture = Fixture(tmp_path)
    for change in (dict(collector="pixiv_web_v1"), dict(seeds=[dict(kind="board", id="123")]),
                   dict(access=dict(mode="authenticated")), dict(discovery=dict(entrypoints=["related_pins"], max_depth=1))):
        with pytest.raises(UpdateError):
            fixture.service.preview(dict(definition={**fixture.spec, **change}))
    assert not fixture.pin_calls


def test_pause_preserves_prepared_claim_and_cancel_publishes_accepted_capture(tmp_path, monkeypatch):
    from studio_lake import archive_io
    from studio_lake.pinterest import runner

    fixture = Fixture(tmp_path / "pause")
    fired = []
    def pause(name):
        if name == "pinterest_after_response" and not fired:
            fired.append(name)
            row = fixture.service.job(fixture.job["id"])
            fixture.service.action(dict(job_id=row["id"], action="pause", expected_revision=row["revision"]))
    monkeypatch.setattr(runner, "failpoint", pause)
    result = fixture.run()
    assert result["state"] == "paused"
    fixture.service.action(dict(job_id=result["id"], action="resume", expected_revision=result["revision"]))
    assert fixture.run()["state"] == "completed"
    assert len(fixture.pin_calls) == 1

    fixture = Fixture(tmp_path / "cancel")
    class Crash(BaseException):
        pass
    def cancel(name):
        if name == "pinterest_after_commit" and fixture.pin_calls:
            row = fixture.service.job(fixture.job["id"])
            fixture.service.action(dict(job_id=row["id"], action="cancel", expected_revision=row["revision"]))
            raise Crash()
    monkeypatch.setattr(archive_io, "failpoint", cancel)
    with pytest.raises(Crash):
        fixture.run()
    result = fixture.run()
    assert result["state"] == "cancelled" and result["archive_seq"] == result["served_seq"] == 2
    assert not fixture.media_calls
    assert fixture.service.library(fixture.lake["library_id"]).verify(deep=True)["captures"] == 1
    assert all(v["state"] != "running" for v in fixture.service.page(dict(job_id=result["id"]), "items")["items"])


def test_equal_signatures_do_not_replace_url_identity_or_byte_dedup(tmp_path):
    def transform(pin):
        pin["images"]["orig"]["url"] += "?pin=" + pin["id"]
    fixture = Fixture(tmp_path, ("123", "124"), pin_transform=transform)
    assert fixture.run()["state"] == "completed"
    assert len(fixture.media_calls) == 2
    assert fixture.service.library(fixture.lake["library_id"]).verify(deep=True)["objects"] == 1


def test_control_upgrade_backs_up_v14_and_preserves_older_rows(tmp_path):
    import sqlite3
    from studio_lake.updates.state import State
    from update_fixtures import remove_pinterest_schema

    state = State(tmp_path / "control")
    with state.db() as db:
        remove_pinterest_schema(db)
        db.execute("INSERT INTO settings VALUES('old-fixture','unchanged')")
        db.execute("INSERT INTO schedules VALUES('old-schedule','{}',60,1000,0,7,NULL)")
        db.execute("PRAGMA user_version=14")
    upgraded = State(state.root)
    with upgraded.db() as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == 15
        assert db.execute("SELECT value FROM settings WHERE key='old-fixture'").fetchone()[0] == "unchanged"
        assert db.execute("SELECT revision,next_at FROM schedules WHERE id='old-schedule'").fetchone() == (7, 1000)
    backups = list((state.root / "backups").glob("control-v14-*.sqlite"))
    assert len(backups) == 1
    with sqlite3.connect(backups[0]) as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == 14
        assert not db.execute("SELECT 1 FROM sqlite_master WHERE name='pinterest_jobs'").fetchone()
