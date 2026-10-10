"""A successful empty response can prematurely end a populated board feed."""

import json
import sqlite3

import pytest

from studio_lake.online_storage import connect
from studio_lake.pinterest.service import Service
from studio_lake.updates.state import SCHEMA_VERSION, State
from test_pinterest_discovery import collection, listed, pages
from update_fixtures import remove_pinterest_pagination

BOARD = "https://www.pinterest.com/fixture/board/"


def board(total=None):
    value = dict(id="777", type="board", section_count=0)
    if total is not None:
        value.update(pin_count=total, sectionless_pin_count=total)
    return dict(resource_response=dict(data=value))


def due(fixture):
    with fixture.state.db() as db:
        db.execute("UPDATE pinterest_tasks SET retry_at=0 WHERE job_id=? AND state='waiting_retry'", (fixture.job["id"],))


def drain(fixture):
    for _ in range(8):
        result = fixture.run()
        if result["state"] != "waiting_retry":
            return result
        due(fixture)
    raise AssertionError("The pagination retry must be bounded")


def stream(fixture):
    return next(v for v in fixture.service.page(dict(job_id=fixture.job["id"]), "streams")["items"] if v["entrypoint"] == "board_page")


def test_transient_end_retries_exact_cursor_and_keeps_duplicate_pins_out_of_coverage(tmp_path):
    calls = 0
    def handler(kind, entry):
        nonlocal calls
        if kind == "board_resolve":
            return board(5)
        if entry["cursor"] is None:
            return pages([listed("123"), listed("124")], ("tail",))
        if entry["cursor"] == ["tail"]:
            calls += 1
            return pages([]) if calls == 1 else pages([listed("124"), listed("125")], ("last",))
        assert entry["cursor"] == ["last"]
        return pages([listed("126"), listed("127")])
    fixture = collection(tmp_path, handler, seeds=[dict(kind="board", id=BOARD)])
    assert fixture.run()["state"] == "waiting_retry"
    current = stream(fixture)
    assert current["unique_pins"] == 2 and current["members"] == 2 and current["total"] == 5
    assert current["has_cursor"] and current["reason"] == "discovery_count_shortfall"
    result = drain(fixture)
    assert result["state"] == "completed" and result["api_requests"] == 5
    current = stream(fixture)
    assert current["unique_pins"] == current["total"] == 5 and current["members"] == 6
    assert current["state"] == "exhausted" and not current["has_cursor"]
    assert result["totals"]["admitted_pins"] == 5 and len(fixture.media_calls) == 1
    db = connect(tmp_path / "index/online.sqlite")
    try:
        assert db.execute("SELECT count(*) FROM captures").fetchone()[0] == 5
        assert db.execute("SELECT count(*) FROM discovery_snapshots WHERE complete=0 AND exhausted=0").fetchone()[0] == 1
    finally:
        db.close()


@pytest.mark.parametrize("response_pins", [[], ["124"]])
def test_repeated_early_end_retains_cursor_and_explicit_count_gap(tmp_path, response_pins):
    def handler(kind, entry):
        if kind == "board_resolve":
            return board(5)
        return pages([listed("123")], ("tail",)) if entry["cursor"] is None else pages([listed(v) for v in response_pins])
    fixture = collection(tmp_path, handler, seeds=[dict(kind="board", id=BOARD)])
    result = drain(fixture)
    assert result["state"] == "completed_with_gaps" and result["api_requests"] == 5
    current = stream(fixture)
    assert current["state"] == "needs_review" and current["has_cursor"]
    assert current["reason"] == "discovery_count_shortfall" and current["total"] == 5
    assert current["unique_pins"] == 1 + len(response_pins)
    assert current["members"] == 1 + 3 * len(response_pins)
    assert not result["media_complete"]


def test_unknown_total_confirms_empty_end_without_inventing_a_total(tmp_path):
    fixture = collection(tmp_path, lambda _, entry: pages([listed("123")], ("tail",)) if entry["cursor"] is None else pages([]))
    result = drain(fixture)
    assert result["state"] == "completed" and result["api_requests"] == 4
    current = stream(fixture)
    assert current["total"] is None and current["unique_pins"] == 1
    assert current["state"] == "exhausted" and current["reason"] == "empty_result_confirmed"


def test_reported_empty_board_does_not_retry_a_valid_empty_result(tmp_path):
    fixture = collection(tmp_path, lambda kind, _: board(0) if kind == "board_resolve" else pages([]), seeds=[dict(kind="board", id=BOARD)])
    result = fixture.run()
    assert result["state"] == "completed" and result["api_requests"] == 2
    assert stream(fixture)["total"] == 0 and stream(fixture)["reason"] == "empty_result_confirmed"


