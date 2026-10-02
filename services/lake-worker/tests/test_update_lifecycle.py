from update_fixtures import remove_collection_schema
"""Deterministic command races, lake fairness and cancellation ownership/recovery."""

from concurrent.futures import Future, ThreadPoolExecutor
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time

import pytest

from conftest import png
from test_updates import FakeSite, Images, ImageResponse, job, post, setup
from studio_lake.updates import cleanup, commands, dispatch, media, pipeline, spool
from studio_lake.updates.archive import online, reconcile
from studio_lake.updates.resources import Resources
from studio_lake.updates.runner import Runner
from studio_lake.updates.sites import UpdateError
from studio_lake.updates.state import State
from studio_lake.util import atomic_json


def stopped_job(tmp_path):
    lib, state = setup(tmp_path, "yandere")
    task = job(state, lib, {"kind": "ids", "ids": [11]}, "original")
    state.update(task["id"], state="paused", cursor={"pages": 3, "slice_pages": 3, "slice_items": 1})
    with state.db() as db:
        db.execute(
            "INSERT INTO items(job_id,post_id,observation_id,record_json,state,reason,attempts,retry_at) "
            "VALUES(?,11,'observation','{\"id\":11}','failed','image_http_404',2,123)", (task["id"],),
        )
    return lib, state, task["id"]


@pytest.mark.parametrize("command", ["resume", "retry", "replay"])
def test_cancel_wins_after_restart_reads_old_state(tmp_path, monkeypatch, command):
    _, state, identity = stopped_job(tmp_path)
    old = state.job(identity)
    entered, release = threading.Event(), threading.Event()

    def held(*args):
        # This is after the old-state read and before the atomic command transaction.
        reconcile(*args)
        entered.set()
        assert release.wait(10)

    monkeypatch.setattr(commands, "reconcile", held)
    with ThreadPoolExecutor(max_workers=1) as pool:
        future = pool.submit(state.action, identity, command)
        try:
            assert entered.wait(10)
            cancelled = state.action(identity, "cancel")
            assert cancelled["state"] == "cancelled" and cancelled["execution_active"]
            assert cancelled["cleanup"]["phase"] == "pending"
        finally:
            release.set()
        with pytest.raises(UpdateError, match="Cancelled task is closed"):
            future.result(timeout=10)
    closed = state.action(identity, "cancel")
    assert closed["execution"] == old["execution"] == 0
    assert closed["cursor"] == old["cursor"]
    assert state.items(identity)["items"][0]["attempts"] == 2
    assert state.items(identity)["items"][0]["state"] == "failed"


def test_retry_item_reset_and_execution_intent_roll_back_together(tmp_path, monkeypatch):
    _, state, identity = stopped_job(tmp_path)
    old = state.job(identity)
    reset = commands.reset_items

    def fail(db, identity):
        reset(db, identity)
        raise RuntimeError("crash before command commit")

    monkeypatch.setattr(commands, "reset_items", fail)
    with pytest.raises(RuntimeError, match="before command commit"):
        state.action(identity, "retry")
    after = state.job(identity)
    assert (after["state"], after["execution"], after["cursor"]) == (old["state"], 0, old["cursor"])
    item = state.items(identity)["items"][0]
    assert (item["state"], item["attempts"], item["retry_at"]) == ("failed", 2, 123)


@pytest.mark.parametrize("command", ["resume", "retry", "replay"])
def test_duplicate_restart_does_not_create_another_execution(tmp_path, command):
    _, state, identity = stopped_job(tmp_path)
    first = state.action(identity, command)
    again = state.action(identity, command)
    assert first["state"] == again["state"] == "queued"
    assert first["execution"] == again["execution"] == 1
    assert first["cursor"] == again["cursor"]


