"""Regression cases for the October 2026 Pixiv persistence review."""

import copy
import errno
from dataclasses import replace
from datetime import datetime, timezone
import json
import shutil

import pytest

from test_collections import Client, HTTP, key, setup
from test_collection_media_accounts import Session, cookie
from test_collection_continuous import schedule
from test_collection_recovery import Interrupted
from studio_lake.collectors.pixiv import normalize
from studio_lake.collections import batches, receipts, tasks
from studio_lake.collections.incremental import reusable_work
from studio_lake.collections.schedules import Schedules
from studio_lake.media_lake import library as archive_module
from studio_lake.media_lake.library import MediaLibrary
from studio_lake.media_lake.online import Publisher, initialize
from studio_lake.media_lake.reader import Reader
from studio_lake.media_lake.schema import canonical
from studio_lake.updates.sites import UpdateError
from studio_lake.util import IntegrityError, atomic_json, digest, file_hash, read_json


class EmptyAnimation(Client):
    def request(self, kind, payload):
        response = super().request(kind, payload)
        body = json.loads(response.body)["body"]
        if kind == "work_detail":
            body.update(illustType=2, pageCount=1)
        elif kind == "media_manifest":
            body = dict(originalSrc="https://i.pximg.net/fixture.zip", frames=[])
        return replace(response, body=json.dumps(dict(error=False, body=body)).encode())


def action(service, job_id, name):
    return service.action(job_id, dict(request_key=key(), expected_revision=service.job(job_id)["revision"], action=name))["job"]


def test_empty_animation_archives_incomplete_and_can_cancel(tmp_path):
    service, runner, job, _ = setup(tmp_path)
    runner.client_factory = EmptyAnimation
    result = runner.run(job["id"], time_slice=60)
    assert result["state"] == "completed_with_gaps", result
    with Reader(tmp_path / "archive", tmp_path / "online") as reader:
        assert list(reader.db.execute("SELECT complete,reason,item_count FROM media_manifests")) == [(0, "empty_frames", 1)]
        assert reader.db.execute("SELECT count(*) FROM animation_frames").fetchone()[0] == 0
        assert reader.db.execute("SELECT count(*) FROM captures").fetchone()[0] == 4
    lib = service.state.library(job["library_id"])
    archived = MediaLibrary.archive(lib.root)
    rebuilt = tmp_path / "rebuilt"
    pointer = initialize(archived.info, rebuilt)
    assert Publisher(archived, index=rebuilt, pointer=pointer).sync()["served_seq"] == result["progress"]["publication"]["archive_seq"]
    action(service, job["id"], "cancel")
    assert runner.run(job["id"])["state"] == "cancelled"
    assert not (service.state.root / "collection-spool" / job["id"]).exists()


@pytest.mark.parametrize("invalid", ["title", "duplicate_frames"])
def test_invalid_normalized_shape_falls_back_to_raw_before_prepared(tmp_path, invalid):
    service, runner, job, _ = setup(tmp_path)

    class Changed(EmptyAnimation if invalid == "duplicate_frames" else Client):
        def request(self, kind, payload):
            response = super().request(kind, payload)
            body = json.loads(response.body)["body"]
            if invalid == "title" and kind == "work_detail":
                body["title"] = ["not a scalar"]
            elif invalid == "duplicate_frames" and kind == "media_manifest":
                body["frames"] = [dict(file="same.png", delay=20)] * 2
            return replace(response, body=json.dumps(dict(error=False, body=body)).encode())

    runner.client_factory = Changed
    result = runner.run(job["id"], time_slice=60)
    assert result["state"] == "completed_with_gaps", result
    failed = service.tasks(job["id"], dict(state="needs_review"))["items"]
    assert len(failed) == 1 and failed[0]["reason"] == "COLLECTION_NORMALIZATION_FAILED"
    with Reader(tmp_path / "archive", tmp_path / "online") as reader:
        raw = reader.raw(failed[0]["summary"]["capture_id"])
        assert raw["status"] == "available"
    with service.state.db() as db:
        assert db.execute("SELECT count(*) FROM collection_outbox WHERE state='prepared'").fetchone()[0] == 0


