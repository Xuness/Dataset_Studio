"""Physical root migration, frozen versions and deterministic interruption recovery."""

from pathlib import Path
import shutil
import threading
import os
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

import pytest

from test_updates import FakeSite, Images, job, post, setup
from conftest import png
from studio_lake.updates import relocation, locations, dispatch
from studio_lake.updates.runner import Runner
from studio_lake.updates.resources import Resources
from studio_lake.updates.sites import UpdateError
from studio_lake.updates.state import State
from studio_lake.online import Publisher
from studio_lake.util import atomic_json, read_json


def populated(tmp_path):
    lib, state = setup(tmp_path, "yandere")
    data = png("red")
    task = job(state, lib, {"kind": "ids", "ids": [11]}, "original")
    result = Runner(state, {"yandere": FakeSite("yandere", [post("yandere", 11, data)])},
                    Resources(reserve_bytes=0), Images(data)).run(task["id"])
    assert result["state"] == "completed"
    return lib, state


def copy_roots(tmp_path, lib, mode="both"):
    media, index = lib.root, lib.cache
    if mode in {"both", "media"}:
        media = tmp_path / "moved-media"
        shutil.copytree(lib.root, media)
    if mode in {"both", "index"}:
        index = tmp_path / "moved-index"
        shutil.copytree(lib.cache, index)
    return media, index


@pytest.mark.parametrize("mode", ["media", "index", "both"])
def test_move_roots_and_continue_input_update_and_publish(tmp_path, mode):
    lib, state = populated(tmp_path)
    identity = lib.info["library_id"]
    frozen = state.create_input(identity)
    state.append_input(frozen["id"], [11])
    state.seal_input(frozen["id"])
    move = relocation.prepare(state, identity)
    assert move["phase"] == "prepared"
    media, index = copy_roots(tmp_path, lib, mode)
    if mode in {"both", "media"}:
        lib.root.rename(tmp_path / "offline-media")
    if mode in {"both", "index"}:
        lib.cache.rename(tmp_path / "offline-index")
    switched = relocation.apply(state, move["id"], str(media), str(index))
    assert switched["phase"] == "writer_committed"
    assert relocation.apply(state, move["id"], str(media), str(index)) == switched
    assert dispatch.candidates(state, (), 3) == []
    assert relocation.finish(state, move["id"])["phase"] == "complete"
    assert relocation.finish(state, move["id"])["phase"] == "complete"
    current = state.library(identity)
    assert current.root == media and current.cache == index
    assert Path(read_json(media / "online-index.json")["index_root"]) == index
    Publisher(media, index).sync()
    next_input = state.create_input(identity, frozen["source_version"])
    state.append_input(next_input["id"], [11])
    state.seal_input(next_input["id"])
    task = job(state, current, {"kind": "ids", "ids": [12]}, "metadata_only")
    result = Runner(state, {"yandere": FakeSite("yandere", [post("yandere", 12, png("blue"))])},
                    Resources(reserve_bytes=0)).run(task["id"])
    assert result["state"] == "completed"
    assert (media / "plans/updates" / (frozen["id"] + ".ids")).exists()


@pytest.mark.parametrize("fault", ["relocation_verified", "relocation_publisher", "relocation_owner", "relocation_writer", "relocation_unfrozen"])
def test_every_switch_boundary_can_resume(tmp_path, monkeypatch, fault):
    lib, state = populated(tmp_path)
    move = relocation.prepare(state, lib.info["library_id"])
    media, index = copy_roots(tmp_path, lib)
    monkeypatch.setenv("DANBOORU_STORE_FAIL_AT", fault)
    with pytest.raises(RuntimeError, match="injected failure"):
        relocation.apply(state, move["id"], str(media), str(index))
        relocation.finish(state, move["id"])
    assert locations.pending(state, lib.info["library_id"])
    monkeypatch.delenv("DANBOORU_STORE_FAIL_AT")
    reopened = State(state.root)
    relocation.apply(reopened, move["id"], str(media), str(index))
    assert relocation.finish(reopened, move["id"])["phase"] == "complete"


@pytest.mark.parametrize("fault", ["relocation_prepared_files", "relocation_prepared"])
def test_prepare_interrupt_can_resume_or_cancel(tmp_path, monkeypatch, fault):
    lib, state = populated(tmp_path)
    monkeypatch.setenv("DANBOORU_STORE_FAIL_AT", fault)
    with pytest.raises(RuntimeError, match="injected failure"):
        relocation.prepare(state, lib.info["library_id"])
    monkeypatch.delenv("DANBOORU_STORE_FAIL_AT")
    move = relocation.prepare(State(state.root), lib.info["library_id"])
    assert move["phase"] == "prepared"
    assert relocation.cancel(state, move["id"])["phase"] == "cancelled"
    assert not (lib.cache / "LAKE-RELOCATION.json").exists()
    Publisher(lib.root, lib.cache).sync()


def test_prepare_freezes_new_admission_and_waits_for_actual_holders(tmp_path):
    lib, state = setup(tmp_path, "yandere")
    identity = lib.info["library_id"]
    task = job(state, lib, {"kind": "ids", "ids": [11]})
    with locations.access(state, identity):
        move = relocation.prepare(state, identity)
        assert move["phase"] == "draining"
        assert state.claim(task["id"]) is None
        assert dispatch.candidates(state, (), 3) == []
        with pytest.raises(UpdateError, match="relocation"):
            state.create_input(identity)
        assert Runner(state, {"yandere": FakeSite("yandere", [])}).run(task["id"])["state"] == "queued"
    assert relocation.prepare(state, identity)["phase"] == "prepared"
    relocation.cancel(state, move["id"])
    assert len(dispatch.candidates(state, (), 3)) == 1