def test_cancel_between_runner_selection_and_claim_prevents_work(tmp_path, monkeypatch):
    lib, state = setup(tmp_path, "yandere")
    task = job(state, lib, {"kind": "ids", "ids": [11]})
    remote = FakeSite("yandere", [])
    entered, release = threading.Event(), threading.Event()
    claim = state.claim

    def held(identity):
        entered.set()
        assert release.wait(10)
        return claim(identity)

    monkeypatch.setattr(state, "claim", held)
    with ThreadPoolExecutor(max_workers=1) as pool:
        future = pool.submit(Runner(state, {"yandere": remote}).run, task["id"])
        try:
            assert entered.wait(10)
            state.action(task["id"], "cancel")
        finally:
            release.set()
        result = future.result(timeout=15)
    assert result["state"] == "cancelled" and result["cleanup"]["phase"] == "complete"
    assert not remote.calls and not lib.commits()


def test_retry_drains_old_media_receipt_before_resetting_items(tmp_path, monkeypatch):
    lib, state = setup(tmp_path, "yandere")
    data = png("red")
    task = job(state, lib, {"kind": "ids", "ids": [11]}, "original")
    remote = FakeSite("yandere", [post("yandere", 11, data)])

    class Missing(Images):
        def get(self, *args, **kwargs):
            response = ImageResponse(data)
            response.status_code = 404
            return response

    def crash_after_media(control, library, identity):
        if any(json.loads(c["manifest_json"])["source"].get("update_role") == "media" for c in lib.commits()):
            raise RuntimeError("accepted media before control receipt")
        return reconcile(control, library, identity)

    with monkeypatch.context() as patch:
        patch.setattr("studio_lake.updates.runner.reconcile", crash_after_media)
        stopped = Runner(state, {"yandere": remote}, image_http=Missing(data)).run(task["id"])
    assert stopped["state"] == "needs_review" and stopped["counts"] == {"pending": 1}
    queued = state.action(task["id"], "retry")
    assert queued["execution"] == 1
    item = state.items(task["id"])["items"][0]
    assert item["state"] == "pending" and item["attempts"] == 0
    done = Runner(state, {"yandere": remote}, image_http=Images(data)).run(task["id"])
    assert done["state"] == "completed" and done["counts"] == {"stored": 1}
    assert len(remote.calls) == 1
    assert state.items(task["id"])["items"][0]["attempts"] == 1


@pytest.mark.parametrize("stage", ["download", "encode", "ready", "committed", "finish"])
def test_cancel_waits_for_every_stage_and_preserves_accepted_data(tmp_path, monkeypatch, stage):
    lib, state = setup(tmp_path, "yandere")
    data = png("blue")
    task = job(state, lib, {"kind": "ids", "ids": [11]}, "original")
    entered, release = threading.Event(), threading.Event()

    def gate():
        entered.set()
        assert release.wait(15)

    class HeldImage(ImageResponse):
        def iter_content(self, _):
            yield self.data[:20]
            if stage == "download":
                gate()
            yield self.data[20:]

    class HTTP(Images):
        def get(self, *args, **kwargs):
            return HeldImage(data)

    prepare = media.prepare_image
    publish = pipeline.Pipeline.publish
    update = state.update

    def held_prepare(*args, **kwargs):
        if stage == "encode":
            gate()
        return prepare(*args, **kwargs)

    def held_publish(self, *args, **kwargs):
        if stage == "ready" and self.ready and not entered.is_set():
            gate()
        return publish(self, *args, **kwargs)

    def held_reconcile(*args):
        if stage == "committed" and not entered.is_set() and any(
            json.loads(c["manifest_json"])["source"].get("update_role") == "media" for c in lib.commits()
        ):
            gate()
        return reconcile(*args)

    def held_update(identity, **values):
        if stage == "finish" and values.get("state") == "completed":
            gate()
        return update(identity, **values)

    monkeypatch.setattr(media, "prepare_image", held_prepare)
    monkeypatch.setattr(pipeline.Pipeline, "publish", held_publish)
    monkeypatch.setattr("studio_lake.updates.runner.reconcile", held_reconcile)
    monkeypatch.setattr(state, "update", held_update)
    runner = Runner(state, {"yandere": FakeSite("yandere", [post("yandere", 11, data)])}, image_http=HTTP(data))
    with ThreadPoolExecutor(max_workers=1) as pool:
        future = pool.submit(runner.run, task["id"])
        try:
            assert entered.wait(15)
            assert state.action(task["id"], "cancel")["execution_active"]
            cleanup.run(state, task["id"], runner.resources)
            assert state.job(task["id"])["cleanup"]["phase"] == "pending"
            path = spool.directory(lib, task["id"])
            if stage in {"download", "encode", "ready"}:
                assert path.exists() and list(path.iterdir())
        finally:
            release.set()
        closed = future.result(timeout=20)
    assert closed["state"] == "cancelled" and not closed["execution_active"]
    assert closed["cleanup"]["phase"] == "complete"
    assert not spool.directory(lib, task["id"]).exists()
    assert not runner.resources.reservations and not runner.resources.plans
    assert runner.resources.encode_jobs == runner.resources.decode_bytes == 0
    assert all(n == 0 for n in runner.resources.download_jobs.values())
    with online(lib) as (db, _):
        # Already admitted encoding is allowed to finish and publish its valid result.
        assert db.execute("SELECT count(*) FROM objects").fetchone()[0] == int(stage != "download")
    lib.verify(deep=True)
    for command in ("resume", "retry", "replay"):
        with pytest.raises(UpdateError, match="Cancelled task is closed"):
            state.action(task["id"], command)