def interrupted_detail(service, runner, job, monkeypatch):
    def fail(phase):
        if phase == "collection_after_receipt":
            with service.state.db() as db:
                row = db.execute("SELECT 1 FROM collection_outbox o JOIN collection_tasks t ON t.id=o.task_id WHERE t.kind='work_detail' AND o.state='prepared'").fetchone()
            if row:
                raise Interrupted()
    monkeypatch.setattr(receipts, "failpoint", fail)
    with pytest.raises(Interrupted):
        runner.run(job["id"], time_slice=60)
    monkeypatch.setattr(receipts, "failpoint", lambda _: None)
    with service.state.db() as db:
        return dict(db.execute("SELECT o.* FROM collection_outbox o JOIN collection_tasks t ON t.id=o.task_id WHERE t.kind='work_detail' AND o.state='prepared'").fetchone())


@pytest.mark.parametrize("cancel,changed_hash", [(False, False), (True, False), (True, True)])
def test_bad_prepared_receipt_is_isolated_without_rewriting_evidence(tmp_path, monkeypatch, cancel, changed_hash):
    service, runner, job, _ = setup(tmp_path)
    row = interrupted_detail(service, runner, job, monkeypatch)
    path = service.state.root / row["intent_path"]
    value = read_json(path)
    value["records"]["work_observations"][0]["title"] = ["legacy invalid normalized field"]
    atomic_json(path, value)
    original_bytes = path.read_bytes()
    if not changed_hash:
        with service.state.db() as db:
            db.execute("UPDATE collection_outbox SET content_sha256=? WHERE id=?", (file_hash(path), row["id"]))
    if cancel:
        action(service, job["id"], "cancel")
    result = runner.run(job["id"], time_slice=60)
    assert result["state"] == ("cancelled" if cancel else "completed_with_gaps"), result
    preserved = service.state.root / "collection-quarantine" / job["id"] / row["id"] / "result.json"
    assert preserved.read_bytes() == original_bytes
    with service.state.db() as db:
        quarantined = db.execute("SELECT state,control_applied,published FROM collection_outbox WHERE id=?", (row["id"],)).fetchone()
        assert tuple(quarantined) == ("quarantined", 0, 0)
    assert not (service.state.root / "collection-spool" / job["id"]).exists()
    if not cancel:
        action(service, job["id"], "retry_failed")
        assert runner.run(job["id"], time_slice=60)["state"] == "completed"
        assert preserved.read_bytes() == original_bytes
        with service.state.db() as db:
            assert db.execute("SELECT replacement_receipt_id FROM collection_quarantines WHERE receipt_id=?", (row["id"],)).fetchone()[0]


def test_quarantine_move_crash_resumes_from_preserved_intent(tmp_path, monkeypatch):
    service, runner, job, _ = setup(tmp_path)
    row = interrupted_detail(service, runner, job, monkeypatch)
    path = service.state.root / row["intent_path"]
    path.write_bytes(path.read_bytes() + b" ")

    def fail(phase):
        if phase == "collection_after_quarantine_move":
            raise Interrupted()
    monkeypatch.setattr(receipts, "failpoint", fail)
    with pytest.raises(Interrupted):
        runner.run(job["id"], time_slice=60)
    monkeypatch.setattr(receipts, "failpoint", lambda _: None)
    result = runner.run(job["id"], time_slice=60)
    assert result["state"] == "completed_with_gaps", result
    with service.state.db() as db:
        assert db.execute("SELECT count(*) FROM collection_quarantines").fetchone()[0] == 1


def test_bad_image_stops_and_explicit_retry_fetches_new_bytes(tmp_path):
    service, runner, job, _ = setup(tmp_path)
    bad = Session(b"<html>upstream error</html>")
    runner.image_http = bad
    result = runner.run(job["id"], time_slice=60)
    assert result["state"] == "completed_with_gaps", result
    assert len(bad.calls) == 2
    failed = service.tasks(job["id"], dict(kind="media_download"))["items"]
    assert all(row["state"] == "needs_review" and row["reason"] == "COLLECTION_MEDIA_INVALID" for row in failed)
    assert all(row["summary"]["download"]["sha256"] == digest(bad.body) for row in failed)
    assert not list(service.state.root.rglob("*.downloaded"))
    runner.run(job["id"], time_slice=60)
    assert len(bad.calls) == 2
    action(service, job["id"], "retry_failed")
    good = HTTP()
    runner.image_http = good
    result = runner.run(job["id"], time_slice=60)
    assert result["state"] == "completed", result
    assert good.calls == 2
    assert result["progress"]["media"]["published"] == 2


