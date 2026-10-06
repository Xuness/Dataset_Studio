"""Coverage certificates reduce requests without inferring missing data or a newer capture time."""

import time

import pytest

from studio_lake.updates.archive import online, reconcile
from studio_lake.updates import query_cache
from studio_lake.updates.state import SCHEMA_VERSION, State
from studio_lake.updates.sites import UpdateError
from studio_lake.util import FileLock
from test_tag_collection import setup, scope
from test_updates import job


def reusable_scope(**kwargs):
    return {**scope(**kwargs), "refresh": {"mode": "missing_or_stale", "max_age_hours": 24}}


def test_new_and_queries_share_anchor_capture_and_keep_observation_time(tmp_path):
    lib, state, remote, images, runner = setup(tmp_path)
    first = job(state, lib, reusable_scope(all_=["a", "b"], none=["blocked"]))
    assert runner.run(first["id"])["state"] == "completed"
    fetched = len(remote.calls)
    with online(lib) as (db, _):
        before = list(db.execute("SELECT observation_id,observed_at FROM observations ORDER BY row_id"))
    second = job(state, lib, reusable_scope(all_=["a", "c"]), "original")
    done = runner.run(second["id"])
    assert done["state"] == "completed", done
    assert len(remote.calls) == fetched
    assert done["counts"] == {"excluded": 1, "stored": 2}
    assert done["cursor"]["metadata_reused_records"] == 3
    assert images.calls == 2
    with online(lib) as (db, _):
        assert list(db.execute("SELECT observation_id,observed_at FROM observations ORDER BY row_id")) == before
    third = job(state, lib, reusable_scope(all_=["b", "c"]), "original")
    done = runner.run(third["id"])
    assert done["state"] == "completed"
    assert len(remote.calls) > fetched  # Seeing some B through A was never a B coverage proof.
    assert done["counts"] == {"excluded": 1, "reused": 1, "stored": 1}
    assert images.calls == 3
    assert lib.setting("update_new_metadata_cursor") is None


def test_partial_scan_only_skips_its_proven_prefix(tmp_path):
    lib, state, remote, _, runner = setup(tmp_path, page_size=2)
    first = job(state, lib, reusable_scope(all_=["a", "b"]), page_budget=1)
    paused = runner.run(first["id"])
    assert paused["state"] == "paused" and not paused["cursor"]["metadata_complete"]
    fetched = len(remote.calls)
    second = job(state, lib, reusable_scope(all_=["a", "c"]))
    done = runner.run(second["id"])
    assert done["state"] == "completed"
    assert done["cursor"]["metadata_reused_records"] == 2
    assert len(remote.calls) > fetched
    assert all("id:51.." in p["tags"] or "id:5001.." in p["tags"] for p in remote.calls[fetched:])


@pytest.mark.parametrize("reason", ["expired", "credentials", "forced"])
def test_stale_or_incomparable_coverage_is_refetched(tmp_path, reason):
    lib, state, remote, _, runner = setup(tmp_path)
    first = job(state, lib, reusable_scope(all_=["a"]))
    assert runner.run(first["id"])["state"] == "completed"
    fetched = len(remote.calls)
    second_scope = reusable_scope(all_=["a"])
    if reason == "expired":
        with state.db() as db:
            db.execute("UPDATE discovery_pages SET observed_unix=?", (time.time() - 86401,))
    elif reason == "credentials":
        state.clear_credentials("danbooru")
    else:
        second_scope["refresh"]["mode"] = "all"
    second = job(state, lib, second_scope)
    assert runner.run(second["id"])["state"] == "completed"
    assert len(remote.calls) > fetched


def test_overlapping_range_reuses_only_the_certified_interval(tmp_path):
    lib, state, remote, _, runner = setup(tmp_path)
    first = job(state, lib, {**reusable_scope(all_=["a"], end=5001), "start_id": 10})
    assert runner.run(first["id"])["state"] == "completed"
    count = len(remote.calls)
    second = job(state, lib, {**reusable_scope(all_=["a"], end=9001), "start_id": 20})
    done = runner.run(second["id"])
    assert done["state"] == "completed"
    assert done["cursor"]["metadata_reused_records"] == 2
    assert all("id:5001.." in p["tags"] for p in remote.calls[count:])


def test_automatic_upper_reuses_only_a_real_source_head(tmp_path):
    lib, state, remote, _, runner = setup(tmp_path)
    bounded = job(state, lib, reusable_scope(all_=["a"], end=51))
    assert runner.run(bounded["id"])["state"] == "completed"
    count = len(remote.calls)
    unbounded = job(state, lib, reusable_scope(all_=["a"], end=None))
    first = runner.run(unbounded["id"])
    assert first["state"] == "completed"
    assert first["cursor"]["upper"] == 9001
    assert len(remote.calls) > count
    count = len(remote.calls)
    repeat = job(state, lib, reusable_scope(all_=["a", "c"], end=None))
    assert runner.run(repeat["id"])["state"] == "completed"
    assert len(remote.calls) == count