def cancelled_spool(tmp_path, size=32):
    lib, state, identity = stopped_job(tmp_path)
    path = spool.directory(lib, identity)
    path.mkdir(parents=True)
    partial = path / ("a" * 64 + ".partial")
    partial.write_bytes(b"x" * size)
    state.action(identity, "cancel")
    return lib, state, identity, partial


@pytest.mark.parametrize("point", ["after_cancel_lease_released", "after_cancel_reconciled",
                                  "after_cancel_file_removed", "after_cancel_files_removed"])
def test_cancel_cleanup_survives_process_death_at_each_checkpoint(tmp_path, point):
    lib, state, identity, _ = cancelled_spool(tmp_path)
    command = [sys.executable, str(Path(__file__).parent / "helpers/cancel_cleanup.py"), str(state.root), identity]
    env = {**os.environ, "DANBOORU_STORE_FAIL_AT": point, "DANBOORU_STORE_HARD_EXIT": "1",
           "PYTHONPATH": str(Path(__file__).parents[1] / "src"), "PYTHONDONTWRITEBYTECODE": "1"}
    result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=20)
    assert result.returncode == 91, result.stderr
    reopened = State(state.root)
    assert reopened.job(identity)["state"] == "cancelled"
    assert reopened.job(identity)["cleanup"]["phase"] != "complete"
    cleanup.run(reopened, identity)
    assert reopened.job(identity)["cleanup"]["phase"] == "complete"
    cleanup.run(reopened, identity)
    assert not spool.directory(lib, identity).exists()


def test_locked_file_retries_and_only_removes_owned_task_spool(tmp_path, monkeypatch):
    lib, state, identity, partial = cancelled_spool(tmp_path)
    other = lib.cache / "updates" / ("f" * 32) / ("b" * 64 + ".partial")
    other.parent.mkdir()
    other.write_bytes(b"another task")
    unlink = Path.unlink

    def locked(path, *args, **kwargs):
        if path == partial:
            raise PermissionError("fixture holds file open")
        return unlink(path, *args, **kwargs)

    with monkeypatch.context() as patch:
        patch.setattr(Path, "unlink", locked)
        cleanup.run(state, identity)
    receipt = state.job(identity)["cleanup"]
    assert receipt["phase"] == "reconciled" and receipt["error_code"] == "UPDATE_CLEANUP_IO"
    assert partial.exists() and receipt["retry_at"] > time.time()
    cleanup.run(State(state.root), identity)
    assert state.job(identity)["cleanup"]["phase"] == "complete"
    assert other.read_bytes() == b"another task"