def test_reclaims_share_one_checkpoint_and_do_no_io_in_write_transaction(tmp_path, monkeypatch):
    service, runner, job, _ = setup(tmp_path)
    assert runner.run(job["id"], time_slice=60)["state"] == "completed"
    with service.state.db() as db:
        task_id = db.execute("SELECT id FROM collection_tasks WHERE kind='media_download' LIMIT 1").fetchone()[0]
        db.execute("UPDATE collection_tasks SET state='queued' WHERE id=?", (task_id,))
    stage = receipts.staging

    def outside_transaction(*args, **kwargs):
        with service.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
        return stage(*args, **kwargs)
    monkeypatch.setattr(receipts, "staging", outside_transaction)
    monkeypatch.setattr(shutil, "copyfile", lambda *_: pytest.fail("Retry copied a checkpoint"))
    previous = None
    for attempt in range(4):
        current = service.row(job["id"])
        task = runner.claim(current, media_task=True)
        runner.resources.track(task["directory"])
        lease = runner.resources.try_reserve(task["download_directory"], task["id"], 2 * 1024**2)
        assert lease
        directory = tasks.prepare_download(service, current, task)
        if attempt == 0:
            (directory / (task["id"] + ".downloaded")).write_bytes(b"x" * 1024**2)
        assert previous is None or directory == previous
        previous = directory
        runner.failure(current, task, UpdateError("COLLECTION_REMOTE_UNAVAILABLE", "fixture"))
        lease.release()
        with service.state.db() as db:
            db.execute("UPDATE collection_tasks SET retry_at_ms=0 WHERE id=?", (task["id"],))
        assert sum(p.stat().st_size for p in service.state.root.rglob("*.downloaded")) == 1024**2
        assert not task["directory"].exists()
    action(service, job["id"], "retry_failed")
    task = runner.claim(service.row(job["id"]), media_task=True)
    lease = runner.resources.try_reserve(task["download_directory"], task["id"], 2 * 1024**2)
    tasks.prepare_download(service, service.row(job["id"]), task)
    assert not list(task["download_directory"].glob("*.downloaded"))
    lease.release()


@pytest.mark.parametrize("originals", [False, True])
@pytest.mark.parametrize("legacy", [False, True])
def test_content_revision_rejects_changed_media_but_allows_statistics(tmp_path, originals, legacy):
    from pixiv_fixtures import sample, capture, new_batch, add_images

    lib, fixture = sample(tmp_path)
    body = dict(id="12345", userId="10109777", illustType=0, pageCount=1, width=12, height=9,
                uploadDate="2026-10-03T09:00:00+08:00", xRestrict=0, aiType=1, tags=dict(tags=[]))
    detail = capture(lib, fixture, "work", "12345", body)
    normalized = normalize.work(detail)
    if legacy:
        row = normalized["work_observations"][0]
        fields = json.loads(row["source_fields_json"])
        fields["pixiv"].pop("content_revision")
        row["source_fields_json"] = canonical(fields)
    media = capture(lib, fixture, "media", "12345", [dict(width=12, height=9, urls=dict(original="https://i.pximg.net/fixture.png"))])
    records = normalize.manifest(media, normalized["work_observations"][0])
    batch = new_batch(lib, fixture)
    for name, rows in {"visibility_contexts": [fixture["context"]], "captures": [detail, media], **normalized, **records}.items():
        batch.add(name, rows)
    with lib.writer_lock():
        batch.commit()
    if originals:
        add_images(lib, fixture, records)
    lib.sync_online()
    spec = dict(refresh=dict(mode="missing_or_stale", max_age_hours=168),
                scope=dict(work_types=["illustration"], ratings=["all_ages"], include_ai=True, include_unknown_markers=True),
                media=dict(image_policy=dict(profile="original" if originals else "metadata_only"), retain_original=originals, ugoira="metadata_only"))
    at = datetime(2026, 10, 3, 12, tzinfo=timezone.utc)
    assert reusable_work(lib, "12345", spec, fixture["context"], at=at)
    for ordinal, changed in enumerate((False, True), 2):
        body["bookmarkCount"] = ordinal * 100
        if changed:
            body["uploadDate"] = "2026-10-03T11:45:00+08:00"
        current = capture(lib, fixture, "work", "12345", body, at=f"2026-10-03T0{ordinal}:00:00.000000Z")
        batch = new_batch(lib, fixture)
        for name, rows in {"captures": [current], **normalize.work(current)}.items():
            batch.add(name, rows)
        with lib.writer_lock():
            batch.commit()
        lib.sync_online()
        with Reader(lib.root, lib.cache) as reader:
            assert reader.work("12345")["manifest_state"] == ("needs_refresh" if changed else "ready")
            assert len(reader.media("12345")["items"]) == 1
        retained = reusable_work(lib, "12345", spec, fixture["context"], at=at)
        assert retained
        assert bool(retained.get("fetch_manifest")) is changed
        if changed:
            assert not retained.get("materialize_manifest") and retained["media_count"] == 0


