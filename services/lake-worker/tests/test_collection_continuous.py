import copy
from dataclasses import replace
from datetime import datetime, timedelta, timezone
import json

import pytest

from test_collections import Client, key, setup
from studio_lake.collections import incremental, receipts
from studio_lake.collections.schedules import Schedules
from studio_lake.collections.service import Service
from studio_lake.media_lake.reader import Reader
from studio_lake.media_lake.schema import utc
from studio_lake.updates.sites import UpdateError
from studio_lake.updates.state import SCHEMA_VERSION, State
from update_fixtures import remove_collection_recovery


class FreshClient(Client):
    ids = ["12345"]
    requests = []

    def request(self, kind, payload):
        response = super().request(kind, payload)
        if kind == "author_directory":
            response = replace(response, body=json.dumps(dict(error=False, body=dict(illusts={v: None for v in self.ids}, manga={}))).encode())
        return replace(response, observed_at=utc())


def fresh_setup(tmp_path, **kwargs):
    service, runner, job, request = setup(tmp_path, **kwargs)
    runner.client_factory = FreshClient
    return service, runner, job, request


def next_job(service, request):
    request = copy.deepcopy(request)
    request["request_key"] = key()
    request["definition"]["refresh"] = dict(mode="missing_or_stale", max_age_hours=168)
    return service.create_job(request)["job"], request


def test_public_completion_and_incremental_retention(tmp_path):
    service, runner, job, request = fresh_setup(tmp_path)
    first = runner.run(job["id"], time_slice=60)
    assert first["state"] == "completed" and not first["progress"]["closure"]["visibility_verified"]
    second, request = next_job(service, request)
    FreshClient.requests.clear()
    result = runner.run(second["id"], time_slice=60)
    assert result["state"] == "completed", result
    assert [k for k, _ in FreshClient.requests] == ["author_profile", "author_directory"]
    assert result["progress"]["works"]["retained"] == 1
    assert result["progress"]["media"]["retained"] == 2
    assert result["progress"]["media"]["planned"] == 2
    assert result["progress"]["download_bytes"] == 0
    assert result["progress"]["directory_delta"] == dict(added=0, unchanged=1, no_longer_listed=0)
    task = service.tasks(second["id"], dict(kind="work_detail"))["items"][0]
    assert task["reason"] == "fresh_existing_snapshot"
    assert task["summary"]["retained_work"]["served_seq"] > 0
    assert task["summary"]["directory_receipt_task_id"]
    assert result["progress"]["publication"]["archive_seq"] - first["progress"]["publication"]["archive_seq"] == 2
    reopened = Service(State(service.state.root))
    receipts.reconcile(reopened, reopened.state.library(job["library_id"]))
    assert reopened.job(second["id"])["progress"] == result["progress"]


def test_incremental_fetches_added_work_and_preserves_disappeared(tmp_path, monkeypatch):
    service, runner, job, request = fresh_setup(tmp_path)
    runner.run(job["id"], time_slice=60)
    monkeypatch.setattr(FreshClient, "ids", ["23456"])
    second, _ = next_job(service, request)
    result = runner.run(second["id"], time_slice=60)
    assert result["state"] == "completed", result
    assert result["progress"]["directory_delta"] == dict(added=1, no_longer_listed=1, unchanged=0)
    assert result["progress"]["works"]["retained"] == 0
    with Reader(tmp_path / "archive", tmp_path / "online") as reader:
        assert reader.work("12345")["observation"] is not None
        assert reader.work("23456")["observation"] is not None


def test_metadata_only_history_cannot_hide_missing_originals(tmp_path):
    service, runner, job, request = fresh_setup(tmp_path, metadata_only=True)
    runner.run(job["id"], time_slice=60)
    request["definition"]["media"].update(retain_original=True, ugoira="archive_with_poster")
    request["definition"]["media"]["image_policy"]["profile"] = "original"
    second, _ = next_job(service, request)
    result = runner.run(second["id"], time_slice=60)
    assert result["state"] == "completed", result
    assert result["progress"]["works"]["retained"] == 0
    assert result["progress"]["media"]["downloaded"] == 2


