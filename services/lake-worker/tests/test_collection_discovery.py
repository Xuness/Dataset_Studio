import json

from test_collections import Client, key, setup
from studio_lake.collectors.pixiv.http import Response
from studio_lake.collections import planner


def test_author_graph_paginates_and_stops_at_frozen_depth(tmp_path):
    service, runner, _, request = setup(tmp_path, metadata_only=True)
    request["request_key"] = key()
    request["definition"]["discovery"] = dict(entrypoints=["following", "bookmarks", "recommendations"],
                                               max_depth=1, recommendation_seeds_per_author=1)
    job = service.create_job(request)["job"]
    calls = []

    class Graph(Client):
        def request(self, kind, payload):
            calls.append((kind, dict(payload)))
            if kind == "relationship_page":
                relation = payload["relation"]
                if relation == "following":
                    # Repeated IDs may occur across a changing remote listing. Keep
                    # observations while deduplicating the frontier and its requests.
                    body = dict(users=[dict(userId="201")] * 100, total=101) if payload["cursor"] == 0 else dict(users=[dict(userId="202")], total=101)
                elif relation == "bookmarks":
                    body = dict(works=[dict(id="23456")], total=1)
                else:
                    body = dict(illusts=[dict(id="23456")], nextIds=["34567", "23456"])
            else:
                response = super().request(kind, payload)
                body = json.loads(response.body)["body"]
                if kind == "author_directory":
                    work = {"10109777": "12345", "201": "34567", "202": "23456"}[payload["author_id"]]
                    body = dict(illusts={work: None}, manga={})
                elif kind == "work_detail":
                    body["userId"] = {"12345": "10109777", "23456": "202", "34567": "201"}[payload["work_id"]]
            return Response(json.dumps(dict(error=False, body=body)).encode(), "/fixture/" + kind, {}, 200,
                            "2026-10-03T00:00:00.000000Z", "identity")

    runner.client_factory = Graph
    result = runner.run(job["id"], time_slice=60)
    assert result["state"] == "completed_with_gaps", result
    assert result["progress"]["authors"]["scanned"] == 3
    assert result["progress"]["works"]["details"] == 3
    relationships = [p for k, p in calls if k == "relationship_page"]
    assert {p["root_id"] for p in relationships} == {"10109777", "12345"}
    assert [p["cursor"] for p in relationships if p["relation"] == "following"] == [0, 100]
    assert len([p for k, p in calls if k == "work_detail" and p["work_id"] == "23456"]) == 1
    with service.state.db() as db:
        authors = dict(db.execute("SELECT source_id,min_depth FROM collection_entities WHERE job_id=? AND kind='author'", (job["id"],)))
        assert authors == {"10109777": 0, "201": 1, "202": 1}
        assert db.execute("SELECT revision FROM collection_checkpoints WHERE job_id=? AND stream_key='author:10109777:following'", (job["id"],)).fetchone()[0] == 2
        assert db.execute("SELECT count(*) FROM collection_tasks WHERE job_id=? AND reason='depth_limit'", (job["id"],)).fetchone()[0] > 0


def test_shorter_path_reopens_only_depth_exclusions_and_relaxes_known_descendants(tmp_path):
    service, _, job, _ = setup(tmp_path, metadata_only=True)
    # Use a new immutable scope with a one-hop exploration boundary.
    definition = job["definition"]
    definition["discovery"] = dict(entrypoints=["following"], max_depth=1, recommendation_seeds_per_author=0)
    job = service.create_job(dict(request_key=key(), definition=definition))["job"]
    with service.state.db() as db:
        planner.entity(db, job["id"], "author", "201", 1)
        planner.entity(db, job["id"], "work", "34567", 1)
        db.execute("UPDATE collection_entities SET expanded_depth=1,state='processed' WHERE job_id=? AND source_id='201'", (job["id"],))
        db.execute("INSERT INTO collection_discovery_edges VALUES(?, 'author','201','work','34567',0,'captured-proof')", (job["id"],))
        identity = planner.task(db, job["id"], "relationship_page", "201", dict(root_kind="author", root_id="201", relation="following", cursor=0, directory_snapshot_id="captured-proof"))
        db.execute("UPDATE collection_tasks SET state='excluded',reason='depth_limit' WHERE id=?", (identity,))
        planner.entity(db, job["id"], "author", "201", 0)
        planner.advance(db, service.row(job["id"], db))
        assert db.execute("SELECT state FROM collection_tasks WHERE id=?", (identity,)).fetchone()[0] == "queued"
        assert db.execute("SELECT min_depth FROM collection_entities WHERE job_id=? AND source_id='34567'", (job["id"],)).fetchone()[0] == 0
    assert planner.recommendation_seeds([str(i) for i in range(1, 101)], 3) == ["1", "50", "100"]
