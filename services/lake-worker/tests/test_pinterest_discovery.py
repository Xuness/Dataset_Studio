import json
import uuid

import pytest

from pinterest_fixtures import Fixture
from test_pinterest_manifest import pin
from studio_lake.canonical import utc
from studio_lake.online_storage import connect
from studio_lake.pinterest.http import Response, request_parameters
from studio_lake.pinterest.lake.online import rebuild
from studio_lake.pinterest.lake.library import PinterestLibrary


def listed(identity, **changes):
    return {**pin(identity), "type": "pin", **changes}


def pages(items, cursor=("-end-",)):
    return dict(resource_response=dict(status="success", data=items), resource=dict(options=dict(bookmarks=list(cursor))))


def collection(tmp_path, handler, *, seeds=None, metadata="none", budget=None, discovery=None, detail=None):
    fixture = Fixture(tmp_path, pin_transform=detail)
    fixture.spec.update(seeds=seeds or [dict(kind="board", id="777")], metadata=dict(detail_enrichment=metadata),
        discovery={"include_sections": False, **(discovery or {})}, run_budget=budget or {})
    fixture.job = fixture.service.create(dict(request_key=str(uuid.uuid4()), definition=fixture.spec))
    original = fixture.factory
    fixture.page_calls, fixture.enrichment_calls = [], []
    def factory(root, context, *, cancelled):
        client = original(root, context, cancelled=cancelled)
        old_pin = client.pin
        def request(kind, entry):
            fixture.page_calls.append((kind, entry))
            payload = handler(kind, entry)
            endpoint, options, source = request_parameters(kind, entry)
            return Response(json.dumps(payload).encode(), endpoint, dict(options=options, source_url=source), 200, utc(), context)
        def get_pin(identity, *, expanded=False):
            if expanded:
                fixture.enrichment_calls.append(identity)
            result = old_pin(identity)
            return Response(result.body, result.endpoint, dict(options=dict(id=identity, field_set_key="auth_web_main_pin" if expanded else "detailed")),
                            result.status, result.observed_at, result.context)
        client.request, client.pin = request, get_pin
        return client
    fixture.factory = factory
    return fixture


def continue_job(fixture):
    row = fixture.service.job(fixture.job["id"])
    return fixture.service.action(dict(job_id=row["id"], action="continue", expected_revision=row["revision"]))


def test_board_budget_continuation_preserves_candidates_relations_and_raw(tmp_path):
    def handler(kind, entry):
        assert kind == "board_page"
        return pages([listed("123"), listed("124"), listed("125"), {"type": "story"}], ("next",)) if entry["cursor"] is None else pages([listed("123"), listed("126")])
    fixture = collection(tmp_path, handler, budget=dict(api_requests=1, admitted_pins=2))
    result = fixture.run()
    assert result["state"] == "waiting_budget"
    assert result["api_requests"] == 1 and not fixture.pin_calls
    assert result["metrics"]["original_downloads"] == 1
    assert "continue" in result["actions"]
    continue_job(fixture)
    result = fixture.run()
    assert result["state"] == "completed", result
    assert result["api_requests"] == 2 and result["budget_round"] == 2
    assert len(fixture.media_calls) == 1
    lib = fixture.service.library(fixture.lake["library_id"])
    db = connect(tmp_path / "index" / "online.sqlite")
    try:
        assert db.execute("SELECT count(*) FROM pins").fetchone()[0] == 4
        assert db.execute("SELECT count(*) FROM assets").fetchone()[0] == 4
        assert db.execute("SELECT count(*) FROM source_relations WHERE role='board_member'").fetchone()[0] == 5
        assert db.execute("SELECT count(*) FROM discovery_members").fetchone()[0] == 5
        assert db.execute("SELECT count(*) FROM captures").fetchone()[0] == 2
    finally:
        db.close()
    report = rebuild(PinterestLibrary.archive(lib.root), tmp_path / "rebuilt")
    assert report["served_seq"] == result["served_seq"]
    streams = fixture.service.page(dict(job_id=result["id"]), "streams")["items"]
    assert streams[0]["state"] == "exhausted" and streams[0]["pages"] == 2 and streams[0]["total"] is None


def test_section_resolution_and_members_are_distinct_from_board(tmp_path):
    def handler(kind, entry):
        if kind == "section_resolve":
            return dict(resource_response=dict(data=dict(id="888", type="board_section", board=dict(id="777"))))
        assert kind == "section_page" and entry["subject_id"] == "888"
        return pages([listed("123")])
    fixture = collection(tmp_path, handler, seeds=[dict(kind="section", id="https://www.pinterest.com/test/board/section/")])
    assert fixture.run()["state"] == "completed"
    db = connect(tmp_path / "index" / "online.sqlite")
    try:
        assert dict(db.execute("SELECT role,count(*) FROM source_relations WHERE role IN ('section_member','board_member') GROUP BY role")) == dict(section_member=1, board_member=1)
    finally:
        db.close()


def test_sampling_mismatch_switches_remaining_candidates_to_detail(tmp_path):
    def handler(kind, entry):
        return pages([listed("123"), listed("124")])
    fixture = collection(tmp_path, handler, metadata="sample", detail=lambda p: p.update(is_video=True) if p["id"] == "123" else None)
    result = fixture.run()
    assert result["state"] == "completed_with_gaps"
    assert result["metrics"]["manifest_discrepancies"] == 1
    assert result["metrics"]["sampled_manifests"] == 1
    assert fixture.enrichment_calls == ["123"]
    assert fixture.pin_calls.count("124") == 1
    streams = fixture.service.page(dict(job_id=result["id"]), "streams")["items"]
    assert streams[0]["force_detail"] == 1
    db = connect(tmp_path / "index" / "online.sqlite")
    try:
        assert list(db.execute("SELECT DISTINCT pin_id FROM assets JOIN media_entries USING(media_id)")) == [("124",)]
    finally:
        db.close()


