import json
from pathlib import Path
import sqlite3
import shutil

import pytest

from test_archive_rebuild import archive, record
from studio_lake.archive_rebuild import build_archive, verify_archive, compare_reference
from studio_lake.daily_state import DailyStore
from studio_lake.index import Index
from studio_lake.online_migrate import migrate, verify_projection, activate_projection
from studio_lake.producer_retirement import retire_producer, handoff_legacy
from studio_lake.updates.state import State
from studio_lake.util import IntegrityError, read_json, atomic_json, now


def serving(lib):
    batch = archive(lib, "danbooru", [record(), record(2)], "first")
    Index(lib).sync()
    migrate(lib.root, lib.cache, lib.cache, "danbooru", chunk_rows=2)
    verify_projection(lib.cache, memory_mib=256)
    activate_projection(lib.root, lib.cache)
    return batch


def independent_proof(lib, tmp_path):
    output = tmp_path / "archive-proof"
    build_archive(lib.root, output, "danbooru", reference_index=lib.cache)
    verify_archive(output)
    compare_reference(output)
    return output


def test_retirement_preserves_live_generation_and_future_publication_without_native_files(lib, tmp_path):
    serving(lib)
    proof = independent_proof(lib, tmp_path)
    pointer = read_json(lib.cache / "ONLINE.json")
    current = read_json(lib.cache / "CURRENT.json")
    plan = retire_producer(lib.root, lib.cache, proof)
    assert plan["candidate_bytes"] > 0 and not (lib.cache / "PRODUCER-RETIRED.json").exists()
    receipt = retire_producer(lib.root, lib.cache, proof, apply=True)
    assert receipt["phase"] == "complete" and receipt["deleted_bytes"] > 0
    assert not (lib.cache / "indexes" / current["generation"]).exists()
    assert read_json(lib.cache / "ONLINE.json") == pointer
    assert read_json(lib.cache / "CURRENT.json")["retired"] is True
    with pytest.raises(IntegrityError, match="已退役"):
        Index(lib)
    assert retire_producer(lib.root, lib.cache, proof, apply=True) == receipt
    archive(lib, "danbooru", [record(3)], "later")
    with sqlite3.connect(lib.cache / pointer["file"]) as db:
        assert db.execute("SELECT count(*) FROM observations").fetchone()[0] == 3


def test_retirement_refuses_unknown_files_and_changed_archive(lib, tmp_path):
    batch = serving(lib)
    proof = independent_proof(lib, tmp_path)
    generation = lib.cache / "indexes" / read_json(lib.cache / "CURRENT.json")["generation"]
    extra = generation / "user-note.txt"
    extra.write_text("keep me", encoding="utf-8")
    with pytest.raises(IntegrityError, match="未知文件"):
        retire_producer(lib.root, lib.cache, proof, apply=True)
    assert (generation / "analysis.duckdb").is_file() and not (lib.cache / "PRODUCER-RETIRED.json").exists()
    extra.unlink()
    source = lib.root / "segments" / batch.id / "source.parquet"
    original = source.read_bytes()
    source.write_bytes(b"bad!" + original[4:])
    with pytest.raises(IntegrityError, match="归档文件发生变化"):
        retire_producer(lib.root, lib.cache, proof, apply=True)
    assert (generation / "analysis.duckdb").is_file()


def test_partial_deletion_resumes_from_receipt_and_fails_closed_for_legacy_readers(
    lib, tmp_path, monkeypatch
):
    serving(lib)
    proof = independent_proof(lib, tmp_path)
    unlink = Path.unlink

    def busy(path, *args, **kwargs):
        if path.name == "catalog.sqlite":
            raise PermissionError("simulated busy native file")
        return unlink(path, *args, **kwargs)

    monkeypatch.setattr(Path, "unlink", busy)
    with pytest.raises(PermissionError, match="simulated busy"):
        retire_producer(lib.root, lib.cache, proof, apply=True)
    assert read_json(lib.cache / "PRODUCER-RETIRED.json")["phase"] == "prepared"
    with pytest.raises(IntegrityError, match="已退役"):
        Index(lib)
    monkeypatch.setattr(Path, "unlink", unlink)
    assert retire_producer(lib.root, lib.cache, proof, apply=True)["phase"] == "complete"


