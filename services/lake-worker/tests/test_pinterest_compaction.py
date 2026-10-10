"""Physical consolidation preserves every accepted byte and all logical versions."""

import os
from pathlib import Path
import subprocess
import sys
import threading
import uuid

import pytest

from test_pinterest_batching import Crash, Distinct
from studio_lake.online_storage import connect
from studio_lake.pinterest import receipts
from studio_lake.pinterest.lake import compaction, library, maintenance, online, storage
from studio_lake.util import IntegrityError, file_hash, read_json


def fixture_at(root, count=4):
    fixture = Distinct(root, count=count)
    assert fixture.run()["state"] == "completed"
    return fixture, fixture.service.library(fixture.lake["library_id"])


def files(lib):
    return {p.relative_to(lib.root).as_posix(): file_hash(p) for p in (lib.root / "segments").glob("*/*") if p.is_file()}


def proofs(lib):
    db = connect(lib.cache / "online.sqlite")
    try:
        seq = int(db.execute("SELECT value FROM online_state WHERE key='served_seq'").fetchone()[0])
        return seq, maintenance.signatures(db, seq)
    finally:
        db.close()


def test_consolidation_keeps_all_bytes_receipts_fixed_versions_and_late_dedup(tmp_path):
    fixture, lib = fixture_at(tmp_path, count=100)
    before, pointer = files(lib), read_json(lib.cache / "ONLINE.json")
    seq, signatures = proofs(lib)
    with lib.journal() as db:
        journal = [tuple(r) for r in db.execute("SELECT * FROM commits ORDER BY seq")]
        replay = [tuple(r) for r in db.execute("SELECT * FROM pinterest_receipts ORDER BY receipt_id")]
    held = connect(lib.cache / "online.sqlite")
    held.execute("INSERT INTO leases VALUES('fixed',3,NULL,'project','fixed-version')")
    held.execute("BEGIN")
    frozen = held.execute("SELECT sha256,pack_path,offset,length FROM objects WHERE first_seq<=3 ORDER BY sha256").fetchall()
    try:
        result = compaction.compact(lib, force=True)
        assert result["state"] == "cleaned" and result["removed_files"] == len(before)
        assert result["references"]["retained_leases"] == 1
        assert list((lib.root / "segments").iterdir()) == []
        assert len(list((lib.root / "containers").glob("*/*.tar"))) == 1
        assert held.execute("SELECT sha256,pack_path,offset,length FROM objects WHERE first_seq<=3 ORDER BY sha256").fetchall() == frozen
        for _, path, offset, length in frozen:
            with storage.open_file(lib.root, path) as source:
                source.seek(offset)
                assert len(source.read(length)) == length
    finally:
        held.execute("ROLLBACK")
        held.close()
    for path, sha in before.items():
        with storage.open_file(lib.root, path) as source:
            assert storage.hash_stream(source) == sha
    assert read_json(lib.cache / "ONLINE.json") == pointer
    assert proofs(lib) == (seq, signatures)
    with lib.journal() as db:
        assert [tuple(r) for r in db.execute("SELECT * FROM commits ORDER BY seq")] == journal
        assert [tuple(r) for r in db.execute("SELECT * FROM pinterest_receipts ORDER BY receipt_id")] == replay
    assert lib.verify(deep=True) == dict(batches=5, objects=100, captures=100)
    for through in (3, seq):
        output = tmp_path / ("rebuilt-" + str(through))
        online.rebuild(library.Archive(lib.root), output, through=through)
        db, original = connect(output / "online.sqlite"), connect(lib.cache / "online.sqlite")
        try:
            assert maintenance.signatures(db, through) == maintenance.signatures(original, through)
        finally:
            db.close()
            original.close()
    fixture.job = fixture.service.create(dict(request_key=str(uuid.uuid4()), definition={**fixture.spec,
        "seeds": [dict(kind="pin", id="1000"), dict(kind="pin", id="1200")]}))
    assert fixture.run()["state"] == "completed"
    assert lib.verify(deep=True)["objects"] == 101
    assert compaction.compact(lib, force=True)["state"] == "cleaned"
    assert len(list((lib.root / "containers").glob("*/*.tar"))) == 2
    assert lib.verify(deep=True)["objects"] == 101
    assert compaction.compact(lib, force=True) == dict(state="idle")