@pytest.mark.parametrize("invalid", ["unknown_file", "nested_directory", "controller", "junction"])
def test_cleanup_rejects_unowned_or_redirected_paths(tmp_path, invalid):
    lib, state, identity, partial = cancelled_spool(tmp_path)
    path = partial.parent
    junction = None
    if invalid == "unknown_file":
        (path / "keep.txt").write_text("unrecognized", encoding="utf-8")
    elif invalid == "nested_directory":
        (path / ("b" * 64 + ".partial")).mkdir()
    elif invalid == "controller":
        atomic_json(lib.cache / "UPDATE-CONTROLLER.json", {"library_id": lib.info["library_id"], "root": str(tmp_path)})
    else:
        outside = tmp_path / "outside"
        path.rename(outside)
        if os.name == "nt":
            result = subprocess.run(["cmd", "/c", "mklink", "/J", str(path), str(outside)], capture_output=True)
            assert result.returncode == 0, result.stderr
        else:
            path.symlink_to(outside, target_is_directory=True)
        junction = path
        partial = outside / partial.name
    try:
        cleanup.run(state, identity)
        assert state.job(identity)["cleanup"]["error_code"] == "UPDATE_CLEANUP_UNSAFE"
        assert state.job(identity)["cleanup"]["phase"] != "complete"
        assert partial.read_bytes() == b"x" * 32
    finally:
        if junction is not None:
            junction.rmdir() if os.name == "nt" else junction.unlink()


def test_cleanup_is_bounded_and_unblocks_cross_lake_spool_budget(tmp_path):
    lib, state, identity, partial = cancelled_spool(tmp_path, 8 * 1024**2)
    resources = Resources(spool_bytes=10 * 1024**2, reserve_bytes=0)
    old = resources.try_reserve(partial.parent, partial.stem, 8 * 1024**2)
    assert old is not None
    old.release()
    other = tmp_path / "other-lake" / "updates" / ("f" * 32)
    assert resources.try_reserve(other, "new", 4 * 1024**2) is None
    # More than one cleanup batch leaves a durable reconciled checkpoint between passes.
    for n in range(260):
        (partial.parent / (f"{n:064x}" + ".json")).write_bytes(b"{}")
    cleanup.run(state, identity, resources)
    assert state.job(identity)["cleanup"]["phase"] == "reconciled"
    cleanup.run(state, identity, resources)
    assert state.job(identity)["cleanup"]["phase"] == "complete"
    assert not spool.directory(lib, identity).exists()
    new = resources.try_reserve(other, "new", 4 * 1024**2)
    assert new is not None
    new.release()
    assert resources.snapshot()["staging_bytes"] == 0


def test_v4_upgrade_enqueues_old_cancelled_tasks_and_retains_paused_files(tmp_path):
    lib, state, identity, partial = cancelled_spool(tmp_path)
    paused = job(state, lib, {"kind": "ids", "ids": [12]})
    state.action(paused["id"], "pause")
    kept = spool.directory(lib, paused["id"])
    kept.mkdir()
    (kept / ("c" * 64 + ".downloaded")).write_bytes(b"resume me")
    with state.db() as db:
        db.execute("DROP TRIGGER cancel_cleanup")
        db.execute("DROP TABLE job_cleanup")
        remove_collection_schema(db)
        db.execute("PRAGMA user_version=4")
    reopened = State(state.root)
    assert reopened.job(identity)["cleanup"]["phase"] == "pending"
    cleanup.run(reopened, identity)
    cleanup.run(reopened, paused["id"])
    assert not partial.exists() and (kept / ("c" * 64 + ".downloaded")).read_bytes() == b"resume me"
    assert reopened.job(paused["id"])["cleanup"] is None
    with reopened.db() as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == 10


def queue(tmp_path, backlog):
    state = State(tmp_path / "control")
    specs = []
    with state.db() as db:
        for lake, count in (("A", backlog), ("B", 1), ("C", 1), ("D", 1)):
            db.execute("INSERT INTO lakes VALUES(?, 'yandere', ?, ?, '')", (lake, str(tmp_path), str(tmp_path)))
            for n in range(count):
                identity = f"{len(specs) + 1:032x}"
                specs.append((identity, lake, identity, "{}", identity, identity))
        db.executemany("INSERT INTO jobs(id,lake_id,request_key,definition,state,cursor,created_at,updated_at) "
                       "VALUES(?,?,?,?,'queued','{}',?,?)", specs)
    return state