def test_schedule_limit_follows_unfinished_job_filter(tmp_path):
    service, _, job, request = setup(tmp_path)
    action(service, job["id"], "pause")
    # 100 older blocked rows used to hide this independently runnable schedule.
    for _ in range(100):
        value = schedule(service, request["definition"])
        with service.state.db() as db:
            db.execute("UPDATE collection_schedules SET last_job=?,next_at=1 WHERE id=?", (job["id"], value["id"]))
    spec = copy.deepcopy(request["definition"])
    spec["run_budget"]["api_requests"] += 1
    runnable = schedule(service, spec)
    with service.state.db() as db:
        db.execute("UPDATE collection_schedules SET next_at=2 WHERE id=?", (runnable["id"],))
    Schedules(service).tick(at=100)
    with service.state.db() as db:
        last = db.execute("SELECT last_job FROM collection_schedules WHERE id=?", (runnable["id"],)).fetchone()[0]
    assert last and last != job["id"] and service.job(last)["state"] == "queued"


@pytest.mark.parametrize("concurrent_import", [False, True])
def test_probe_saves_rotated_cookie_with_same_revision_fence(tmp_path, concurrent_import):
    service, _, _, _ = setup(tmp_path)
    account = service.accounts.save(dict(request_key=key(), expected_revision=None, account_id=key(), label="fixture", mode="session", cookies=[cookie()]))
    rotated = {**cookie(), "value": "ROTATED_FIXTURE_ONLY"}

    class Probe:
        def probe(self):
            if concurrent_import:
                service.accounts.save(dict(request_key=key(), expected_revision=account["revision"], account_id=account["id"],
                                           label="new import", mode="session", cookies=[{**cookie(), "value": "NEWER_IMPORT_ONLY"}]))
            return normalize.visibility(service.accounts.row(account["id"])["viewer_key"], login="authenticated", user_id="4242")

        def cookie_snapshot(self):
            return [rotated]

    args = dict(request_key=key(), expected_revision=account["revision"])
    if concurrent_import:
        with pytest.raises(UpdateError) as caught:
            service.accounts.probe(account["id"], args, client=Probe())
        assert caught.value.code == "REVISION_CONFLICT"
    else:
        assert service.accounts.probe(account["id"], args, client=Probe())["account"]["state"] == "valid"
    _, saved, _ = service.accounts.session(account["id"], require_valid=False)
    assert saved[0]["value"] == ("NEWER_IMPORT_ONLY" if concurrent_import else rotated["value"])


