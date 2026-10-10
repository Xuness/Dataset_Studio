"""Source contradictions and shallower paths must survive admission deduplication."""

import pytest

from test_pinterest_discovery import collection, listed, pages
from test_pinterest_pagination import BOARD, drain


@pytest.mark.parametrize("sections", [[], [dict(id="888", type="board_section", pin_count=1000)] * 2])
def test_missing_or_duplicate_sections_cannot_report_complete(tmp_path, sections):
    def handler(kind, entry):
        if kind == "board_resolve":
            return dict(resource_response=dict(data=dict(id="777", section_count=2, sectionless_pin_count=0, pin_count=2000)))
        if kind == "board_sections":
            return pages(sections)
        return pages([])

    fixture = collection(tmp_path, handler, seeds=[dict(kind="board", id=BOARD)], discovery=dict(include_sections=True))
    result = drain(fixture)
    assert result["state"] == "completed_with_gaps" and not result["media_complete"]
    assert "retry" in result["actions"]
    streams = fixture.service.page(dict(job_id=result["id"]), "streams")["items"]
    sections_stream = next(s for s in streams if s["entrypoint"] == "board_sections")
    assert sections_stream["unique_sections"] == bool(sections)
    assert sections_stream["total"] == 2 and sections_stream["reason"] == "discovery_count_shortfall"
    if sections:
        pins_stream = next(s for s in streams if s["entrypoint"] == "section_page")
        assert pins_stream["total"] == 1000 and pins_stream["reason"] == "discovery_count_shortfall"
    fixture.service.action(dict(job_id=result["id"], action="retry", expected_revision=result["revision"]))
    assert drain(fixture)["state"] == "completed_with_gaps"


def test_section_counts_follow_their_capture_and_complete_when_satisfied(tmp_path):
    def handler(kind, entry):
        if kind == "board_resolve":
            return dict(resource_response=dict(data=dict(id="777", section_count=2, sectionless_pin_count=0)))
        if kind == "board_sections":
            if entry["cursor"] is None:
                return pages([dict(id="888", type="board_section", pin_count=1)], ("next",))
            return pages([dict(id="888", type="board_section", pin_count=1), dict(id="889", type="board_section", pin_count=0)])
        if kind == "section_page" and entry["subject_id"] == "888":
            return pages([listed("123")])
        return pages([])

    fixture = collection(tmp_path, handler, seeds=[dict(kind="board", id=BOARD)], discovery=dict(include_sections=True))
    result = drain(fixture)
    assert result["state"] == "completed" and result["media_complete"]
    streams = fixture.service.page(dict(job_id=result["id"]), "streams")["items"]
    assert next(s for s in streams if s["entrypoint"] == "board_sections")["unique_sections"] == 2
    with fixture.state.db() as db:
        import json
        hint = json.loads(db.execute("SELECT parameters_json FROM pinterest_streams WHERE subject_id='888'").fetchone()[0])["source_total"]
        assert hint["field"] == "pin_count" and hint["count"] == 1 and hint["capture_id"] and hint["observed_at"]


@pytest.mark.parametrize("first", ["123", "124"])
def test_shallower_duplicate_pin_expands_without_redownloading(tmp_path, first):
    def handler(kind, entry):
        if kind == "board_page":
            return pages([listed(first)], ("next",)) if entry["cursor"] is None else pages([listed("124")])
        assert kind == "related_pins"
        return pages([listed({"123": "124", "124": "125"}[entry["subject_id"]])])

    fixture = collection(tmp_path, handler, discovery=dict(entrypoints=["related_pins"], max_depth=1))
    result = drain(fixture)
    assert result["state"] == "completed"
    assert "124" in [e["subject_id"] for k, e in fixture.page_calls if k == "related_pins"]
    assert len(fixture.media_calls) == 1
    with fixture.state.db() as db:
        assert db.execute("SELECT 1 FROM pinterest_admitted WHERE job_id=? AND kind='pin' AND source_id='125'", (result["id"],)).fetchone()


