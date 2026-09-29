import hashlib
import json
import re
import time
import zlib
import threading
from concurrent.futures import ThreadPoolExecutor

import pytest

from conftest import png
from studio_lake.config import Config
from studio_lake.index import Index
from studio_lake.library import Library
from studio_lake.online_migrate import migrate, verify_projection, activate_projection
from studio_lake.updates.archive import online, reconcile
from studio_lake.updates.credentials import encode, decode
from studio_lake.updates.media import Resources
from studio_lake.updates.protocol import definition
from studio_lake.updates.runner import Runner
from studio_lake.updates.sites import Site, Response, UpdateError, split_response
from studio_lake.updates.state import State
from studio_lake.util import atomic_json
from studio_lake.updates.read_model import activity


class FakeSite(Site):
    def __init__(self, name, records, bad=False):
        super().__init__(name, delay=0)
        self.records, self.bad, self.calls = records, bad, []

    def request(self, params, cancelled=lambda: False, resource="posts"):
        if resource != "posts":
            tags = {tag for r in self.records for tag in r.get("tags", "").split(" ") if tag}
            data = (
                {
                    "version": 1,
                    "data": " ".join(("4" if tag == "b" else "0") + "`" + tag + "`" for tag in tags),
                }
                if resource == "tag_summary"
                else {
                    "@attributes": {"count": len(tags)},
                    "tag": [{"name": tag, "type": 4 if tag == "b" else 0} for tag in tags],
                }
            )
            return Response(
                json.dumps(data).encode(), 200, {"endpoint": "reference-fixture", "parameters": params}
            )
        self.calls.append(params)
        tags = params.get("tags", "")
        records = list(self.records)
        if not self.bad:
            if "id" in params:
                records = [r for r in records if r["id"] == params["id"]]
            elif m := re.search(r"id:(\d+)\.\.(\d+)", tags):
                lo, hi = map(int, m.groups())
                records = [r for r in records if lo <= r["id"] <= hi]
            elif m := re.search(r"id:>(\d+) id:<(\d+)", tags):
                lo, hi = map(int, m.groups())
                records = [r for r in records if lo < r["id"] < hi]
            elif m := re.search(r"id:([\d,]+)", tags):
                ids = set(map(int, m[1].split(",")))
                records = [r for r in records if r["id"] in ids]
        records.sort(key=lambda r: r["id"], reverse="desc" in tags)
        records = records[: params["limit"]]
        data = (
            {"@attributes": {"count": len(records)}, "post": records} if self.name == "gelbooru" else records
        )
        return Response(
            json.dumps(data).encode(), 200, {"endpoint": "recorded-fixture", "parameters": params}
        )


class ImageResponse:
    status_code = 200
    headers = {}

    def __init__(self, data):
        self.data = data

    def __enter__(self):
        return self

    def __exit__(self, *_):
        pass

    def iter_content(self, _):
        yield self.data


class Images:
    headers = {}

    def __init__(self, data):
        self.data = data
        self.calls = 0

    def get(self, *_, **kwargs):
        self.calls += 1
        return ImageResponse(self.data)


def setup(tmp_path, site):
    lib = Library.initialize(
        Config(root=tmp_path / "archive", cache=tmp_path / "index", threads=1, memory_limit="1GB")
    )
    if site != "danbooru":
        atomic_json(lib.root / "source_manifests/hf-conversion-plan.json", {"site": site})
    Index(lib).sync()
    migrate(lib.root, lib.cache, lib.cache, site)
    verify_projection(lib.cache)
    activate_projection(lib.root, lib.cache)
    state = State(tmp_path / "control")
    state.register(
        {
            "library_id": lib.info["library_id"],
            "site": site,
            "media_root": str(lib.root),
            "index_root": str(lib.cache),
        }
    )
    return lib, state


def post(site, pid, data):
    value = {
        "id": pid,
        "rating": "g" if site == "danbooru" else "s" if site == "yandere" else "general",
        "score": pid,
        "md5": hashlib.md5(data).hexdigest(),
        "file_url": "https://example.invalid/file.png",
        "file_ext": "png",
        "created_at": "2026-09-25T01:00:00Z",
        "updated_at": "2026-09-26T01:00:00Z",
        "unknown": {"retained": [False, 0, None, "中文"]},
    }
    if site == "danbooru":
        value.update(tag_string="a b", image_width=4, image_height=4)
    else:
        value.update(tags="a b", width=4, height=4, status="active", change=1790384400, image="original.png")
    return value