@pytest.mark.parametrize("point", ["after_plan", "during_copy", "after_rename", "after_verify", "after_activate", "during_reclaim", "after_reclaim"])
def test_every_switch_and_reclamation_boundary_recovers(tmp_path, monkeypatch, point):
    fixture, lib = fixture_at(tmp_path)
    expected = files(lib)
    before = proofs(lib)
    fired = []
    def fault(name):
        if name == "pinterest_compact_" + point and not fired:
            fired.append(name)
            raise Crash()
    monkeypatch.setattr(compaction, "failpoint", fault)
    with pytest.raises(Crash):
        compaction.compact(lib, force=True)
    lib = fixture.service.library(fixture.lake["library_id"])
    assert lib.verify(deep=True)["objects"] == 4
    result = compaction.compact(lib, force=True)
    assert result["state"] in {"cleaned", "idle"}
    assert len(list((lib.root / "containers").glob("*/*.tar"))) == 1
    assert files(lib) == {}
    for path, sha in expected.items():
        with storage.open_file(lib.root, path) as source:
            assert storage.hash_stream(source) == sha
    assert proofs(lib) == before
    assert len(fixture.pin_calls) == len(fixture.media_calls) == 4
    assert receipts.replay(fixture.state, lib, fixture.job["id"]) == 0


@pytest.mark.parametrize("point", ["after_plan", "after_rename", "after_activate", "during_reclaim"])
def test_abrupt_process_death_resumes_without_new_logical_work(tmp_path, point):
    fixture, lib = fixture_at(tmp_path)
    expected = files(lib)
    worker = Path(__file__).resolve().parents[1]
    env = {**os.environ, "PYTHONPATH": str(worker / "src"), "DANBOORU_STORE_FAIL_AT": "pinterest_compact_" + point, "DANBOORU_STORE_HARD_EXIT": "1"}
    command = [sys.executable, "-m", "studio_lake.pinterest.lake.compaction", "--media-root", str(lib.root), "--index-root", str(lib.cache), "--force", "--drain"]
    crashed = subprocess.run(command, env=env, capture_output=True, timeout=60)
    assert crashed.returncode != 0
    env.pop("DANBOORU_STORE_FAIL_AT")
    env.pop("DANBOORU_STORE_HARD_EXIT")
    resumed = subprocess.run(command, env=env, capture_output=True, timeout=60)
    assert resumed.returncode == 0, resumed.stderr.decode(errors="replace")
    assert files(lib) == {} and lib.verify(deep=True)["objects"] == 4
    for path, sha in expected.items():
        with storage.open_file(lib.root, path) as source:
            assert storage.hash_stream(source) == sha
    assert len(fixture.pin_calls) == len(fixture.media_calls) == 4


def test_corrupt_container_blocks_activation_and_keeps_sources(tmp_path, monkeypatch):
    _, lib = fixture_at(tmp_path)
    expected = files(lib)
    def fault(name):
        if name == "pinterest_compact_after_verify":
            raise Crash()
    monkeypatch.setattr(compaction, "failpoint", fault)
    with pytest.raises(Crash):
        compaction.compact(lib, force=True)
    path = next((lib.root / "containers").glob("*/*.tar"))
    with path.open("r+b") as target:
        target.seek(520)
        target.write(b"bad")
    with pytest.raises(IntegrityError, match="hash differs"):
        compaction.compact(lib, force=True)
    assert files(lib) == expected
    with lib.journal() as db:
        assert db.execute("SELECT state FROM archive_containers").fetchone()[0] == "sealed"


def test_bad_object_reference_prevents_any_reclamation(tmp_path):
    _, lib = fixture_at(tmp_path)
    expected = files(lib)
    db = connect(lib.cache / "online.sqlite")
    db.execute("UPDATE objects SET offset=9999999999")
    db.close()
    with pytest.raises(IntegrityError, match="reference"):
        compaction.compact(lib, force=True)
    assert files(lib) == expected


def test_open_handle_retains_only_the_busy_file_and_retries_cleanup(tmp_path, monkeypatch):
    _, lib = fixture_at(tmp_path)
    original = Path.unlink
    held = next((lib.root / "segments").glob("*/media.tar"))
    def unlink(path, *args, **kwargs):
        if path == held:
            raise PermissionError("held by a reader")
        return original(path, *args, **kwargs)
    monkeypatch.setattr(Path, "unlink", unlink)
    result = compaction.compact(lib, force=True)
    assert result["state"] == "active" and result["retained_files"] == 1
    assert list(files(lib)) == [held.relative_to(lib.root).as_posix()]
    assert lib.verify(deep=True)["objects"] == 4
    monkeypatch.setattr(Path, "unlink", original)
    assert compaction.compact(lib)["state"] == "cleaned"
    assert files(lib) == {}