@pytest.mark.parametrize("phase", ["collection_after_batch_plan", "collection_after_seal", "collection_after_rename", "collection_after_commit"])
def test_group_membership_survives_each_archive_boundary(tmp_path, monkeypatch, phase):
    service, runner, job, _ = setup(tmp_path)
    hit = False

    def fail(name):
        nonlocal hit
        if hit or name != phase:
            return
        with service.state.db() as db:
            plan = db.execute("SELECT b.id FROM collection_batches b JOIN collection_outbox o ON o.batch_id=b.id JOIN collection_tasks t ON t.id=o.task_id WHERE t.kind='media_download' GROUP BY b.id HAVING count(*)=2").fetchone()
        if plan:
            hit = True
            raise Interrupted()
    monkeypatch.setattr(batches, "failpoint", fail)
    monkeypatch.setattr(archive_module, "failpoint", fail)
    with pytest.raises(Interrupted):
        runner.run(job["id"], time_slice=60)
    with service.state.db() as db:
        plans = [tuple(r) for r in db.execute("SELECT id,receipt_ids_json FROM collection_batches ORDER BY id")]
    result = runner.run(job["id"], time_slice=60)
    assert hit and result["state"] == "completed", result
    assert result["progress"]["media"]["downloaded"] == result["progress"]["media"]["published"] == 2
    with service.state.db() as db:
        assert all(tuple(r) in [tuple(v) for v in db.execute("SELECT id,receipt_ids_json FROM collection_batches")] for r in plans)
        assert db.execute("SELECT count(*) FROM collection_outbox WHERE control_applied<>1 OR published<>1").fetchone()[0] == 0
    assert len(lib_commits := service.state.library(job["library_id"]).commits()) < 7, lib_commits


def test_legacy_receipt_keeps_its_original_physical_batch(tmp_path, monkeypatch):
    service, runner, job, _ = setup(tmp_path)
    row = interrupted_detail(service, runner, job, monkeypatch)
    path = service.state.root / row["intent_path"]
    value = read_json(path)
    value.update(version=1, batch_id=key())
    atomic_json(path, value)
    with service.state.db() as db:
        db.execute("UPDATE collection_outbox SET content_sha256=? WHERE id=?", (file_hash(path), row["id"]))
    result = runner.run(job["id"], time_slice=60)
    assert result["state"] == "completed", result
    with service.state.db() as db:
        assert db.execute("SELECT batch_id FROM collection_outbox WHERE id=?", (row["id"],)).fetchone()[0] == value["batch_id"]
    assert (tmp_path / "archive" / "segments" / value["batch_id"] / "manifest.json").is_file()


def test_lost_prepared_file_can_be_cancelled_with_explicit_missing_evidence(tmp_path, monkeypatch):
    service, runner, job, _ = setup(tmp_path)
    row = interrupted_detail(service, runner, job, monkeypatch)
    (service.state.root / row["intent_path"]).unlink()
    action(service, job["id"], "cancel")
    assert runner.run(job["id"])["state"] == "cancelled"
    with service.state.db() as db:
        assert db.execute("SELECT observed_sha256 FROM collection_quarantines WHERE receipt_id=?", (row["id"],)).fetchone()[0] is None


def test_serving_unavailability_is_not_misclassified_as_a_bad_receipt(tmp_path, monkeypatch):
    service, runner, job, _ = setup(tmp_path)
    row = interrupted_detail(service, runner, job, monkeypatch)
    pointer = tmp_path / "online" / "ONLINE.json"
    pointer.rename(pointer.with_suffix(".saved"))
    result = runner.run(job["id"], time_slice=60)
    assert result["state"] == "needs_review" and result["wait_reason"] == "COLLECTION_IO", result
    with service.state.db() as db:
        assert db.execute("SELECT state FROM collection_outbox WHERE id=?", (row["id"],)).fetchone()[0] == "prepared"
        assert db.execute("SELECT count(*) FROM collection_quarantines").fetchone()[0] == 0
    pointer.with_suffix(".saved").rename(pointer)
    action(service, job["id"], "resume")
    assert runner.run(job["id"], time_slice=60)["state"] == "completed"