def test_age_context_and_recipe_changes_require_recheck(tmp_path):
    service, runner, job, request = fresh_setup(tmp_path)
    runner.run(job["id"], time_slice=60)
    spec = {**request["definition"], "refresh": dict(mode="missing_or_stale", max_age_hours=1)}
    lib = service.state.library(job["library_id"])
    context = service.accounts.session(job["account_id"])[2]
    assert incremental.reusable_work(lib, "12345", spec, context)
    assert incremental.reusable_work(lib, "12345", spec, context, at=datetime.now(timezone.utc)+timedelta(hours=2)) is None
    assert incremental.reusable_work(lib, "12345", spec, {**context, "comparison_key": "different"}) is None
    spec["media"]["image_policy"]["profile"] = "webp-2048-q95"
    assert incremental.reusable_work(lib, "12345", spec, context) is None


def test_identical_active_intent_coalesces_and_request_stays_bound(tmp_path):
    service, _, job, request = fresh_setup(tmp_path)
    args = {**request, "request_key": key()}
    result = service.create_job(args)
    assert result["coalesced"] and result["job"]["id"] == job["id"]
    service.action(job["id"], dict(request_key=key(), expected_revision=result["job"]["revision"], action="cancel"))
    assert service.create_job(args)["job"]["id"] == job["id"]
    assert service.create_job({**request, "request_key": key()})["job"]["id"] != job["id"]


def test_review_blocked_snapshot_can_be_rechecked_as_new_intent(tmp_path):
    service, _, job, request = fresh_setup(tmp_path)
    with service.state.db() as db:
        db.execute("UPDATE collection_jobs SET state='needs_review',error_code='COLLECTION_SCOPE_CHANGED' WHERE id=?", (job["id"],))
    result = service.create_job({**request, "request_key": key()})
    assert result["job"]["id"] != job["id"] and not result["coalesced"]


def schedule(service, spec):
    return Schedules(service).save(dict(request_key=key(), id=key(), expected_revision=0, definition=spec,
                                        every_seconds=60, first_run_at="2026-10-01T00:00:00Z", enabled=True))


def test_schedule_coalesces_missed_ticks_and_waits_for_unfinished_job(tmp_path):
    service, runner, job, request = fresh_setup(tmp_path)
    runner.run(job["id"], time_slice=60)
    value = schedule(service, request["definition"])
    controller = Schedules(service)
    at = datetime(2026, 10, 2, tzinfo=timezone.utc).timestamp()
    controller.tick(at)
    first = controller.list({})["items"][0]
    assert first["last_job"] and first["last_job"] != job["id"]
    waiting = service.job(first["last_job"])
    service.action(waiting["id"], dict(request_key=key(), expected_revision=waiting["revision"], action="pause"))
    controller.tick(at+3600)
    assert controller.list({})["items"][0]["last_job"] == first["last_job"]
    waiting = service.job(waiting["id"])
    service.action(waiting["id"], dict(request_key=key(), expected_revision=waiting["revision"], action="cancel"))
    controller.tick(at+3600)
    assert controller.list({})["items"][0]["last_job"] != first["last_job"]
    assert controller.list({})["items"][0]["id"] == value["id"]
    assert len(service.jobs({})["items"]) == 3


def test_schedule_revision_replay_removal_and_workspace_pages(tmp_path):
    service, _, job, request = fresh_setup(tmp_path)
    value = schedule(service, request["definition"])
    with pytest.raises(UpdateError, match="changed"):
        Schedules(service).remove(dict(id=value["id"], expected_revision=2, request_key=key()))
    args = dict(id=value["id"], expected_revision=1, request_key=key())
    assert Schedules(service).remove(args) == Schedules(service).remove(args) == dict(removed=True)
    service.action(job["id"], dict(action="pause", expected_revision=job["revision"], request_key=key()))
    other = service.create_job({**request, "request_key": key()})["job"]
    first = service.dispatch("workspace_jobs", dict(limit=1))
    second = service.dispatch("workspace_jobs", dict(limit=1, cursor=first["next_cursor"]))
    assert {r["job"]["id"] for r in first["items"]+second["items"]} == {job["id"], other["id"]}
    with pytest.raises(UpdateError):
        service.dispatch("workspace_jobs", dict(state="paused", cursor=first["next_cursor"]))
    assert service.dispatch("workspace_lakes", {})["items"][0]["site"] == "pixiv"