def test_final_tail_stays_loose_until_threshold_and_aggregates_across_jobs(tmp_path):
    fixture, lib = fixture_at(tmp_path, count=2)
    count = len(files(lib))
    assert compaction.compact(lib, trigger_files=count + 1)["state"] == "idle"
    fixture.job = fixture.service.create(dict(request_key=str(uuid.uuid4()), definition={**fixture.spec, "seeds": [dict(kind="pin", id="1200")]}))
    assert fixture.run()["state"] == "completed"
    result = compaction.compact(lib, trigger_files=count + 1)
    assert result["state"] == "cleaned"
    assert result["references"]["batches"] > 3
    assert len(list((lib.root / "containers").glob("*/*.tar"))) == 1


def test_range_reader_cannot_read_neighbor_files(tmp_path):
    path = tmp_path / "bytes.bin"
    path.write_bytes(b"prefixPAYLOADsuffix")
    with storage.RangeReader(path.open("rb"), 6, 7) as reader:
        assert reader.read(100) == b"PAYLOAD"
        assert reader.read() == b""
        reader.seek(-3, 2)
        assert reader.read() == b"OAD"
        reader.seek(100)
        assert reader.read() == b""
        with pytest.raises(ValueError):
            reader.seek(-1)
    with pytest.raises(IntegrityError):
        storage.RangeReader(path.open("rb"), 10, 100)


def test_copy_interrupt_retains_all_original_files_and_resumes(tmp_path, monkeypatch):
    _, lib = fixture_at(tmp_path)
    expected = files(lib)
    stopped = []
    monkeypatch.setattr(compaction, "failpoint", lambda name: stopped.append(True) if name == "pinterest_compact_during_copy" else None)
    assert compaction.compact(lib, force=True, stop=lambda: bool(stopped))["state"] == "interrupted"
    assert files(lib) == expected
    assert compaction.compact(lib, force=True)["state"] == "cleaned"
    with lib.journal() as db:
        row = db.execute("SELECT path,sha256 FROM archive_containers").fetchone()
    assert file_hash(lib.root / row[0]) == row[1]


def test_idle_maintenance_recovers_a_finished_job_without_changing_its_control_state(tmp_path, monkeypatch):
    from concurrent.futures import ThreadPoolExecutor
    from studio_lake.updates.runner import Runner as Coordinator

    fixture, lib = fixture_at(tmp_path)
    def fault(name):
        if name == "pinterest_compact_after_plan":
            raise Crash()
    monkeypatch.setattr(compaction, "failpoint", fault)
    with pytest.raises(Crash):
        compaction.compact(lib, force=True)
    monkeypatch.setattr(compaction, "failpoint", lambda _: None)
    before = fixture.service.row(fixture.job["id"])
    coordinator = Coordinator(fixture.state)
    original = coordinator.archiving.run
    def maintain(identity):
        try:
            return original(identity)
        finally:
            coordinator.stop.set()
    monkeypatch.setattr(coordinator.archiving, "run", maintain)
    timer = threading.Timer(20, coordinator.stop.set)
    timer.start()
    try:
        with ThreadPoolExecutor(max_workers=4) as pool:
            coordinator.schedule(pool, False)
    finally:
        timer.cancel()
    assert coordinator.archiving.status(fixture.lake["library_id"])["state"] == "cleaned"
    assert fixture.service.row(fixture.job["id"]) == before
    assert files(lib) == {}


def test_idle_maintenance_excludes_busy_lakes_and_backs_off_corruption(tmp_path, monkeypatch):
    from studio_lake.pinterest import archiving

    fixture, _ = fixture_at(tmp_path)
    worker = archiving.Archiving(fixture.service, threading.Event())
    identity = fixture.lake["library_id"]
    assert worker.candidate([identity], at=30) is None
    assert worker.candidate(at=60) == identity
    assert worker.candidate(at=61) is None
    before = fixture.service.row(fixture.job["id"])
    def corrupt(*_, **__):
        raise IntegrityError("container corrupted")
    monkeypatch.setattr(archiving, "compact", corrupt)
    result = worker.run(identity)
    assert result["state"] == "needs_review"
    assert worker.status(identity)["error_message"] == "container corrupted"
    assert worker.candidate(at=120) is None
    assert fixture.service.row(fixture.job["id"]) == before