def job(state, lib, range_, profile="metadata_only", **kw):
    return state.create(
        {
            "library_id": lib.info["library_id"],
            "range": range_,
            "media": {"profile": profile, "allow_sample": False},
            **kw,
        },
        str(time.time_ns()),
    )


@pytest.mark.parametrize("site", ["danbooru", "yandere", "gelbooru"])
def test_three_sites_archive_raw_publish_and_reuse_without_analysis(tmp_path, site):
    lib, state = setup(tmp_path, site)
    data = png("red")
    records = [post(site, 11, data), post(site, 12, data)]
    remote = FakeSite(site, records)
    images = Images(data)
    runner = Runner(state, {site: remote}, Resources(reserve_bytes=0), images)
    first = job(state, lib, {"kind": "id_range", "start": 11, "end": 13}, "original")
    done = runner.run(first["id"])
    assert done["state"] == "completed", done
    assert done["counts"] == {"stored": 2}
    assert done["telemetry"]["phase"] == "completed"
    assert done["telemetry"]["downloaded_bytes"] == 2 * len(data)
    with online(lib) as (db, s):
        assert int(s["served_seq"]) > int(s["analysis_seq"])
        assert next(db.execute("SELECT count(*) FROM objects"))[0] == 1
        assert next(db.execute("SELECT count(*) FROM assets"))[0] == 2
        if site != "danbooru":
            assert next(db.execute("SELECT tag_string_character FROM observations LIMIT 1"))[0] == "b"
        assert next(db.execute("SELECT count(*) FROM post_versions WHERE valid_until IS NULL"))[0] == 2
        raws = [json.loads(zlib.decompress(r[0])) for r in db.execute("SELECT raw_zlib FROM raw_metadata")]
        assert [r["unknown"] for r in raws] == [r["unknown"] for r in records]
        old_seq = int(s["served_seq"])
    count = images.calls
    records[0]["score"] = 99
    second = job(state, lib, {"kind": "ids", "ids": [11]}, "original")
    assert runner.run(second["id"])["counts"] == {"reused": 1}
    assert images.calls == count
    with online(lib) as (db, s):
        assert (
            next(
                db.execute(
                    "SELECT o.score FROM post_versions p JOIN observations o USING(row_id) WHERE p.post_id=11 AND p.valid_from<=? AND (p.valid_until IS NULL OR p.valid_until>?)",
                    (old_seq, old_seq),
                )
            )[0]
            == 11
        )
        assert (
            next(
                db.execute(
                    "SELECT o.score FROM post_versions p JOIN observations o USING(row_id) WHERE p.post_id=11 AND valid_until IS NULL"
                )
            )[0]
            == 99
        )
    # The independent native projection can catch up, including the new source formats.
    Index(lib).sync()
    with Index(lib).read() as db:
        assert db.execute("SELECT count(*) FROM raw_metadata").fetchone()[0] == 3


@pytest.mark.parametrize("site", ["danbooru", "yandere", "gelbooru"])
def test_bad_page_archived_without_advancing_or_projecting(tmp_path, site):
    lib, state = setup(tmp_path, site)
    task = job(state, lib, {"kind": "id_range", "start": 50, "end": 60})
    done = Runner(state, {site: FakeSite(site, [post(site, 1, png("red"))], bad=True)}).run(task["id"])
    assert done["state"] == "needs_review" and done["error_code"] == "UPDATE_PAGE_INVALID", done
    assert done["cursor"]["next_id"] == 50
    with online(lib) as (db, _):
        assert next(db.execute("SELECT count(*) FROM observations"))[0] == 0
    assert any((p / "response_body.bin").exists() for p in (lib.root / "segments").iterdir())


def test_archive_replay_recovers_queue_without_duplicate_posts(tmp_path, monkeypatch):
    lib, state = setup(tmp_path, "danbooru")
    task = job(state, lib, {"kind": "ids", "ids": [11]})
    remote = FakeSite("danbooru", [post("danbooru", 11, png("red"))])
    real = reconcile

    def crash(control, library, identity):
        with library.journal() as db:
            n = db.execute(
                "SELECT count(*) FROM commits WHERE manifest_json LIKE '%response_body.bin%'"
            ).fetchone()[0]
        if n:
            raise RuntimeError("crash after archive commit")
        real(control, library, identity)

    with monkeypatch.context() as patch:
        patch.setattr("studio_lake.updates.runner.reconcile", crash)
        assert Runner(state, {"danbooru": remote}).run(task["id"])["state"] == "needs_review"
    state.action(task["id"], "resume")
    assert Runner(state, {"danbooru": remote}).run(task["id"])["state"] == "completed"
    assert len(remote.calls) == 1
    with online(lib) as (db, _):
        assert next(db.execute("SELECT count(*) FROM observations"))[0] == 1


