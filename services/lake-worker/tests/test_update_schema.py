"""Upgrade controls that a development runner opened before v5 was complete."""

from concurrent.futures import ThreadPoolExecutor
import threading

import pytest

from studio_lake.sqlite_control import Connection
from studio_lake.updates import dispatch
from studio_lake.updates.sites import UpdateError
from studio_lake.updates.state import SCHEMA_VERSION, State
from studio_lake.util import FileLock
from update_fixtures import remove_collection_schema


def legacy_control(tmp_path, version, missing=True):
    state = State(tmp_path / "control")
    with state.db() as db:
        remove_collection_schema(db)
        for lake in ("A", "B"):
            db.execute("INSERT INTO lakes VALUES(?,'yandere',?,?,'fixture')",
                       (lake, str(tmp_path / lake), str(tmp_path / (lake + "-index"))))
            db.execute(
                "INSERT INTO jobs(id,lake_id,request_key,definition,state,cursor,created_at,updated_at) "
                "VALUES(?,?,?,'{}','queued','{\"pages\":3}',?,?)", (lake.lower() * 32, lake, lake, lake, lake),
            )
        db.execute("INSERT INTO credentials VALUES('yandere',?,4)", (b"opaque fixture credential",))
        db.execute("INSERT INTO schedules VALUES('saved','{}',60,123,0,4,NULL)")
        db.execute("INSERT INTO settings VALUES('fixture','retained')")
        db.execute("INSERT INTO inputs VALUES('fixed','A','sealed','frozen','{}','fixture',0,'digest')")
        db.execute("INSERT INTO input_ids VALUES('fixed',11)")
        db.execute("INSERT INTO items(job_id,post_id,record_json,state) VALUES(?,11,'{}','stored')", ("a" * 32,))
        db.execute("INSERT INTO lake_dispatch VALUES('A',27)")
        if missing:
            db.execute("DROP TABLE lake_dispatch")
        if version == 5:
            db.execute("DROP TABLE lake_relocations")
        db.execute(f"PRAGMA user_version={version}")
    return state


def rows(state):
    with state.db() as db:
        tables = db.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'").fetchall()
        return {name: sorted([tuple(r) for r in db.execute(f'SELECT * FROM "{name}"')], key=repr)
                for (name,) in tables}


@pytest.mark.parametrize("version,missing", [(5, True), (5, False), (6, True), (6, False), (7, False)])
def test_dispatch_upgrade_retains_control_data_and_existing_service_order(tmp_path, version, missing):
    state = legacy_control(tmp_path, version, missing)
    before = rows(state)
    reopened = State(state.root)
    # This is the real scheduler query that killed the previously healthy handshake.
    assert [r["lake_id"] for r in dispatch.candidates(reopened, (), 3)] == (["A", "B"] if missing else ["B", "A"])
    after = rows(reopened)
    assert {name: after[name] for name in before} == before
    dispatch.submitted(reopened, "A")
    assert [r["lake_id"] for r in dispatch.candidates(reopened, (), 3)] == ["B", "A"]
    with reopened.db() as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == SCHEMA_VERSION
    durable = rows(reopened)
    assert rows(State(state.root)) == durable


@pytest.mark.parametrize("owner", ["runner", "execution"])
@pytest.mark.parametrize("version", [6, 7])
def test_dispatch_upgrade_waits_for_actual_old_owners(tmp_path, owner, version):
    state = legacy_control(tmp_path, version, missing=version == 6)
    lock = FileLock(state.root / "runner.lock", timeout=0) if owner == "runner" else state.execution_lock("a" * 32)
    with lock:
        with pytest.raises(UpdateError, match="旧版"):
            State(state.root)
        with state.db() as db:
            assert db.execute("PRAGMA user_version").fetchone()[0] == version
            assert bool(db.execute("PRAGMA table_info(lake_dispatch)").fetchall()) == (version == 7)
            assert not db.execute("PRAGMA table_info(collection_jobs)").fetchall()
    assert len(dispatch.candidates(State(state.root), (), 3)) == 2


@pytest.mark.parametrize("version", [6, 7])
def test_dispatch_upgrade_rolls_back_schema_and_version_together(tmp_path, monkeypatch, version):
    state = legacy_control(tmp_path, version, missing=version == 6)
    before = rows(state)
    execute = Connection.execute

    def interrupted(self, sql, args=()):
        if sql == f"PRAGMA user_version={version + 1}":
            raise OSError("fixture stopped before migration commit")
        return execute(self, sql, args)

    with monkeypatch.context() as patch:
        patch.setattr(Connection, "execute", interrupted)
        with pytest.raises(OSError, match="migration commit"):
            State(state.root)
    assert rows(state) == before
    with state.db() as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == version
    assert len(dispatch.candidates(State(state.root), (), 3)) == 2


def test_concurrent_dispatch_repairs_share_one_upgrade(tmp_path):
    state = legacy_control(tmp_path, 6)
    before = rows(state)
    gate = threading.Barrier(4)

    def reopen(_):
        gate.wait(timeout=10)
        return len(dispatch.candidates(State(state.root), (), 3))

    with ThreadPoolExecutor(max_workers=4) as pool:
        assert list(pool.map(reopen, range(4))) == [2] * 4
    after = rows(state)
    assert {name: after[name] for name in before} == before