def test_grouped_receipts_count_downloads_and_historical_reuse_separately(tmp_path, monkeypatch):
    service, runner, job, request = setup(tmp_path)
    assert runner.run(job["id"], time_slice=60)["state"] == "completed"
    request["request_key"] = key()
    request["definition"]["seeds"] = dict(kind="works", ids=["12345", "23456"])
    request["definition"]["media"]["reuse"] = dict(mode="historical_if_same_locator", max_age_hours=8760)
    following = service.create_job(request)["job"]
    monkeypatch.setattr(batches, "MAX_WAIT_SECONDS", 60)
    result = runner.run(following["id"], time_slice=60)
    assert result["state"] == "completed", result
    assert result["progress"]["media"]["downloaded"] == 2
    assert result["progress"]["media"]["historical_reused"] == 2
    assert result["progress"]["media"]["published"] == 4
    with service.state.db() as db:
        groups = list(db.execute("SELECT o.batch_id,count(*) FROM collection_outbox o JOIN collection_tasks t ON t.id=o.task_id WHERE o.job_id=? AND t.kind='media_download' GROUP BY o.batch_id", (following["id"],)))
        assert len(groups) == 1 and groups[0][1] == 4
    archive = service.state.library(job["library_id"])
    rebuilt = tmp_path / "mixed-rebuilt"
    from studio_lake.media_lake import maintenance

    maintenance.build(archive.root, rebuilt, reference_index=archive.cache)
    maintenance.verify(rebuilt)
    assert maintenance.compare(rebuilt)["equal"]


def test_v10_upgrade_is_atomic_and_preserves_existing_receipts(tmp_path, monkeypatch):
    from studio_lake.sqlite_control import Connection
    from studio_lake.updates.state import SCHEMA_VERSION, State
    from update_fixtures import remove_collection_recovery

    service, runner, job, _ = setup(tmp_path)
    assert runner.run(job["id"], time_slice=60)["state"] == "completed"
    before = service.job(job["id"])["progress"]
    with service.state.db() as db:
        remove_collection_recovery(db)
        db.execute("PRAGMA user_version=10")
        receipts_before = [tuple(row) for row in db.execute("SELECT * FROM collection_outbox ORDER BY id")]
    execute = Connection.execute

    def interrupted(self, sql, args=()):
        if sql == "PRAGMA user_version=11":
            raise OSError("fixture interrupts version advancement")
        return execute(self, sql, args)

    with monkeypatch.context() as patch:
        patch.setattr(Connection, "execute", interrupted)
        with pytest.raises(OSError):
            State(service.state.root)
    with service.state.db() as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == 10
        assert "download_generation" not in {row[1] for row in db.execute("PRAGMA table_info(collection_tasks)")}
        assert [tuple(row) for row in db.execute("SELECT * FROM collection_outbox ORDER BY id")] == receipts_before
    reopened = State(service.state.root)
    with reopened.db() as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == SCHEMA_VERSION
        assert not list(db.execute("PRAGMA foreign_key_check"))
        assert [tuple(row)[:-1] for row in db.execute("SELECT * FROM collection_outbox ORDER BY id")] == receipts_before
    assert service.job(job["id"])["progress"] == before