def test_old_single_receipt_archive_can_compact_then_accept_new_bundles(tmp_path, monkeypatch):
    from pinterest_fixtures import Fixture
    from studio_lake.pinterest.lake import FEATURES
    from studio_lake.util import atomic_json

    fixture = Fixture(tmp_path, ("123",))
    lib = fixture.service.library(fixture.lake["library_id"])
    atomic_json(lib.root / "library.json", {**lib.info, "required_features": list(FEATURES)})
    save = receipts.save_prepared
    monkeypatch.setattr(receipts, "save_prepared", lambda root, replay, records, media=None, **_: save(root, replay, records, media))
    assert fixture.run()["state"] == "completed"
    lib = fixture.service.library(fixture.lake["library_id"])
    assert compaction.compact(lib, force=True)["state"] == "cleaned"
    assert read_json(lib.root / "library.json")["required_features"] == [*FEATURES, "archive-containers-v1"]
    monkeypatch.setattr(receipts, "save_prepared", save)
    fixture.job = fixture.service.create(dict(request_key=str(uuid.uuid4()), definition={**fixture.spec, "seeds": [dict(kind="pin", id="124")]}))
    assert fixture.run()["state"] == "completed"
    assert "archive-containers-v1" in read_json(lib.root / "library.json")["required_features"]
    assert fixture.service.library(fixture.lake["library_id"]).verify(deep=True)["captures"] == 2


def test_deep_verification_checks_the_outer_container_not_only_object_bytes(tmp_path):
    _, lib = fixture_at(tmp_path)
    compaction.compact(lib, force=True)
    path = next((lib.root / "containers").glob("*/*.tar"))
    with path.open("r+b") as target:
        target.write(b"changed-header")
    with pytest.raises(IntegrityError, match="header"):
        lib.verify(deep=True)


@pytest.mark.parametrize("phase", ["after_plan", "after_activate"])
def test_redirected_member_cannot_delete_another_file_inside_the_same_lake(tmp_path, monkeypatch, phase):
    _, lib = fixture_at(tmp_path)
    def fault(name):
        if name == "pinterest_compact_" + phase:
            raise Crash()
    monkeypatch.setattr(compaction, "failpoint", fault)
    with pytest.raises(Crash):
        compaction.compact(lib, force=True)
    source = next((lib.root / "segments").glob("*/media.tar"))
    unrelated = lib.root / "unmanaged.tar"
    unrelated.write_bytes(source.read_bytes())
    before = file_hash(unrelated)
    source.unlink()
    try:
        source.symlink_to(unrelated)
    except OSError:
        pytest.skip("File symlink creation is unavailable")
    monkeypatch.setattr(compaction, "failpoint", lambda _: None)
    with pytest.raises(IntegrityError, match="链接"):
        compaction.compact(lib, force=True)
    assert source.is_symlink() and file_hash(unrelated) == before


def test_insufficient_space_keeps_the_original_layout_and_capability_marker(tmp_path, monkeypatch):
    from types import SimpleNamespace

    _, lib = fixture_at(tmp_path)
    marker, original = file_hash(lib.root / "library.json"), files(lib)
    monkeypatch.setattr(compaction.shutil, "disk_usage", lambda _: SimpleNamespace(free=0))
    assert compaction.compact(lib, force=True)["state"] == "waiting_space"
    assert file_hash(lib.root / "library.json") == marker and files(lib) == original


def test_container_batch_cap_rolls_over_without_changing_logical_versions(tmp_path, monkeypatch):
    _, lib = fixture_at(tmp_path)
    expected, before = files(lib), proofs(lib)
    monkeypatch.setattr(compaction, "MAX_BATCHES", 2)
    first = compaction.compact(lib, force=True)
    assert first["state"] == "cleaned" and first["references"]["batches"] == 2
    assert files(lib) and lib.verify(deep=True)["objects"] == 4
    second = compaction.compact(lib, force=True)
    assert second["state"] == "cleaned" and second["references"]["batches"] == 1
    assert first["removed_files"] + second["removed_files"] == len(expected)
    assert files(lib) == {} and proofs(lib) == before