def test_budget_resume_and_missing_id_are_not_false_deletions(tmp_path):
    lib, state = setup(tmp_path, "gelbooru")
    remote = FakeSite("gelbooru", [post("gelbooru", 11, png("red"))])
    task = job(state, lib, {"kind": "ids", "ids": [11, 12]}, page_budget=1)
    runner = Runner(state, {"gelbooru": remote})
    first = runner.run(task["id"])
    assert first["state"] == "paused" and first["cursor"]["position"] == 1, first
    state.action(task["id"], "resume")
    done = runner.run(task["id"])
    assert done["state"] == "completed_with_exclusions", done
    assert done["counts"] == {"metadata": 1, "unavailable": 1}
    assert state.items(task["id"])["items"][1]["reason"] == "not_returned_by_api"


def test_date_filter_does_not_publish_outside_scope(tmp_path):
    lib, state = setup(tmp_path, "danbooru")
    data = png("red")
    rows = [post("danbooru", 11, data), post("danbooru", 12, data)]
    rows[1]["created_at"] = "2026-09-24T00:00:00Z"
    task = job(
        state,
        lib,
        {
            "kind": "created",
            "start": "2026-09-25T00:00:00Z",
            "end": "2026-09-26T00:00:00Z",
            "timezone": "Asia/Shanghai",
            "start_id": 11,
            "end_id": 13,
        },
    )
    done = Runner(state, {"danbooru": FakeSite("danbooru", rows)}).run(task["id"])
    assert done["state"] == "completed", done
    assert done["counts"] == {"metadata": 1}


def test_schedule_missed_ticks_coalesce_and_idempotency_conflicts(tmp_path):
    lib, state = setup(tmp_path, "danbooru")
    spec = {"library_id": lib.info["library_id"], "range": {"kind": "ids", "ids": [11]}}
    same = state.create(spec, "test-key")
    assert state.create(spec, "test-key")["id"] == same["id"]
    with pytest.raises(UpdateError, match="another definition"):
        state.create({**spec, "range": {"kind": "ids", "ids": [12]}}, "test-key")
    state.set_schedule(spec, 60, "2026-09-26T00:00:00Z", True)
    from studio_lake.updates.protocol import timestamp

    at = timestamp("2026-09-26T00:09:00Z").timestamp()
    state.tick_schedules(at)
    state.tick_schedules(at)
    assert len(state.jobs()["items"]) == 2


def test_lossless_envelope_and_strict_policies():
    raw = b'{"@attributes":{"count":1},"post":[ {"id":1,"unknown":1.2345678901234567890123} ]}'
    rows = split_response("gelbooru", raw)
    assert "1.2345678901234567890123" in rows[0][0]
    with pytest.raises(UpdateError):
        definition({"library_id": "x", "range": {"kind": "ids", "ids": [True]}})
    with pytest.raises(UpdateError):
        definition(
            {
                "library_id": "x",
                "range": {"kind": "ids", "ids": [1]},
                "media": {"profile": "original", "allow_sample": True},
            }
        )


def test_credentials_encrypt_and_survive_reopen():
    import os

    if os.name != "nt":
        pytest.skip("Windows credential provider")
    value = {"login": "fixture", "api_key": "not-a-real-secret"}
    blob = encode("danbooru", value)
    assert b"not-a-real-secret" not in blob
    assert decode(blob) == value
    with pytest.raises(UpdateError):
        decode(b"corrupted")


def test_response_saved_before_seal_resumes_without_refetch(tmp_path, monkeypatch):
    lib, state = setup(tmp_path, "gelbooru")
    task = job(state, lib, {"kind": "ids", "ids": [11]})
    remote = FakeSite("gelbooru", [post("gelbooru", 11, png("red"))])
    with monkeypatch.context() as patch:
        patch.setenv("DANBOORU_STORE_FAIL_AT", "after_update_response_saved")
        assert Runner(state, {"gelbooru": remote}).run(task["id"])["state"] == "needs_review"
    state.action(task["id"], "resume")
    done = Runner(state, {"gelbooru": remote}).run(task["id"])
    assert done["state"] == "completed", done
    assert len(remote.calls) == 1


