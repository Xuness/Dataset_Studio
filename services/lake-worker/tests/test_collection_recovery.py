import json

import pytest

from test_collections import Client, HTTP, key, setup
from studio_lake.collections import receipts
from studio_lake.collections.runner import Runner
from studio_lake.media_lake import library as archive_module
from studio_lake.media_lake.reader import Reader
from studio_lake.updates.resources import Resources
from studio_lake.updates.state import State
from studio_lake.util import IntegrityError


class Interrupted(BaseException):
    pass


@pytest.mark.parametrize("phase", ["collection_after_receipt", "collection_after_rename", "collection_after_commit",
                                  "collection_after_outbox_archive", "collection_after_control", "collection_after_published"])
def test_each_durable_boundary_recovers_without_repeating_capture(tmp_path, monkeypatch, phase):
    service, runner, job, _ = setup(tmp_path)
    hit = False

    def fail(name):
        nonlocal hit
        if hit or name != phase:
            return
        with service.state.db() as db:
            ready = db.execute("SELECT 1 FROM collection_outbox o JOIN collection_tasks t ON t.id=o.task_id WHERE t.kind='media_download' LIMIT 1").fetchone()
        if ready:
            hit = True
            raise Interrupted()

    monkeypatch.setattr(receipts, "failpoint", fail)
    monkeypatch.setattr(archive_module, "failpoint", fail)
    with pytest.raises(Interrupted):
        runner.run(job["id"], time_slice=60)
    assert hit
    with service.state.db() as db:
        count_before = db.execute("SELECT count(*) FROM collection_outbox WHERE job_id=?", (job["id"],)).fetchone()[0]
    restarted = Runner(State(service.state.root), resources=Resources(reserve_bytes=0), client_factory=Client, image_http=HTTP())
    result = restarted.run(job["id"], time_slice=60)
    assert result["state"] == "completed", result
    assert result["progress"]["media"]["downloaded"] == 2
    assert result["progress"]["media"]["archived"] == 2
    assert result["progress"]["media"]["published"] == 2
    with service.state.db() as db:
        rows = list(db.execute("SELECT state,control_applied,published FROM collection_outbox WHERE job_id=?", (job["id"],)))
        assert len(rows) >= count_before
        assert all(tuple(row) == ("released", 1, 1) for row in rows)
        assert db.execute("SELECT count(*) FROM collection_checkpoints WHERE job_id=?", (job["id"],)).fetchone()[0] == 1
    with Reader(tmp_path / "archive", tmp_path / "online") as reader:
        assert len(reader.objects()["items"]) == 1
    assert not (service.state.root / "collection-spool" / job["id"]).exists()


def test_pause_preserves_claim_and_cancel_replays_already_archived_result(tmp_path, monkeypatch):
    service, runner, job, _ = setup(tmp_path)
    hit = False

    def fail(name):
        nonlocal hit
        if name == "collection_after_commit" and not hit:
            hit = True
            raise Interrupted()

    monkeypatch.setattr(archive_module, "failpoint", fail)
    with pytest.raises(Interrupted):
        runner.run(job["id"], time_slice=60)
    latest = service.job(job["id"])
    paused = service.action(job["id"], dict(request_key=key(), expected_revision=latest["revision"], action="pause"))["job"]
    scratch = service.state.root / "collection-spool" / job["id"]
    files = sorted(str(p) for p in scratch.rglob("*"))
    assert runner.run(job["id"])["state"] == "paused"
    assert sorted(str(p) for p in scratch.rglob("*")) == files
    service.action(job["id"], dict(request_key=key(), expected_revision=paused["revision"], action="cancel"))
    assert runner.run(job["id"])["state"] == "cancelled"
    assert not scratch.exists()
    with service.state.db() as db:
        row = db.execute("SELECT counters_json FROM collection_jobs WHERE id=?", (job["id"],)).fetchone()
        assert json.loads(row[0])["cleanup"] == "complete"
        assert db.execute("SELECT count(*) FROM collection_applied_batches").fetchone()[0] == 1


def test_new_acceptance_rejects_changed_epoch(tmp_path, monkeypatch):
    service, runner, job, _ = setup(tmp_path)

    def fail(name):
        if name == "collection_after_receipt":
            raise Interrupted()

    monkeypatch.setattr(receipts, "failpoint", fail)
    with pytest.raises(Interrupted):
        runner.run(job["id"])
    with service.state.db() as db:
        row = dict(db.execute("SELECT * FROM collection_outbox").fetchone())
        db.execute("UPDATE collection_jobs SET execution_epoch=execution_epoch+1 WHERE id=?", (job["id"],))
    lib = service.state.library(job["library_id"])
    with pytest.raises(IntegrityError, match="Stale"):
        receipts.accept(service, lib, row)
    assert lib.commits() == []