def test_old_terminal_empty_can_resume_the_existing_run_and_download_cache(tmp_path):
    recovering = False
    def handler(kind, entry):
        if kind == "board_resolve":
            return board(3 if recovering else None)
        if entry["cursor"] is None:
            assert not recovering, "Recovery must preserve the actual last request cursor"
            return pages([listed("123")], ("tail",))
        assert entry["cursor"] == ["tail"]
        return pages([listed("124"), listed("125")]) if recovering else pages([])
    fixture = collection(tmp_path, handler, seeds=[dict(kind="board", id=BOARD)])
    original = drain(fixture)
    with fixture.state.db() as db:
        # v19 persisted this state after its first terminal empty response.
        db.execute("UPDATE pinterest_streams SET reason='empty_result_reason_unknown' WHERE job_id=?", (fixture.job["id"],))
    assert "retry" in fixture.service.job(original["id"])["actions"]
    recovering = True
    fixture.service.action(dict(job_id=original["id"], expected_revision=original["revision"], action="retry"))
    result = fixture.run()
    assert result["id"] == original["id"] and result["state"] == "completed"
    assert result["api_requests"] == original["api_requests"] + 2
    assert result["budget_round"] == original["budget_round"] and result["totals"]["admitted_pins"] == 3
    assert len(fixture.media_calls) == 1 and stream(fixture)["unique_pins"] == stream(fixture)["total"] == 3


def test_v19_upgrade_backfills_distinct_candidates_and_keeps_claims(tmp_path):
    fixture = collection(tmp_path, lambda *_: pages([listed("123"), listed("124")]), budget=dict(admitted_pins=1))
    assert fixture.run()["state"] == "waiting_budget"
    with fixture.state.db() as db:
        remove_pinterest_pagination(db)
        db.execute("UPDATE pinterest_tasks SET state='running',claim_token='saved',receipt_id='receipt' WHERE job_id=? AND state='waiting_budget'", (fixture.job["id"],))
        db.execute("PRAGMA user_version=19")
        tasks = db.execute("SELECT * FROM pinterest_tasks ORDER BY task_row").fetchall()
    fixture.state = State(fixture.state.root)
    fixture.service = Service(fixture.state)
    with fixture.state.db() as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == SCHEMA_VERSION
        assert db.execute("SELECT * FROM pinterest_tasks ORDER BY task_row").fetchall() == tasks
    assert stream(fixture)["unique_pins"] == 2 and stream(fixture)["total"] is None
    assert fixture.service.job(fixture.job["id"])["totals"]["admitted_pins"] == 1
    backups = list((fixture.state.root / "backups").glob("control-v19-*.sqlite"))
    assert len(backups) == 1
    with sqlite3.connect(backups[0]) as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == 19
        assert not db.execute("SELECT 1 FROM sqlite_master WHERE name='pinterest_stream_pins'").fetchone()


@pytest.mark.parametrize("point", ["pinterest_after_response", "pinterest_after_prepared", "pinterest_after_control_replay"])
def test_empty_page_retry_survives_receipt_recovery(tmp_path, monkeypatch, point):
    from studio_lake.pinterest import runner
    from studio_lake import archive_io
    from studio_lake.pinterest.lake import library, online
    calls = 0
    def handler(kind, entry):
        nonlocal calls
        if kind == "board_resolve":
            return board(2)
        if entry["cursor"] is None:
            return pages([listed("123")], ("tail",))
        calls += 1
        return pages([]) if calls == 1 else pages([listed("124")])
    fixture = collection(tmp_path, handler, seeds=[dict(kind="board", id=BOARD)])
    class Crash(BaseException):
        pass
    hit = False
    def crash(name):
        nonlocal hit
        if name == point and calls == 1 and not hit:
            hit = True
            raise Crash()
    for module in (runner, archive_io, library, online):
        monkeypatch.setattr(module, "failpoint", crash)
    with pytest.raises(Crash):
        fixture.run()
    assert drain(fixture)["state"] == "completed"
    assert calls == 2 and stream(fixture)["unique_pins"] == 2
    with fixture.state.db() as db:
        saved = json.loads(db.execute("SELECT parameters_json FROM pinterest_streams WHERE scan_id=?", (stream(fixture)["scan_id"],)).fetchone()[0])
        assert saved["source_total"]["count"] == 2 and saved["source_total"]["capture_id"]