def test_retry_refetches_missing_post_and_preserves_frozen_range(tmp_path):
    lib, state = setup(tmp_path, "gelbooru")
    remote = FakeSite("gelbooru", [])
    task = job(state, lib, {"kind": "ids", "ids": [11]})
    runner = Runner(state, {"gelbooru": remote})
    assert runner.run(task["id"])["state"] == "completed_with_exclusions"
    remote.records = [post("gelbooru", 11, png("red"))]
    state.action(task["id"], "retry")
    done = runner.run(task["id"])
    assert done["state"] == "completed" and done["counts"] == {"metadata": 1}, done


def test_sealed_input_maps_all_origins_deduplicates_and_survives_missing_ids(tmp_path):
    lib, state = setup(tmp_path, "gelbooru")
    data = png("red")
    remote = FakeSite("gelbooru", [post("gelbooru", 11, data), post("gelbooru", 12, data)])
    runner = Runner(state, {"gelbooru": remote}, Resources(reserve_bytes=0), Images(data))
    task = job(state, lib, {"kind": "id_range", "start": 11, "end": 13}, "original")
    assert runner.run(task["id"])["state"] == "completed"
    frozen = state.create_input(lib.info["library_id"], provenance={"scope": "fixed workset fixture"})
    state.append_input(frozen["id"], post_ids=[10, 11], object_sha256s=[hashlib.sha256(data).hexdigest()])
    frozen = state.seal_input(frozen["id"])
    assert frozen["count"] == 3 and frozen["state"] == "sealed"
    assert (lib.root / "plans" / "updates" / (frozen["id"] + ".ids")).read_text() == "10\n11\n12\n"
    with pytest.raises(UpdateError):
        state.append_input(frozen["id"], post_ids=[13])
    task = job(state, lib, {"kind": "input", "input_id": frozen["id"]})
    done = runner.run(task["id"])
    assert done["state"] == "completed_with_exclusions", done
    assert done["counts"] == {"unavailable": 1, "metadata": 2}


def test_missing_md5_never_attaches_new_metadata_to_an_unproven_old_image(tmp_path):
    lib, state = setup(tmp_path, "danbooru")
    data = png("red")
    rows = [post("danbooru", 11, data)]
    remote = FakeSite("danbooru", rows)
    runner = Runner(state, {"danbooru": remote}, Resources(reserve_bytes=0), Images(data))
    first = job(state, lib, {"kind": "ids", "ids": [11]}, "original")
    assert runner.run(first["id"])["state"] == "completed"
    rows[0].pop("md5")
    task = job(state, lib, {"kind": "ids", "ids": [11]})
    assert runner.run(task["id"])["state"] == "completed"
    with online(lib) as (db, _):
        assert (
            next(db.execute("SELECT asset_id FROM post_versions WHERE post_id=11 AND valid_until IS NULL"))[0]
            is None
        )
        assert next(db.execute("SELECT count(*) FROM objects"))[0] == 1


def test_new_metadata_cursor_progresses_even_when_media_needs_review(tmp_path):
    lib, state = setup(tmp_path, "danbooru")
    remote = FakeSite("danbooru", [post("danbooru", 11, png("red"))])
    task = job(state, lib, {"kind": "new", "after_id": 10}, "original")
    done = Runner(state, {"danbooru": remote}, Resources(reserve_bytes=0), Images(png("blue"))).run(
        task["id"]
    )
    assert done["state"] == "needs_review", done
    assert lib.setting("update_new_metadata_cursor") == 11
    assert done["cursor"]["metadata_complete"]


def test_pause_while_network_response_arrives_and_resume_keeps_range(tmp_path):
    lib, state = setup(tmp_path, "danbooru")
    task = job(state, lib, {"kind": "ids", "ids": [11]})
    remote = FakeSite("danbooru", [post("danbooru", 11, png("red"))])
    original = remote.request

    def pause(params, cancelled):
        response = original(params, cancelled)
        state.action(task["id"], "pause")
        return response

    remote.request = pause
    done = Runner(state, {"danbooru": remote}).run(task["id"])
    assert done["state"] == "paused" and done["counts"] == {}
    remote.request = original
    state.action(task["id"], "resume")
    assert Runner(state, {"danbooru": remote}).run(task["id"])["counts"] == {"metadata": 1}


