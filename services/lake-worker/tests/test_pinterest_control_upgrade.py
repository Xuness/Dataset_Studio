"""Repair databases opened while the v15-v18 development migrations were evolving."""

from concurrent.futures import ThreadPoolExecutor
import sqlite3
import threading

import pytest

from studio_lake.pinterest.service import Service
from studio_lake.sqlite_control import Connection
from studio_lake.updates.sites import UpdateError
from studio_lake.updates.state import SCHEMA_VERSION, State
from studio_lake.util import FileLock
from test_pinterest_discovery import collection, listed, pages
from test_update_schema import rows
from update_fixtures import remove_pinterest_pagination


def legacy_control(tmp_path, layout="missing"):
    fixture = collection(tmp_path, lambda *_: pages([listed("123"), listed("124")]),
                         budget=dict(api_requests=1, admitted_pins=1))
    assert fixture.run()["state"] == "waiting_budget"
    with fixture.state.db() as db:
        remove_pinterest_pagination(db)
        db.execute("INSERT INTO settings VALUES('unrelated-setting','retained')")
        db.execute("INSERT INTO credentials VALUES('yandere',?,7)", (b"opaque fixture credential",))
        db.execute("INSERT INTO schedules VALUES('old-schedule','{}',60,123,0,4,NULL)")
        db.execute("UPDATE pinterest_jobs SET state='needs_review',error_code='PINTEREST_EXECUTOR_FAILED' WHERE id=?",
                   (fixture.job["id"],))
        if layout != "complete":
            db.execute("DROP TRIGGER pinterest_admitted_insert")
            db.execute("DROP INDEX pinterest_jobs_state")
            db.execute("DROP INDEX pinterest_tasks_scan")
            if layout == "missing":
                db.execute("DROP TABLE pinterest_admitted_counts")
            else:
                db.execute("UPDATE pinterest_admitted_counts SET n=999")
        db.execute("PRAGMA user_version=18")
    return fixture


@pytest.mark.parametrize("layout", ["missing", "stale", "complete"])
def test_v18_upgrade_preserves_jobs_and_backfills_derived_counts(tmp_path, layout):
    fixture = legacy_control(tmp_path, layout)
    before = rows(fixture.state)
    fixture.state = State(fixture.state.root)
    fixture.service = Service(fixture.state)
    after = rows(fixture.state)
    assert {name: after[name] for name in before if name not in ("pinterest_admitted_counts", "pinterest_streams")} == {
        name: value for name, value in before.items() if name not in ("pinterest_admitted_counts", "pinterest_streams")}
    assert [r[:-2] for r in after["pinterest_streams"]] == before["pinterest_streams"]
    with fixture.state.db() as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == SCHEMA_VERSION
        assert db.execute("SELECT job_id,kind,n FROM pinterest_admitted_counts ORDER BY job_id,kind").fetchall() == db.execute(
            "SELECT job_id,kind,count(*) FROM pinterest_admitted GROUP BY job_id,kind ORDER BY job_id,kind").fetchall()
        assert {r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE name IN "
            "('pinterest_admitted_insert','pinterest_jobs_state','pinterest_tasks_scan')")} == {
                "pinterest_admitted_insert", "pinterest_jobs_state", "pinterest_tasks_scan"}
        original = fixture.service.row(fixture.job["id"], db)
    job = fixture.service.job(fixture.job["id"])
    assert job["state"] == "needs_review" and job["totals"]["admitted_pins"] == 1
    assert job["totals"]["admitted_boards"] == 1 and job["api_requests"] == 1
    assert fixture.service.create(dict(request_key=original["request_key"], definition=job["definition"]))["id"] == job["id"]
    assert rows(State(fixture.state.root)) == after
    backups = list((fixture.state.root / "backups").glob("control-v18-*.sqlite"))
    assert len(backups) == 1
    with sqlite3.connect(backups[0]) as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == 18
        assert bool(db.execute("SELECT 1 FROM sqlite_master WHERE name='pinterest_admitted_counts'").fetchone()) == (layout != "missing")
    # Resume the existing task and exhaust its original budget before granting a new round.
    fixture.service.action(dict(job_id=job["id"], expected_revision=job["revision"], action="resume"))
    waiting = fixture.run()
    assert waiting["state"] == "waiting_budget" and waiting["totals"]["admitted_pins"] == 1
    fixture.service.action(dict(job_id=job["id"], expected_revision=waiting["revision"], action="continue"))
    completed = fixture.run()
    assert completed["state"] == "completed" and completed["totals"]["admitted_pins"] == 2
    assert completed["api_requests"] == 1 and len(fixture.media_calls) == 1
    with fixture.state.db() as db:
        db.execute("INSERT INTO pinterest_admitted VALUES(?,'pin','124','duplicate') ON CONFLICT DO NOTHING", (job["id"],))
    assert fixture.service.job(job["id"])["totals"]["admitted_pins"] == 2


@pytest.mark.parametrize("owner", ["runner", "execution"])
def test_v18_upgrade_waits_for_existing_owners(tmp_path, owner):
    fixture = legacy_control(tmp_path)
    lock = (FileLock(fixture.state.root / "runner.lock", timeout=0) if owner == "runner"
            else fixture.service.execution_lock(fixture.job["id"]))
    before = rows(fixture.state)
    with lock, pytest.raises(UpdateError, match="旧版"):
        State(fixture.state.root)
    assert rows(fixture.state) == before
    assert not list((fixture.state.root / "backups").glob("control-v18-*.sqlite"))
    assert Service(State(fixture.state.root)).job(fixture.job["id"])["state"] == "needs_review"


def test_v18_repair_rolls_back_schema_and_version_together(tmp_path, monkeypatch):
    fixture = legacy_control(tmp_path)
    before = rows(fixture.state)
    execute = Connection.execute
    def interrupted(self, sql, args=()):
        if sql == f"PRAGMA user_version={SCHEMA_VERSION}":
            raise OSError("fixture stopped before migration commit")
        return execute(self, sql, args)
    with monkeypatch.context() as patch:
        patch.setattr(Connection, "execute", interrupted)
        with pytest.raises(OSError, match="migration commit"):
            State(fixture.state.root)
    assert rows(fixture.state) == before
    with fixture.state.db() as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == 18
        assert not db.execute("SELECT 1 FROM sqlite_master WHERE name='pinterest_jobs_state'").fetchone()
    assert Service(State(fixture.state.root)).job(fixture.job["id"])["totals"]["admitted_pins"] == 1


def test_concurrent_v18_opens_share_one_repair(tmp_path):
    fixture = legacy_control(tmp_path)
    gate = threading.Barrier(4)
    def reopen(_):
        gate.wait(timeout=10)
        return Service(State(fixture.state.root)).job(fixture.job["id"])["totals"]["admitted_pins"]
    with ThreadPoolExecutor(max_workers=4) as pool:
        assert list(pool.map(reopen, range(4))) == [1] * 4
    assert len(list((fixture.state.root / "backups").glob("control-v18-*.sqlite"))) == 1