def test_v8_upgrade_preserves_completed_facts_and_controls(tmp_path):
    service, runner, job, _ = fresh_setup(tmp_path)
    result = runner.run(job["id"], time_slice=60)
    with service.state.db() as db:
        remove_collection_recovery(db)
        db.execute("DROP TABLE collection_schedules")
        db.execute("ALTER TABLE collection_tasks DROP COLUMN summary_json")
        for index in ("collection_job_definition", "collection_job_created", "update_job_created"):
            db.execute("DROP INDEX " + index)
        db.execute("PRAGMA user_version=8")
    reopened = Service(State(service.state.root))
    assert reopened.job(job["id"])["progress"] == result["progress"]
    with reopened.state.db() as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == SCHEMA_VERSION


def test_early_development_v8_indexes_rebuilt_from_archive(tmp_path):
    service, runner, job, request = fresh_setup(tmp_path)
    result = runner.run(job["id"], time_slice=60)
    second, _ = next_job(service, request)
    runner.run(second["id"], time_slice=60)
    with service.state.db() as db:
        remove_collection_recovery(db)
        original = [tuple(r) for r in db.execute("SELECT * FROM collection_discovery_edges ORDER BY snapshot_id,target_id")]
        db.execute("DROP TRIGGER collection_count_insert")
        db.execute("DROP TRIGGER collection_count_update")
        db.execute("DROP TABLE collection_counts")
        db.execute("DROP TABLE collection_discovery_edges")
        db.execute("ALTER TABLE collection_entities DROP COLUMN expanded_depth")
        db.execute("PRAGMA user_version=9")
    reopened = Service(State(service.state.root))
    assert reopened.job(job["id"])["progress"] == result["progress"]
    with reopened.state.db() as db:
        assert [tuple(r) for r in db.execute("SELECT * FROM collection_discovery_edges ORDER BY snapshot_id,target_id")] == original


def test_directory_reuse_replays_after_archive_acceptance(tmp_path, monkeypatch):
    service, runner, first, request = fresh_setup(tmp_path)
    runner.run(first["id"], time_slice=60)
    job, _ = next_job(service, request)
    hit = False

    class PowerLoss(BaseException):
        pass

    def fail(phase):
        nonlocal hit
        if phase != "collection_after_outbox_archive" or hit:
            return
        with service.state.db() as db:
            ready = db.execute("SELECT 1 FROM collection_outbox o JOIN collection_tasks t ON t.id=o.task_id WHERE o.job_id=? AND t.kind='author_directory' AND o.state='archive_committed'", (job["id"],)).fetchone()
        if ready:
            hit = True
            raise PowerLoss()

    monkeypatch.setattr(receipts, "failpoint", fail)
    with pytest.raises(PowerLoss):
        runner.run(job["id"], time_slice=60)
    result = runner.run(job["id"], time_slice=60)
    assert result["state"] == "completed" and result["progress"]["works"]["retained"] == 1
    assert result["progress"]["media"]["retained"] == 2
    assert result["progress"]["download_bytes"] == 0
    with service.state.db() as db:
        assert db.execute("SELECT count(*) FROM collection_dependencies WHERE job_id=?", (job["id"],)).fetchone()[0] >= 1


@pytest.mark.parametrize("source_error", [False, True])
def test_unrecognized_and_error_responses_keep_raw_evidence(tmp_path, source_error):
    service, runner, job, _ = fresh_setup(tmp_path)
    raw = json.dumps(dict(error=source_error, body=dict(unknown_future_schema=True))).encode()

    class ChangedClient(FreshClient):
        def request(self, kind, payload):
            response = super().request(kind, payload)
            if kind != "work_detail":
                return response
            response = replace(response, body=raw)
            if source_error:
                error = UpdateError("COLLECTION_NOT_ACCESSIBLE", "Source returned an error")
                error.response = response
                raise error
            return response

    runner.client_factory = ChangedClient
    result = runner.run(job["id"], time_slice=60)
    assert result["state"] == "completed_with_gaps"
    assert result["progress"]["works"]["gaps"] == 1
    task = service.tasks(job["id"], dict(kind="work_detail"))["items"][0]
    assert task["reason"] == ("COLLECTION_NOT_ACCESSIBLE" if source_error else "COLLECTION_NORMALIZATION_FAILED")
    with Reader(tmp_path / "archive", tmp_path / "online") as reader:
        saved = reader.raw(task["summary"]["capture_id"])
        assert json.loads(saved["json"]) == json.loads(raw)
        assert saved["bytes"] == len(raw)