def test_keep_existing_is_separate_from_explicit_profile_upgrade(tmp_path):
    lib, state = setup(tmp_path, "danbooru")
    image = png("red")
    remote = FakeSite("danbooru", [post("danbooru", 11, image)])
    images = Images(image)
    runner = Runner(state, {"danbooru": remote}, Resources(reserve_bytes=0), images)
    first = job(state, lib, {"kind": "ids", "ids": [11]}, "webp-2048-q95")
    assert runner.run(first["id"])["counts"] == {"stored": 1}
    second = job(state, lib, {"kind": "ids", "ids": [11]}, "original")
    assert runner.run(second["id"])["counts"] == {"reused": 1}
    assert images.calls == 1
    third = state.create(
        {
            "library_id": lib.info["library_id"],
            "range": {"kind": "ids", "ids": [11]},
            "media": {"profile": "original", "existing": "match_profile"},
        },
        "upgrade",
    )
    assert runner.run(third["id"])["counts"] == {"stored": 1}
    assert images.calls == 2


def test_single_scheduled_run_fires_once_and_disables_itself(tmp_path):
    from studio_lake.updates.protocol import timestamp

    lib, state = setup(tmp_path, "danbooru")
    state.set_schedule(
        {"library_id": lib.info["library_id"], "range": {"kind": "ids", "ids": [11]}},
        None,
        "2026-09-26T00:00:00Z",
        True,
    )
    at = timestamp("2026-09-26T00:00:01Z").timestamp()
    state.tick_schedules(at)
    state.tick_schedules(at + 86400)
    assert len(state.jobs()["items"]) == 1
    assert state.schedules()["items"][0]["enabled"] is False
    assert state.schedules()["items"][0]["every_seconds"] is None


def test_live_site_parameter_and_referrer_contracts():
    y = Site("yandere")
    params = y.params(11, 13, 2, ids=[11, 12])
    assert "status:" not in params["tags"]
    assert "deleted:all" in params["tags"] and "holds:all" in params["tags"]
    g = Site("gelbooru")
    assert g.params(11, 13, 2)["tags"] == "id:>10 id:<13 sort:id:asc"
    assert g.params(11, 12, 1, ids=[11])["id"] == 11


def test_replay_saved_response_uses_original_observation_time_without_network(tmp_path):
    from studio_lake.util import split_json_posts

    lib, state = setup(tmp_path, "danbooru")
    row = post("danbooru", 11, png("red"))
    row["id"] = "11"
    remote = FakeSite("danbooru", [row], bad=True)
    task = job(state, lib, {"kind": "ids", "ids": [11]})
    runner = Runner(state, {"danbooru": remote})
    assert runner.run(task["id"])["error_code"] == "UPDATE_RESPONSE_INVALID"

    def repaired(response):
        rows = split_json_posts(response.body)
        for _, r in rows:
            r["id"] = int(r["id"])
        return rows

    remote.parse = repaired
    state.action(task["id"], "replay")
    done = runner.run(task["id"])
    assert done["state"] == "completed", done
    assert len(remote.calls) == 1
    with lib.journal() as db:
        manifests = [json.loads(r[0]) for r in db.execute("SELECT manifest_json FROM commits ORDER BY seq")]
    failed = next(m["source"]["observed_at"] for m in manifests if m["source"]["update_role"] == "error")
    accepted = next(m["source"]["observed_at"] for m in manifests if m["source"]["update_role"] == "page")
    assert failed == accepted


def test_control_v1_migrates_without_changing_jobs_or_credentials(tmp_path):
    lib, state = setup(tmp_path, "danbooru")
    task = job(state, lib, {"kind": "ids", "ids": [11]})
    state.set_credentials("danbooru", {"login": "fixture", "api_key": "not-a-real-key"})
    with state.db() as db:
        db.execute("DROP TABLE telemetry")
        db.execute("PRAGMA user_version=1")
    reopened = State(state.root)
    assert reopened.job(task["id"])["definition"] == task["definition"]
    assert reopened.job(task["id"])["telemetry"] == {}
    assert reopened.credentials("danbooru")["api_key"] == "not-a-real-key"
    with reopened.db() as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == 7