def test_enrichment_budget_does_not_block_list_downloads(tmp_path):
    fixture = collection(tmp_path, lambda *_: pages([listed("123"), listed("124")]), metadata="all", budget=dict(detail_requests=0))
    result = fixture.run()
    assert result["state"] == "waiting_budget" and result["media_complete"]
    assert result["enrichment_pending"] == 2 and not fixture.pin_calls
    assert fixture.service.library(fixture.lake["library_id"]).verify(deep=True)["objects"] == 1


def test_sampling_selects_supported_list_manifests_after_unsupported_items(tmp_path):
    fixture = collection(tmp_path, lambda *_: pages([listed("123", is_video=True), listed("124")]), metadata="sample")
    fixture.spec["metadata"]["sample_size"] = 1
    fixture.job = fixture.service.create(dict(request_key=str(uuid.uuid4()), definition=fixture.spec))
    assert fixture.run()["state"] == "completed"
    assert fixture.enrichment_calls == ["124"]


def test_v15_upgrade_preserves_claims_frozen_definition_and_counts(tmp_path):
    from studio_lake.updates.state import State
    from update_fixtures import remove_pinterest_schema
    from pathlib import Path
    fixture = Fixture(tmp_path)
    row = fixture.service.row(fixture.job["id"])
    with fixture.state.db() as db:
        remove_pinterest_schema(db)
        db.executescript((Path(__file__).parents[1] / "src/studio_lake/pinterest/control.sql").read_text())
        db.execute("INSERT INTO pinterest_lakes VALUES(?,3,4)", (row["lake_id"],))
        values = {k: v for k, v in row.items() if k not in ("detail_requests", "budget_round", "budget_baseline_json")}
        db.execute("INSERT INTO pinterest_jobs(" + ",".join(values) + ") VALUES(" + ",".join("?" for _ in values) + ")", tuple(values.values()))
        db.execute("INSERT INTO pinterest_tasks(task_id,job_id,kind,pin_id,input_json,state,claim_token,receipt_id,attempts,updated_at) VALUES('saved-task',?,'pin_detail','123','{}','running','saved-claim','saved-receipt',1,?)", (row["id"], utc()))
        db.execute("PRAGMA user_version=15")
    upgraded = State(fixture.state.root)
    with upgraded.db() as db:
        assert db.execute("SELECT definition_json FROM pinterest_jobs WHERE id=?", (row["id"],)).fetchone()[0] == row["definition_json"]
        assert tuple(db.execute("SELECT state,claim_token,receipt_id FROM pinterest_tasks").fetchone()) == ("running", "saved-claim", "saved-receipt")
        assert db.execute("SELECT n FROM pinterest_counts WHERE state='running'").fetchone()[0] == 1
    assert list((fixture.state.root / "backups").glob("control-v15-*.sqlite"))


@pytest.mark.parametrize("case", ["repeat", "missing", "shape"])
def test_ambiguous_pagination_never_claims_exhaustion(tmp_path, case):
    def handler(kind, entry):
        if case == "shape":
            return dict(resource_response=dict(data=dict(unexpected=[])))
        value = pages([listed("123")], ("again",))
        if case == "missing":
            value.pop("resource")
        return value
    fixture = collection(tmp_path, handler)
    result = fixture.run()
    assert result["state"] == "completed_with_gaps"
    assert len(fixture.page_calls) == (2 if case == "repeat" else 1)
    db = connect(tmp_path / "index" / "online.sqlite")
    try:
        assert db.execute("SELECT coalesce(sum(exhausted),0) FROM discovery_snapshots").fetchone()[0] == 0
    finally:
        db.close()


@pytest.mark.parametrize("point", ["pinterest_after_response", "pinterest_after_prepared", "pinterest_after_seal",
    "pinterest_after_rename", "pinterest_after_commit", "pinterest_after_publish", "pinterest_after_control_replay"])
def test_page_receipt_recovery_does_not_refetch_or_skip_members(tmp_path, monkeypatch, point):
    from studio_lake import archive_io
    from studio_lake.pinterest import runner
    from studio_lake.pinterest.lake import library, online
    fixture = collection(tmp_path, lambda kind, entry: pages([listed("123")], ("next",)) if entry["cursor"] is None else pages([listed("124")]))
    class Crash(BaseException):
        pass
    fired = []
    def fault(name):
        if name == point and fixture.page_calls and not fired:
            fired.append(True)
            raise Crash()
    for module in (runner, archive_io, library, online):
        monkeypatch.setattr(module, "failpoint", fault)
    with pytest.raises(Crash):
        fixture.run()
    assert fixture.run()["state"] == "completed"
    assert len(fixture.page_calls) == 2 and len(fixture.media_calls) == 1
    assert fixture.service.library(fixture.lake["library_id"]).verify(deep=True)["objects"] == 1
    db = connect(tmp_path / "index" / "online.sqlite")
    try:
        assert db.execute("SELECT count(*) FROM assets").fetchone()[0] == 2
    finally:
        db.close()
