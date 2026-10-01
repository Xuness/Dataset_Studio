import hashlib
import json
import sqlite3

import pyarrow as pa
import pytest

from studio_lake.api_metadata import normalize_api
from studio_lake.archive_rebuild import build_archive, verify_archive, activate_archive, compare_reference
from studio_lake.archive_rebuild import cleanup_preparation
from studio_lake.library import Batch
from studio_lake.metadata import asset
from studio_lake.raw_codec import encode, decode
from studio_lake.util import IntegrityError, read_json


def archive(lib, site, records, key, observed="2026-01-01T00:00:00+00:00", images=True):
    kind = {"danbooru": "api_json", "yandere": "api_yandere_v1", "gelbooru": "api_gelbooru_v1"}[site]
    body = [json.dumps(r, ensure_ascii=False, separators=(",", ":")) for r in records]
    batch = Batch(lib, key, {"kind": "fixture"})
    batch.add_source(pa.table({"post_json": body}))
    for i, row in enumerate(records):
        observation = normalize_api(row, key, i, kind, i, observed, "exact")
        batch.observations.append(observation)
        if images:
            data = (key + str(row["id"])).encode()
            sha, ext = batch.add_blob(data, "bin", lambda _: None)
            batch.assets.append(asset(observation, sha, ext, len(data)))
    batch.commit()
    return batch


def record(post=1, **extra):
    return {
        "id": post,
        "md5": f"{post:032x}",
        "rating": "s",
        "tags": "a b",
        "tag_string": "a b",
        "score": 4,
        "unknown": {"保留": [None, False, 2**60 + 3]},
        **extra,
    }


@pytest.mark.parametrize("site", ["danbooru", "yandere", "gelbooru"])
def test_archive_bootstrap_without_any_native_index_and_then_incremental_publish(lib, tmp_path, site):
    archive(lib, site, [record(), record(2)], "first")
    archive(lib, site, [record(score=9)], "refresh", "2026-02-01T00:00:00+00:00", images=False)
    output = tmp_path / "online"
    assert not (lib.cache / "CURRENT.json").exists()
    build_archive(lib.root, output, site, chunk_rows=1)
    proof = verify_archive(output)
    assert proof["raw_roundtrip_verified"] and proof["tables"]["observations"]["rows"] == 3
    assert not (lib.cache / "indexes").exists()
    activate_archive(lib.root, output)
    assert activate_archive(lib.root, output)["site"] == site
    pointer = read_json(output / "ONLINE.json")
    with sqlite3.connect(output / pointer["file"]) as db:
        row = db.execute(
            "SELECT o.score,p.asset_id FROM post_versions p JOIN observations o ON o.row_id=p.row_id "
            "WHERE p.post_id=1 AND p.valid_until IS NULL"
        ).fetchone()
        assert row[0] == 9 and row[1] is not None
    archive(lib, site, [record(3)], "new", "2026-03-01T00:00:00+00:00")
    with sqlite3.connect(output / pointer["file"]) as db:
        assert db.execute("SELECT value FROM online_state WHERE key='served_seq'").fetchone()[0] == "3"
        assert db.execute("SELECT count(*) FROM observations").fetchone()[0] == 4


def test_resume_after_raw_chunk_and_detect_changed_archive_prefix(lib, tmp_path):
    archive(lib, "danbooru", [record(), record(2), record(3)], "first")
    output = tmp_path / "rebuilt"

    def interrupt(phase, _seq, _chunks):
        if phase == "raw":
            raise RuntimeError("simulated interruption")

    with pytest.raises(RuntimeError, match="simulated interruption"):
        build_archive(lib.root, output, "danbooru", chunk_rows=1, stop=interrupt)
    build_archive(lib.root, output, "danbooru", chunk_rows=2)
    assert verify_archive(output)["tables"]["raw_metadata"]["rows"] == 3
    with lib.journal() as db, db:
        db.execute("UPDATE commits SET manifest_json=manifest_json||' '")
    with pytest.raises(IntegrityError, match="身份已变化"):
        build_archive(lib.root, output, "danbooru")