def test_ui_recent_filters_counts_and_sparse_problems(tmp_path):
    lib, state = setup(tmp_path, "danbooru")
    created = [job(state, lib, {"kind": "ids", "ids": [11]}) for _ in range(5)]
    for i, row in enumerate(created):
        with state.db() as db:
            db.execute("UPDATE jobs SET created_at=? WHERE id=?", (f"2026-09-27T00:00:0{i}Z", row["id"]))
    state.update(created[2]["id"], state="needs_review")
    first = state.jobs(limit=2)
    assert [j["id"] for j in first["items"]] == [created[4]["id"], created[3]["id"]]
    second = state.jobs(first["next_cursor"], limit=2)
    assert [j["id"] for j in second["items"]] == [created[2]["id"], created[1]["id"]]
    with pytest.raises(UpdateError, match="cursor"):
        state.jobs(first["next_cursor"], status="needs_review")
    assert state.jobs(status="attention")["items"][0]["id"] == created[2]["id"]
    counts = {c["state"]: c["n"] for c in activity(state)["counts"]}
    assert counts == {"queued": 4, "needs_review": 1}
    with state.db() as db:
        db.executemany("INSERT INTO items(job_id,post_id,record_json,state,reason) VALUES(?,?,?,?,?)",
            [(created[0]["id"], i, "{}", "failed" if i in (701, 901) else "stored", "TEST_ISSUE" if i == 901 else None) for i in range(1, 1001)])
    assert [i["post_id"] for i in state.items(created[0]["id"], status="problems")["items"]] == [701, 901]
    assert state.items(created[0]["id"], reason="TEST_ISSUE")["items"][0]["post_id"] == 901


def test_pause_acknowledges_only_after_execution_exits(tmp_path):
    lib, state = setup(tmp_path, "yandere")
    entered, release = threading.Event(), threading.Event()
    class GatedSite(FakeSite):
        def request(self, *args, **kwargs):
            entered.set()
            assert release.wait(10)
            return super().request(*args, **kwargs)
    runner = Runner(state, {"yandere": GatedSite("yandere", [])}, Resources(reserve_bytes=0))
    task = job(state, lib, {"kind": "ids", "ids": [11]})
    with ThreadPoolExecutor(max_workers=1) as pool:
        future = pool.submit(runner.run, task["id"])
        assert entered.wait(10)
        paused = state.action(task["id"], "pause")
        assert paused["state"] == "paused" and paused["execution_active"]
        with pytest.raises(UpdateError, match="still stopping"):
            state.action(task["id"], "resume")
        release.set()
        future.result(timeout=15)
    stopped = state.job(task["id"])
    assert stopped["state"] == "paused" and not stopped["execution_active"]
    assert state.action(task["id"], "resume")["state"] == "queued"


def test_fixed_input_creation_retries_use_same_identity(tmp_path):
    lib, state = setup(tmp_path, "yandere")
    first = state.create_input(lib.info["library_id"])
    identity = "a" * 32
    value = state.create_input(lib.info["library_id"], first["source_version"], {"project": "fixture"}, identity)
    assert state.create_input(lib.info["library_id"], first["source_version"], {"project": "fixture"}, identity)["id"] == value["id"]
    with pytest.raises(UpdateError, match="different provenance"):
        state.create_input(lib.info["library_id"], first["source_version"], {"project": "changed"}, identity)


def test_preview_requires_new_baseline_and_reports_sealed_input_count(tmp_path):
    from studio_lake.updates.__main__ import dispatch
    lib, state = setup(tmp_path, "yandere")
    spec = {"library_id": lib.info["library_id"], "range": {"kind": "new"}}
    with pytest.raises(UpdateError) as error:
        dispatch(state, "preview", {"definition": spec})
    assert error.value.code == "UPDATE_BASELINE_REQUIRED"
    lib.set_setting("update_new_metadata_cursor", 12)
    assert dispatch(state, "preview", {"definition": spec})["known_candidates"] is None
    frozen = state.create_input(lib.info["library_id"])
    state.append_input(frozen["id"], post_ids=[11, 12])
    state.seal_input(frozen["id"])
    spec = {"library_id": lib.info["library_id"], "range": {"kind": "input", "input_id": frozen["id"]}}
    assert dispatch(state, "preview", {"definition": spec})["known_candidates"] == 2


def test_history_pages_are_bounded_by_bytes_with_large_id_lists(tmp_path):
    lib, state = setup(tmp_path, "danbooru")
    ids = list(range(1, 10001))
    created = {job(state, lib, {"kind": "ids", "ids": ids})["id"] for _ in range(50)}
    page = state.jobs(limit=50)
    assert 1 <= len(page["items"]) < 50 and page["next_cursor"]
    found = [r["id"] for r in page["items"]]
    while page["next_cursor"]:
        page = state.jobs(page["next_cursor"], limit=50)
        found.extend(r["id"] for r in page["items"])
    assert len(found) == len(set(found)) == 50 and set(found) == created