@pytest.mark.parametrize("backlog", [100, 1000])
def test_per_lake_heads_survive_backlog_delayed_retry_and_failed_future(tmp_path, backlog):
    state = queue(tmp_path, backlog)
    runner = Runner(state)
    assert [r["lake_id"] for r in dispatch.candidates(state, {}, 3)] == ["A", "B", "C"]
    assert [r["lake_id"] for r in dispatch.candidates(state, {"A": None}, 2)] == ["B", "C"]
    submitted = []
    futures = {}

    class Pool:
        def submit(self, function, identity):
            assert function == runner.run_slice
            lake = state.job(identity)["lake_id"]
            submitted.append(lake)
            futures[lake] = Future()
            return futures[lake]

    class Clock:
        turn = 0

        def is_set(self):
            return self.turn >= 3

        def wait(self, _):
            self.turn += 1
            if self.turn == 1:
                assert submitted == ["A", "B", "C"]
                futures["B"].set_exception(RuntimeError("unexpected executor exception"))
            elif self.turn == 2:
                assert submitted == ["A", "B", "C", "D"]
                with state.db() as db:
                    b = db.execute("SELECT * FROM jobs WHERE lake_id='B'").fetchone()
                    assert b["state"] == "waiting_retry" and b["error_code"] == "UPDATE_EXECUTOR_FAILED"
                    db.execute("UPDATE jobs SET retry_at=0 WHERE lake_id='B'")
                    db.execute("UPDATE jobs SET state='completed' WHERE lake_id='D'")
                futures["D"].set_result(None)

    runner.stop = Clock()
    runner.schedule(Pool(), False)
    assert submitted == ["A", "B", "C", "D", "B"]
    assert not futures["A"].done() and not futures["C"].done()


def test_failed_future_cannot_overwrite_cancel_or_new_execution(tmp_path):
    state = queue(tmp_path, 1)
    a, b = dispatch.candidates(state, {}, 2)
    state.action(a["id"], "cancel")
    dispatch.failed(state, a["id"], 0)
    with state.db() as db:
        db.execute("UPDATE jobs SET execution=1 WHERE id=?", (b["id"],))
    dispatch.failed(state, b["id"], 0)
    assert state.job(a["id"])["state"] == "cancelled"
    assert state.job(b["id"])["state"] == "queued"


def test_cleanup_gets_idle_lake_before_next_backlogged_execution(tmp_path):
    state = queue(tmp_path, 1000)
    first = dispatch.candidates(state, (), 1)[0]
    state.action(first["id"], "cancel")
    assert dispatch.next_cleanup(state, {"A"}) is None
    assert dispatch.next_cleanup(state)["job_id"] == first["id"]
    runner = Runner(state)
    submitted = []

    class Pool:
        def submit(self, function, identity):
            submitted.append((function.__name__, state.job(identity)["lake_id"]))
            return Future()

    class Stop:
        stopped = False

        def is_set(self):
            return self.stopped

        def wait(self, _):
            self.stopped = True

    runner.stop = Stop()
    runner.schedule(Pool(), False)
    assert submitted == [("cleanup", "A"), ("run_slice", "B"), ("run_slice", "C"), ("run_slice", "D")]


def test_fourth_lake_gets_next_slot_before_old_backlog_and_order_survives_restart(tmp_path):
    state = queue(tmp_path, 1000)
    runner = Runner(state)
    submitted, futures = [], {}

    class Pool:
        def submit(self, function, identity):
            lake = state.job(identity)["lake_id"]
            submitted.append(lake)
            futures[lake] = (Future(), identity)
            return futures[lake][0]

    class Stop:
        turn = 0

        def is_set(self):
            return self.turn >= 2

        def wait(self, _):
            self.turn += 1
            if self.turn == 1:
                assert submitted == ["A", "B", "C"]
                future, identity = futures["A"]
                state.update(identity, state="completed")
                future.set_result(None)

    runner.stop = Stop()
    runner.schedule(Pool(), False)
    assert submitted == ["A", "B", "C", "D"]
    assert not futures["B"][0].done() and not futures["C"][0].done()
    reopened = State(state.root)
    assert dispatch.candidates(reopened, {"B", "C"}, 1)[0]["lake_id"] == "A"
    dispatch.submitted(reopened, "A")
    assert dispatch.candidates(reopened, {"B", "C"}, 1)[0]["lake_id"] == "D"