def test_source_hash_corruption_is_rejected_before_build_is_published(lib, tmp_path):
    batch = archive(lib, "danbooru", [record()], "first")
    source = lib.root / "segments" / batch.id / "source.parquet"
    source.write_bytes(source.read_bytes() + b"changed")
    output = tmp_path / "rebuilt"
    with pytest.raises(IntegrityError, match="封存元数据校验失败"):
        build_archive(lib.root, output, "danbooru")
    assert not (output / "ONLINE.json").exists()


def test_raw_corruption_and_changed_verified_database_cannot_activate(lib, tmp_path):
    archive(lib, "danbooru", [record()], "first")
    output = tmp_path / "rebuilt"
    build = build_archive(lib.root, output, "danbooru")
    verify_archive(output)
    with sqlite3.connect(output / build["file"]) as db:
        db.execute("UPDATE raw_metadata SET raw_zlib=?", (b"broken",))
    with pytest.raises(IntegrityError, match="校验后发生变化"):
        activate_archive(lib.root, output)
    with pytest.raises(IntegrityError, match="压缩内容"):
        verify_archive(output)
    assert not (lib.root / "online-index.json").exists()


def test_existing_online_generation_is_preserved_and_reference_comparison_uses_identity(lib, tmp_path):
    archive(lib, "danbooru", [record(), record(2)], "first")
    existing = tmp_path / "existing"
    build_archive(lib.root, existing, "danbooru")
    verify_archive(existing)
    original = activate_archive(lib.root, existing)
    archive(lib, "danbooru", [record(3)], "new")
    output = tmp_path / "comparison"
    build_archive(lib.root, output, "danbooru", reference_index=existing)
    with sqlite3.connect(existing / original["file"]) as db:
        assert db.execute("SELECT count(*) FROM leases WHERE purpose='archive-validation'").fetchone()[0] == 1
    verify_archive(output)
    assert compare_reference(output)["equal"]
    assert read_json(existing / "ONLINE.json") == original
    with sqlite3.connect(existing / original["file"]) as db:
        assert db.execute("SELECT count(*) FROM leases WHERE purpose='archive-validation'").fetchone()[0] == 0
    with pytest.raises(IntegrityError, match="现有在线湖"):
        activate_archive(lib.root, output)
    with pytest.raises(IntegrityError, match="不能覆盖"):
        build_archive(lib.root, existing, "danbooru")


@pytest.mark.parametrize("raw", [None, "", '{"中文":1.000000000001,"n":1152921504606846979}'])
def test_raw_codec_preserves_exact_text_and_null(raw):
    length, sha, compressed = encode(raw)
    assert decode(compressed, length, sha, maximum_bytes=1000) == raw
    assert sha == hashlib.sha256((raw or "").encode()).hexdigest()
    with pytest.raises(IntegrityError):
        decode(compressed + b"trailing", length, sha, maximum_bytes=1000)
    with pytest.raises(IntegrityError):
        decode(compressed, length, "0" * 64, maximum_bytes=1000)


def test_raw_codec_enforces_declared_and_actual_decompression_bounds():
    length, sha, compressed = encode("x" * 10000)
    with pytest.raises(IntegrityError, match="预算"):
        decode(compressed, length, sha, maximum_bytes=9999)
    with pytest.raises(IntegrityError, match="长度校验"):
        decode(compressed, 10, sha, maximum_bytes=100)


def test_preparation_cleanup_preserves_proofs_and_refuses_an_active_serving_database(lib, tmp_path):
    archive(lib, "danbooru", [record()], "first")
    output = tmp_path / "private"
    build = build_archive(lib.root, output, "danbooru")
    verify_archive(output)
    evidence = tmp_path / "evidence"
    receipt = cleanup_preparation(output, evidence)
    assert receipt["deleted_bytes"] > 0 and not (output / build["file"]).exists()
    assert (evidence / "ONLINE-VERIFY.json").is_file()
    assert read_json(output / "ONLINE-BUILD.json")["state"] == "cleaned"
    assert cleanup_preparation(output, evidence) == receipt
    active = tmp_path / "active"
    build_archive(lib.root, active, "danbooru")
    verify_archive(active)
    activate_archive(lib.root, active)
    with pytest.raises(IntegrityError):
        cleanup_preparation(active, tmp_path / "forbidden")
