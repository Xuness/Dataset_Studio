"""Physical root migration, frozen versions and deterministic interruption recovery."""

from pathlib import Path
import json
import shutil
import sqlite3
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
                                  "generation", "retention", "publication", "archive_head", "owner"])
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
    elif damage == "publication":
        from studio_lake.online_migrate import connect
        db = connect(index / read_json(index / "ONLINE.json")["file"])
        with db:
            db.execute("UPDATE publications SET batch_id='wrong' WHERE seq=(SELECT max(seq) FROM publications)")
        db.close()
    elif damage == "archive_head":
        db = sqlite3.connect(media / "journal.sqlite")
        with db:
            db.execute("UPDATE commits SET batch_id='wrong' WHERE seq=(SELECT max(seq) FROM commits)")
        db.close()
    else:
        atomic_json(index / "UPDATE-CONTROLLER.json", {"library_id": identity, "root": str(tmp_path / "another-controller")})
    with pytest.raises(UpdateError):
        relocation.apply(state, move["id"], str(media), str(index))
    assert state.lake(identity)["media"] == str(lib.root)
    assert locations.pending(state, identity)


@pytest.mark.parametrize("mode", ["media", "index", "both"])
def test_already_moved_directories_can_reconnect_without_restoring_old_paths(tmp_path, mode):
    lib, state = populated(tmp_path)
    media, index = lib.root, lib.cache
    if mode in {"media", "both"}:
        media = lib.root.rename(tmp_path / "moved-media")
    if mode in {"index", "both"}:
        index = lib.cache.rename(tmp_path / "moved-index")
    move = relocation.prepare(state, lib.info["library_id"])
    assert move["phase"] == "prepared"
    assert lib.root.exists() == (mode == "index")
    assert lib.cache.exists() == (mode == "media")
    relocation.apply(state, move["id"], str(media), str(index))
    assert relocation.finish(state, move["id"])["phase"] == "complete"
    current = state.library(lib.info["library_id"])
    assert (current.root, current.cache) == (media, index)
    Publisher(media, index).sync()


def test_relocation_never_enumerates_payloads_reads_historical_manifests_or_recovers(tmp_path, monkeypatch):
    from studio_lake.library import Library

    lib, state = populated(tmp_path)
    media, index = copy_roots(tmp_path, lib)
    roots = (lib.root, lib.cache, media, index)
    original_iterdir, original_connect = Path.iterdir, sqlite3.connect

    def bounded_iterdir(path):
        assert not any(path == root or root in path.parents for root in roots), path
        return original_iterdir(path)

    def bounded_connect(*args, **kwargs):
        db = original_connect(*args, **kwargs)
        db.set_authorizer(lambda action, table, column, *_:
                          sqlite3.SQLITE_DENY if action == sqlite3.SQLITE_READ
                          and table == "commits" and column == "manifest_json" else sqlite3.SQLITE_OK)
        return db

    def no_recovery(*args, **kwargs):
        pytest.fail("Location changes must not run archive recovery")

    monkeypatch.setattr(Path, "iterdir", bounded_iterdir)
    monkeypatch.setattr(sqlite3, "connect", bounded_connect)
    monkeypatch.setattr(Library, "recover", no_recovery)
    move = relocation.prepare(state, lib.info["library_id"])
    relocation.apply(state, move["id"], str(media), str(index))
    assert relocation.finish(state, move["id"])["phase"] == "complete"
    assert not (state.root / "relocations").exists()


@pytest.mark.parametrize("phase", ["draining", "prepared", "verified"])
def test_old_interrupted_relocations_resume_without_inventory_scan(tmp_path, phase):
    lib, state = populated(tmp_path)
    move = relocation.prepare(state, lib.info["library_id"])
    media, index = copy_roots(tmp_path, lib)
    old = json.loads(relocation.row(state, move["id"])["checkpoint"])
    old.pop("version")
    old.pop("archive_batch")
    old["archive_digest"] = "legacy-full-journal-digest"
    with state.db() as db:
        db.execute("UPDATE lake_relocations SET phase=?,checkpoint=?,media_root=?,index_root=? WHERE id=?",
                   (phase, None if phase == "draining" else json.dumps(old),
                    str(media) if phase == "verified" else None,
                    str(index) if phase == "verified" else None, move["id"]))
    # Neither require nor open an old, possibly interrupted inventory database.
    evidence = state.root / "relocations" / (move["id"] + ".sqlite")
    evidence.parent.mkdir()
    evidence.write_bytes(b"interrupted old inventory")
    if phase == "draining":
        assert relocation.prepare(state, lib.info["library_id"])["phase"] == "prepared"
    relocation.apply(state, move["id"], str(media), str(index))
    assert relocation.finish(state, move["id"])["phase"] == "complete"
    assert evidence.read_bytes() == b"interrupted old inventory"


def test_cancel_after_files_moved_only_releases_pending_handoff(tmp_path):
    lib, state = populated(tmp_path)
    before = state.lake(lib.info["library_id"])
    lib.root.rename(tmp_path / "moved-media")
    lib.cache.rename(tmp_path / "moved-index")
    move = relocation.prepare(state, lib.info["library_id"])
    assert relocation.cancel(state, move["id"])["phase"] == "cancelled"
    assert state.lake(lib.info["library_id"]) == before
    assert not lib.root.exists() and not lib.cache.exists()


def test_cancel_after_preparing_and_moving_index_can_reconnect_again(tmp_path):
    lib, state = populated(tmp_path)
    identity = lib.info["library_id"]
    first = relocation.prepare(state, identity)
    index = lib.cache.rename(tmp_path / "moved-index")
    assert relocation.cancel(state, first["id"])["phase"] == "cancelled"
    assert read_json(index / "LAKE-RELOCATION.json")["id"] == first["id"]
    second = relocation.prepare(state, identity)
    assert second["id"] != first["id"]
    relocation.apply(state, second["id"], str(lib.root), str(index))
    assert relocation.finish(state, second["id"])["phase"] == "complete"
    assert not (index / "LAKE-RELOCATION.json").exists()


def test_unknown_relocation_marker_cannot_be_replaced(tmp_path):
    lib, state = populated(tmp_path)
    identity = lib.info["library_id"]
    index = lib.cache.rename(tmp_path / "moved-index")
    owner = {"id": "f" * 32, "library_id": identity}
    atomic_json(index / "LAKE-RELOCATION.json", owner)
    move = relocation.prepare(state, identity)
    with pytest.raises(UpdateError, match="Another relocation"):
        relocation.apply(state, move["id"], str(lib.root), str(index))
    assert read_json(index / "LAKE-RELOCATION.json") == owner


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