def test_depth_improvement_propagates_through_previously_exhausted_streams(tmp_path):
    def handler(kind, entry):
        if kind == "board_page":
            return pages([listed("123")], ("next",)) if entry["cursor"] is None else pages([listed("124")])
        child = {"123": "124", "124": "125", "125": "126", "126": "127"}.get(entry["subject_id"])
        return pages([listed(child)] if child else [])

    fixture = collection(tmp_path, handler, discovery=dict(entrypoints=["related_pins"], max_depth=3))
    result = drain(fixture)
    assert result["state"] == "completed"
    with fixture.state.db() as db:
        assert db.execute("SELECT 1 FROM pinterest_admitted WHERE job_id=? AND source_id='127'", (result["id"],)).fetchone()
        assert db.execute("SELECT min_depth FROM pinterest_expansion_depths WHERE job_id=? AND kind='pin' AND source_id='124'", (result["id"],)).fetchone()[0] == 0
    assert len(fixture.page_calls) < 15 and len(fixture.media_calls) == 1


def test_seed_depth_prevents_reentering_a_recommendation_cycle(tmp_path):
    def handler(kind, entry):
        assert kind == "related_pins"
        return pages([listed("124" if entry["subject_id"] == "123" else "123")])
    fixture = collection(tmp_path, handler, seeds=[dict(kind="pin", id="123")],
        discovery=dict(entrypoints=["related_pins"], max_depth=3))
    assert fixture.run()["state"] == "completed"
    assert [e["subject_id"] for _, e in fixture.page_calls] == ["123", "124"]
    with fixture.state.db() as db:
        assert db.execute("SELECT min_depth FROM pinterest_expansion_depths WHERE job_id=? AND source_id='123'", (fixture.job["id"],)).fetchone()[0] == 0


def test_v20_upgrade_exposes_old_section_contradiction_without_changing_claims(tmp_path):
    from studio_lake.updates.state import State
    from update_fixtures import remove_pinterest_coverage
    from studio_lake.pinterest.service import Service
    import sqlite3

    def handler(kind, entry):
        if kind == "board_resolve":
            return dict(resource_response=dict(data=dict(id="777", section_count=2, sectionless_pin_count=0)))
        return pages([])

    fixture = collection(tmp_path, handler, seeds=[dict(kind="board", id=BOARD)], discovery=dict(include_sections=True))
    drain(fixture)
    with fixture.state.db() as db:
        remove_pinterest_coverage(db)
        db.execute("UPDATE pinterest_streams SET state='exhausted',reason='empty_result_reason_unknown' WHERE entrypoint='board_sections'")
        db.execute("UPDATE pinterest_tasks SET state='done',reason=NULL")
        db.execute("UPDATE pinterest_jobs SET state='completed' WHERE id=?", (fixture.job["id"],))
        db.execute("PRAGMA user_version=20")
        tasks = db.execute("SELECT * FROM pinterest_tasks ORDER BY task_row").fetchall()
        definition = fixture.service.row(fixture.job["id"], db)["definition_json"]
    fixture.state = State(fixture.state.root)
    fixture.service = Service(fixture.state)
    with fixture.state.db() as db:
        assert db.execute("SELECT * FROM pinterest_tasks ORDER BY task_row").fetchall() == tasks
        assert fixture.service.row(fixture.job["id"], db)["definition_json"] == definition
    result = fixture.service.job(fixture.job["id"])
    assert result["state"] == "completed_with_gaps" and not result["media_complete"]
    assert "retry" in result["actions"]
    backups = list((fixture.state.root / "backups").glob("control-v20-*.sqlite"))
    assert len(backups) == 1
    with sqlite3.connect(backups[0]) as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == 20
    fixture.service.action(dict(job_id=result["id"], action="retry", expected_revision=result["revision"]))
    assert drain(fixture)["state"] == "completed_with_gaps"