@pytest.mark.parametrize("damage", ["stale", "identity", "database_identity", "database_site",
                                  "generation", "retention", "spool", "input", "owner"])
def test_bad_copy_cannot_switch_writer(tmp_path, damage):
    lib, state = populated(tmp_path)
    identity = lib.info["library_id"]
    staging = lib.cache / "updates" / ("a" * 32)
    staging.mkdir(parents=True)
    (staging / ("b" * 64 + ".partial")).write_bytes(b"resume")
    frozen = state.create_input(identity)
    state.append_input(frozen["id"], [11])
    state.seal_input(frozen["id"])
    move = relocation.prepare(state, identity)
    media, index = copy_roots(tmp_path, lib)
    if damage in {"stale", "retention", "generation", "database_identity", "database_site"}:
        from studio_lake.online_migrate import connect
        from studio_lake.online_schema import set_state
        db = connect(index / read_json(index / "ONLINE.json")["file"])
        with db:
            set_state(db, **{"stale": {"served_seq": 0}, "retention": {"min_seq": 999},
                             "generation": {"generation": "wrong"}, "database_identity": {"library_id": "wrong"},
                             "database_site": {"site": "wrong"}}[damage])
        db.close()
    elif damage == "identity":
        value = read_json(media / "library.json")
        value["library_id"] = "other"
        atomic_json(media / "library.json", value)
    elif damage == "spool":
        (index / "updates" / ("a" * 32) / ("b" * 64 + ".partial")).write_bytes(b"wrong!")
    elif damage == "input":
        (media / "plans/updates" / (frozen["id"] + ".ids")).unlink()
    else:
        atomic_json(index / "UPDATE-CONTROLLER.json", {"library_id": identity, "root": str(tmp_path / "another-controller")})
    with pytest.raises(UpdateError):
        relocation.apply(state, move["id"], str(media), str(index))
    assert state.lake(identity)["media"] == str(lib.root)
    assert locations.pending(state, identity)


def test_offline_without_prepared_checkpoint_is_not_trusted(tmp_path):
    lib, state = setup(tmp_path, "yandere")
    lib.root.rename(tmp_path / "offline")
    with pytest.raises(UpdateError, match="old lake"):
        relocation.prepare(state, lib.info["library_id"])


def test_old_copy_stays_write_blocked_after_index_move(tmp_path):
    lib, state = populated(tmp_path)
    move = relocation.prepare(state, lib.info["library_id"])
    media, index = copy_roots(tmp_path, lib)
    relocation.apply(state, move["id"], str(media), str(index))
    relocation.finish(state, move["id"])
    with pytest.raises(RuntimeError, match="迁移"):
        Publisher(lib.root, lib.cache).sync()
    with pytest.raises(RuntimeError, match="迁移"):
        with lib.writer_lock():
            pass


def test_stale_cancel_cannot_undo_an_applied_relocation(tmp_path, monkeypatch):
    lib, state = populated(tmp_path)
    move = relocation.prepare(state, lib.info["library_id"])
    media, index = copy_roots(tmp_path, lib)
    entered, release = threading.Event(), threading.Event()
    read = relocation.row
    first = True

    def held(*args):
        nonlocal first
        result = read(*args)
        if threading.current_thread().name.startswith("cancel-stale") and first:
            first = False
            entered.set()
            assert release.wait(10)
        return result

    monkeypatch.setattr(relocation, "row", held)
    with ThreadPoolExecutor(max_workers=1, thread_name_prefix="cancel-stale") as pool:
        future = pool.submit(relocation.cancel, state, move["id"])
        try:
            assert entered.wait(10)
            assert relocation.apply(state, move["id"], str(media), str(index))["phase"] == "writer_committed"
        finally:
            release.set()
        with pytest.raises(UpdateError, match="completed"):
            future.result(timeout=10)
    assert state.lake(lib.info["library_id"])["media"] == str(media)
    assert relocation.finish(state, move["id"])["phase"] == "complete"


@pytest.mark.parametrize("fault", ["relocation_prepared_files", "relocation_prepared", "relocation_verified",
                                  "relocation_publisher", "relocation_owner", "relocation_writer", "relocation_unfrozen"])
def test_hard_process_exit_preserves_relocation_responsibility(tmp_path, fault):
    lib, state = populated(tmp_path)
    prepare = fault in {"relocation_prepared_files", "relocation_prepared"}
    if prepare:
        arguments = ["prepare", lib.info["library_id"]]
    else:
        move = relocation.prepare(state, lib.info["library_id"])
        media, index = copy_roots(tmp_path, lib)
        arguments = ["apply", move["id"], str(media), str(index)]
    environment = {**os.environ, "DANBOORU_STORE_FAIL_AT": fault, "DANBOORU_STORE_HARD_EXIT": "1",
                   "PYTHONPATH": str(Path(__file__).resolve().parents[1] / "src")}
    result = subprocess.run([sys.executable, str(Path(__file__).parent / "helpers/relocate.py"),
                             str(state.root), *arguments], env=environment, capture_output=True, timeout=30)
    assert result.returncode == 91, result.stderr.decode(errors="replace")
    reopened = State(state.root)
    if prepare:
        move = relocation.prepare(reopened, lib.info["library_id"])
        media, index = copy_roots(tmp_path, lib)
    relocation.apply(reopened, move["id"], str(media), str(index))
    assert relocation.finish(reopened, move["id"])["phase"] == "complete"