@pytest.mark.parametrize("failed_first,tamper", [(False, False), (True, False), (False, True)])
def test_v10_pending_publication_restores_each_archived_outcome(tmp_path, monkeypatch, failed_first, tamper):
    from studio_lake.updates.state import State
    from update_fixtures import remove_collection_recovery

    # v10 archives contain one logical receipt and no per-outcome receipt ID.
    monkeypatch.setattr(batches, "MAX_RECEIPTS", 1)
    add_receipt = batches.add_receipt

    def legacy_outcome(service, batch, result):
        add_receipt(service, batch, result)
        for outcome in batch.replay["task_outcomes"]:
            outcome.pop("receipt_id", None)

    monkeypatch.setattr(batches, "add_receipt", legacy_outcome)
    service, runner, job, _ = setup(tmp_path)
    if failed_first:
        runner.image_http = Session(b"<html>upstream error</html>")
        assert runner.run(job["id"], time_slice=60)["state"] == "completed_with_gaps"
        action(service, job["id"], "retry_failed")
        runner.image_http = HTTP()
    assert runner.run(job["id"], time_slice=60)["state"] == "completed"
    lib = service.state.library(job["library_id"])
    with service.state.db() as db:
        # Publication reached serving, but its control acknowledgement was lost.
        db.execute("UPDATE collection_outbox SET state='archive_committed',published=0 WHERE job_id=?", (job["id"],))
        current = service.row(job["id"], db)
        counters = json.loads(current["counters_json"])
        counters["published_media"] = 0
        db.execute("UPDATE collection_jobs SET counters_json=? WHERE id=?", (canonical(counters), job["id"]))
        remove_collection_recovery(db)
        db.execute("PRAGMA user_version=10")
    State(service.state.root)
    if tamper:
        with service.state.db() as db:
            batch_id = db.execute("SELECT batch_id FROM collection_outbox WHERE task_id IS NOT NULL LIMIT 1").fetchone()[0]
        replay = lib.root / "segments" / batch_id / "collection_replay.json"
        original = replay.read_bytes()
        replay.write_bytes(original + b" ")
        with pytest.raises(IntegrityError, match="replay hash"):
            receipts.publish_ack(service, lib, job["id"])
        assert service.job(job["id"])["progress"]["media"]["published"] == 0
        replay.write_bytes(original)
    receipts.publish_ack(service, lib, job["id"])
    assert service.job(job["id"])["progress"]["media"]["published"] == 2
    receipts.publish_ack(service, lib, job["id"])
    assert service.job(job["id"])["progress"]["media"]["published"] == 2
    with service.state.db() as db:
        assert db.execute("SELECT count(*) FROM collection_outbox WHERE task_id IS NOT NULL AND outcome_state IS NULL").fetchone()[0] == 0
        assert db.execute("SELECT count(*) FROM collection_outbox WHERE outcome_state='needs_review'").fetchone()[0] == (2 if failed_first else 0)


@pytest.mark.parametrize("kind", ["time", "legacy_identity", "checkpoint"])
def test_permanently_invalid_envelope_is_quarantined_before_group_assignment(tmp_path, monkeypatch, kind):
    service, runner, job, _ = setup(tmp_path)
    row = interrupted_detail(service, runner, job, monkeypatch)
    path = service.state.root / row["intent_path"]
    value = read_json(path)
    if kind == "time":
        value["acquired_at"] = 123
    elif kind == "legacy_identity":
        value.update(version=1, batch_id="not-a-uuid")
    else:
        value["checkpoints"] = [dict(stream_key="broken", expected_revision=-1, next_revision=0, next_cursor={}, exhausted=True)]
    atomic_json(path, value)
    with service.state.db() as db:
        db.execute("UPDATE collection_outbox SET content_sha256=? WHERE id=?", (file_hash(path), row["id"]))
    assert runner.run(job["id"], time_slice=60)["state"] == "completed_with_gaps"
    with service.state.db() as db:
        assert tuple(db.execute("SELECT state,batch_id FROM collection_outbox WHERE id=?", (row["id"],)).fetchone()) == ("quarantined", None)


@pytest.mark.parametrize("space", [False, True])
def test_admission_exception_releases_download_slot_and_classifies_task(tmp_path, monkeypatch, space):
    service, runner, job, _ = setup(tmp_path)
    reserve = runner.resources.try_reserve

    def unavailable(directory, key, *args, **kwargs):
        if "collection-downloads" in directory.parts:
            if space:
                raise OSError(errno.ENOSPC, "fixture disk full")
            raise UpdateError("UPDATE_RESOURCE_LIMIT", "fixture exceeds allocation limit")
        return reserve(directory, key, *args, **kwargs)

    monkeypatch.setattr(runner.resources, "try_reserve", unavailable)
    result = runner.run(job["id"], time_slice=60)
    assert result["state"] == ("waiting_resources" if space else "completed_with_gaps"), result
    assert runner.resources.download_jobs["pixiv"] == 0
    assert not runner.resources.reservations
    rows = service.tasks(job["id"], dict(kind="media_download"))["items"]
    assert len(rows) == 2 and all(row["reason"] == ("UPDATE_SPACE" if space else "COLLECTION_LIMIT") for row in rows)
