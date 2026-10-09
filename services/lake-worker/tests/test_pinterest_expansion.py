import json

from test_pinterest_discovery import collection, listed, pages
from studio_lake.online_storage import connect
from studio_lake.pinterest.http import Response
from studio_lake.canonical import utc


def test_recommendations_keep_membership_separate_and_count_source_cost(tmp_path):
    def handler(kind, entry):
        if kind == "board_page":
            return pages([listed("123")])
        assert kind == "board_more_ideas"
        return pages([listed("124"), listed("123"), {"type": "story"}])
    fixture = collection(tmp_path, handler, discovery=dict(entrypoints=["board_more_ideas"], max_depth=1))
    result = fixture.run()
    assert result["state"] == "completed", result
    assert result["metrics"]["board_more_ideas:unique_pins"] == 2
    assert result["metrics"]["board_more_ideas:admitted_pins"] == 1
    assert result["metrics"]["board_more_ideas:unique_objects"] == 1
    db = connect(tmp_path / "index/online.sqlite")
    try:
        assert db.execute("SELECT count(*) FROM source_relations WHERE role='board_member'").fetchone()[0] == 1
        assert db.execute("SELECT count(*) FROM source_relations WHERE role='recommended_for_board'").fetchone()[0] == 2
    finally:
        db.close()


def test_pin_recommendations_require_details_for_incomplete_items_and_depth_is_bounded(tmp_path):
    def handler(kind, entry):
        assert kind == "related_pins"
        return pages([dict(type="pin", id="124", images=listed("124")["images"])])
    fixture = collection(tmp_path, handler, seeds=[dict(kind="pin", id="123")],
                         discovery=dict(entrypoints=["related_pins"], max_depth=1))
    result = fixture.run()
    assert result["state"] == "completed"
    assert fixture.pin_calls == ["123", "124"]
    assert len(fixture.page_calls) == 1
    db = connect(tmp_path / "index/online.sqlite")
    try:
        assert db.execute("SELECT count(*) FROM source_relations WHERE role='board_member'").fetchone()[0] == 0
        assert db.execute("SELECT count(*) FROM source_entities WHERE kind='board'").fetchone()[0] == 0
    finally:
        db.close()


def test_board_search_preserves_ranking_and_expands_only_admitted_boards(tmp_path):
    def handler(kind, entry):
        if kind == "search_page":
            return pages([dict(type="board", id="777", name="A"), dict(type="board", id="778", name="B")])
        assert kind == "board_page" and entry["subject_id"] == "777"
        return pages([listed("123")])
    fixture = collection(tmp_path, handler, seeds=[dict(kind="search_boards", id="watercolor landscape")],
                         budget=dict(admitted_boards=1))
    result = fixture.run()
    assert result["state"] == "waiting_budget"
    assert result["metrics"]["search_boards:unique_boards"] == 2
    assert len(fixture.page_calls) == 2
    with fixture.state.db() as db:
        task = db.execute("SELECT input_json FROM pinterest_tasks WHERE pin_id='778'").fetchone()
        value = json.loads(task[0])
        assert value["discovery_ordinal"] == 1 and value["capture_id"]


def test_empty_search_records_unknown_reason_with_explicit_end(tmp_path):
    fixture = collection(tmp_path, lambda *_: pages([]), seeds=[dict(kind="search_pins", id="unknown query")])
    result = fixture.run()
    assert result["state"] == "completed" and not fixture.media_calls
    stream = fixture.service.page(dict(job_id=result["id"]), "streams")["items"][0]
    assert stream["state"] == "exhausted" and stream["reason"] == "empty_result_reason_unknown" and stream["total"] is None


def test_entry_request_budget_preserves_next_cursor(tmp_path):
    fixture = collection(tmp_path, lambda *_: pages([listed("123")], ("next",)),
        seeds=[dict(kind="search_pins", id="test")], discovery=dict(entry_requests=dict(search_page=1)))
    result = fixture.run()
    assert result["state"] == "waiting_budget" and len(fixture.page_calls) == 1
    stream = fixture.service.page(dict(job_id=result["id"]), "streams")["items"][0]
    assert stream["has_cursor"] and stream["state"] == "active"


def test_enrichment_only_requests_admitted_pins(tmp_path):
    fixture = collection(tmp_path, lambda *_: pages([listed("123"), listed("124")]), metadata="all", budget=dict(admitted_pins=1))
    assert fixture.run()["state"] == "waiting_budget"
    assert fixture.enrichment_calls == ["123"]


def test_ideas_resources_preserve_original_html_and_topic_relationship(tmp_path):
    fixture = collection(tmp_path, lambda *_: None, seeds=[dict(kind="topic", id="https://www.pinterest.com/ideas/test/777/")])
    options = dict(interest="777", field_set_key="unauth_react", gated=True)
    def resource(value):
        return {json.dumps(list(options.items())): value}
    document = dict(initialReduxState=dict(resources=dict(
        InterestResource=resource(dict(data=dict(id="777", name="Topic", related_boards=[dict(id="888", type="board")]))),
        BestPinsFeedAltResource=resource(dict(data=[listed("123")], nextBookmark="-end-")))))
    body = ('<html><script id="__PWS_INITIAL_PROPS__" type="application/json">' + json.dumps(document) + '</script></html>').encode()
    original = fixture.factory
    def factory(root, context, *, cancelled):
        client = original(root, context, cancelled=cancelled)
        client.request = lambda kind, entry: Response(body, "/ideas/test/777/", dict(options=dict(field_set_key="__PWS_INITIAL_PROPS__")), 200, utc(), context)
        return client
    fixture.factory = factory
    result = fixture.run()
    assert result["state"] == "completed", result
    db = connect(tmp_path / "index/online.sqlite")
    try:
        assert db.execute("SELECT raw_format FROM captures").fetchone()[0] == "text"
        assert db.execute("SELECT count(*) FROM source_relations WHERE role='topic_result'").fetchone()[0] == 1
        assert db.execute("SELECT count(*) FROM source_relations WHERE role='board_member'").fetchone()[0] == 0
        assert db.execute("SELECT count(*) FROM discovery_members").fetchone()[0] == 1
    finally:
        db.close()