def test_reuse_receipt_replays_after_control_failure_without_network(tmp_path, monkeypatch):
    lib, state, remote, _, runner = setup(tmp_path)
    first = job(state, lib, reusable_scope(all_=["a"]))
    assert runner.run(first["id"])["state"] == "completed"
    count = len(remote.calls)
    second = job(state, lib, reusable_scope(all_=["a", "c"]))
    from studio_lake.sqlite_control import Connection
    execute = Connection.execute
    failed = False

    def interrupt(connection, sql, args=()):
        nonlocal failed
        if not failed and sql.startswith("INSERT INTO items") and args and args[0] == second["id"]:
            failed = True
            raise OSError("control write interrupted")
        return execute(connection, sql, args)

    with monkeypatch.context() as patch:
        patch.setattr(Connection, "execute", interrupt)
        assert runner.run(second["id"])["state"] == "needs_review"
    state.action(second["id"], "resume")
    done = runner.run(second["id"])
    assert done["state"] == "completed"
    assert done["counts"] == {"excluded": 1, "metadata": 2}
    reconcile(state, lib, second["id"])
    assert state.job(second["id"])["counts"] == done["counts"]
    assert len(remote.calls) == count


def test_changing_access_context_cannot_mix_a_paused_scan(tmp_path):
    lib, state, _, _, runner = setup(tmp_path, page_size=2)
    task = job(state, lib, reusable_scope(all_=["a"]), page_budget=1)
    assert runner.run(task["id"])["state"] == "paused"
    state.clear_credentials("danbooru")
    state.action(task["id"], "resume")
    done = runner.run(task["id"])
    assert done["state"] == "needs_review" and done["error_code"] == "UPDATE_SCOPE_CHANGED"


def test_no_posts_above_lower_bound_finishes_after_head_probe(tmp_path):
    lib, state, remote, _, runner = setup(tmp_path)
    task = job(state, lib, {**reusable_scope(all_=["a"], end=None), "start_id": 10000})
    assert runner.run(task["id"])["state"] == "completed"
    assert len(remote.calls) == 1


def test_query_cache_migration_is_atomic_and_fences_active_owners(tmp_path, monkeypatch):
    state = State(tmp_path / "control")
    with state.db() as db:
        db.execute("DROP TABLE discovery_pages")
        db.execute("DROP TABLE source_access_epochs")
        db.execute("PRAGMA user_version=12")
    with FileLock(state.root / "runner.lock", timeout=0):
        with pytest.raises(UpdateError, match="旧版"):
            State(state.root)
    migrate = query_cache.migrate
    def fail(db):
        migrate(db)
        raise OSError("before version commit")
    with monkeypatch.context() as patch:
        patch.setattr(query_cache, "migrate", fail)
        with pytest.raises(OSError, match="version commit"):
            State(state.root)
    with state.db() as db:
        assert not db.execute("SELECT 1 FROM sqlite_master WHERE name='discovery_pages'").fetchone()
        assert db.execute("PRAGMA user_version").fetchone()[0] == 12
    upgraded = State(state.root)
    with upgraded.db() as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == SCHEMA_VERSION
        assert db.execute("SELECT count(*) FROM source_access_epochs").fetchone()[0] == 3


def test_different_capture_times_cannot_invent_a_tag_intersection(tmp_path):
    lib, state, remote, images, runner = setup(tmp_path)
    remote.records = remote.records[:1]
    remote.records[0]["tag_string"] = "a"
    first = job(state, lib, reusable_scope(all_=["a"], end=12))
    assert runner.run(first["id"])["state"] == "completed"
    remote.records[0]["tag_string"] = "b"
    second = job(state, lib, reusable_scope(all_=["b"], end=12))
    assert runner.run(second["id"])["state"] == "completed"
    count = len(remote.calls)
    intersection = job(state, lib, reusable_scope(all_=["a", "b"], end=12), "original")
    done = runner.run(intersection["id"])
    assert done["state"] == "completed" and done["counts"] == {"excluded": 1}
    assert len(remote.calls) == count and images.calls == 0


def test_cached_page_can_resume_inside_its_original_membership(tmp_path):
    lib, state, remote, _, runner = setup(tmp_path)
    first = job(state, lib, reusable_scope(all_=["a"]))
    assert runner.run(first["id"])["state"] == "completed"
    count = len(remote.calls)
    repeated = job(state, lib, reusable_scope(all_=["a", "c"]), item_budget=1)
    for _ in range(6):
        done = runner.run(repeated["id"])
        if done["state"] != "paused":
            break
        state.action(repeated["id"], "resume")
    assert done["state"] == "completed" and done["counts"] == {"metadata": 2, "excluded": 1}
    assert done["cursor"]["metadata_reused_records"] == 3
    assert len(remote.calls) == count