def test_legacy_handoff_requires_complete_equivalent_scope_and_does_not_forge_verification(lib, tmp_path):
    batch = serving(lib)
    state = State(tmp_path / "control")
    state.register(
        {
            "library_id": lib.info["library_id"],
            "site": "danbooru",
            "media_root": str(lib.root),
            "index_root": str(lib.cache),
        }
    )
    DailyStore(lib).initialize()
    identity = "a" * 32
    with lib.journal() as db, db:
        db.execute(
            "INSERT INTO daily_runs(run_id,kind,state,api_complete,parameters_json,created_at,updated_at) "
            "VALUES(?,'new_posts','planning',1,?,?,?)",
            (identity, json.dumps({"profile": "webp-2048-q95"}), now(), now()),
        )
        db.execute("INSERT INTO daily_run_batches VALUES(?,?,1,'api')", (identity, batch.id))
    job = state.create(
        {
            "library_id": lib.info["library_id"],
            "range": {"kind": "ids", "ids": [1, 2]},
            "media": {"profile": "webp-2048-q95", "allow_sample": False, "existing": "keep"},
            "page_budget": 10,
            "item_budget": 10,
        },
        "handoff-fixture",
    )
    with pytest.raises(IntegrityError, match="尚未完成"):
        handoff_legacy(lib.root, lib.cache, state.root, identity, job["id"], apply=True)
    with state.db() as db:
        db.execute("UPDATE jobs SET state='completed_with_exclusions' WHERE id=?", (job["id"],))
        db.executemany(
            "INSERT INTO items(job_id,post_id,record_json,state) VALUES(?,?,'{}',?)",
            [(job["id"], 1, "stored"), (job["id"], 2, "unavailable")],
        )
    receipt = handoff_legacy(lib.root, lib.cache, state.root, identity, job["id"], apply=True)
    assert receipt["counts"] == {"stored": 1, "reused": 0, "unavailable": 1}
    with lib.journal() as db:
        row = db.execute(
            "SELECT state,completed_at,result_json FROM daily_runs WHERE run_id=?", (identity,)
        ).fetchone()
        assert tuple(row) == ("handed_off", None, None)
    assert DailyStore(lib).runs(unfinished=True) == []
    assert Path(receipt["journal_backup"]["file"]).is_file()
    assert handoff_legacy(lib.root, lib.cache, state.root, identity, job["id"], apply=True) == receipt


def test_retirement_refuses_a_different_comparison_generation(lib, tmp_path):
    serving(lib)
    proof = independent_proof(lib, tmp_path)
    compared = read_json(proof / "ONLINE-COMPARE.json")
    compared["reference_generation"] = "other"
    atomic_json(proof / "ONLINE-COMPARE.json", compared)
    with pytest.raises(IntegrityError, match="对照证据"):
        retire_producer(lib.root, lib.cache, proof, apply=True)
    assert not (lib.cache / "PRODUCER-RETIRED.json").exists()


@pytest.mark.parametrize("site", ["danbooru", "gelbooru", "yandere"])
def test_owned_downloader_publishes_and_reuses_after_producer_retirement(tmp_path, site):
    from conftest import png
    from test_updates import setup, post, job, FakeSite, Images
    from studio_lake.updates.runner import Runner
    from studio_lake.updates.resources import Resources

    lib, state = setup(tmp_path, site)
    proof = tmp_path / "proof"
    build_archive(lib.root, proof, site, reference_index=lib.cache)
    verify_archive(proof)
    compare_reference(proof)
    retire_producer(lib.root, lib.cache, proof, apply=True)
    data = png("red")
    remote, images = FakeSite(site, [post(site, 11, data)]), Images(data)
    runner = Runner(state, {site: remote}, Resources(reserve_bytes=0), images)
    first = job(state, lib, {"kind": "ids", "ids": [11]}, "original")
    assert runner.run(first["id"])["counts"] == {"stored": 1}
    second = job(state, lib, {"kind": "ids", "ids": [11]}, "original")
    assert runner.run(second["id"])["counts"] == {"reused": 1}
    assert read_json(lib.cache / "PRODUCER-RETIRED.json")["phase"] == "complete"


def test_relocation_preserves_retirement_mode_and_rejects_missing_evidence(lib, tmp_path):
    from studio_lake.updates import relocation
    from studio_lake.updates.sites import UpdateError

    serving(lib)
    proof = independent_proof(lib, tmp_path)
    state = State(tmp_path / "controller")
    state.register(
        {
            "library_id": lib.info["library_id"],
            "site": "danbooru",
            "media_root": str(lib.root),
            "index_root": str(lib.cache),
        }
    )
    retire_producer(lib.root, lib.cache, proof, apply=True)
    move = relocation.prepare(state, lib.info["library_id"])
    destination = tmp_path / "moved-index"
    shutil.copytree(lib.cache, destination)
    marker = destination / "PRODUCER-RETIRED.json"
    contents = marker.read_bytes()
    marker.unlink()
    with pytest.raises(UpdateError, match="retirement marker"):
        relocation.apply(state, move["id"], str(lib.root), str(destination))
    marker.write_bytes(contents)
    assert relocation.apply(state, move["id"], str(lib.root), str(destination))["phase"] == "writer_committed"
    relocation.finish(state, move["id"])
    with pytest.raises(IntegrityError, match="已退役"):
        Index(state.library(lib.info["library_id"]))
