import sqlite3

import pytest

from studio_lake.archive_bindings import RELATIVE, adopt
from studio_lake.archive_rebuild import build_archive, verify_archive, compare_reference, activate_archive
from studio_lake.util import IntegrityError, read_json, atomic_json
from test_archive_rebuild import archive, record


def legacy_projection(lib, tmp_path, md5=None):
    archive(lib, "danbooru", [record()], "saved")
    archive(lib, "danbooru", [record(md5=md5)], "hidden", "2026-02-01T00:00:00+00:00", images=False)
    existing = tmp_path / "existing"
    build_archive(lib.root, existing, "danbooru")
    verify_archive(existing)
    pointer = activate_archive(lib.root, existing)
    # Reproduce the historical importer retaining an earlier same-post asset.
    with sqlite3.connect(existing / pointer["file"]) as db:
        asset = db.execute("SELECT asset_id FROM assets WHERE post_id=1").fetchone()[0]
        db.execute("UPDATE post_versions SET asset_id=? WHERE post_id=1", (asset,))
    prepared = tmp_path / "prepared"
    build_archive(lib.root, prepared, "danbooru", reference_index=existing)
    verify_archive(prepared)
    with pytest.raises(IntegrityError, match="不一致"):
        compare_reference(prepared)
    return existing, prepared, pointer, asset


def test_legacy_association_is_archived_and_recoverable_without_producer_or_reference(lib, tmp_path):
    existing, prepared, pointer, asset = legacy_projection(lib, tmp_path)
    before_pointer = (existing / "ONLINE.json").read_bytes()
    receipt = adopt(prepared)
    assert receipt["rows"] == 1 and receipt["phase"] == "complete"
    assert (existing / "ONLINE.json").read_bytes() == before_pointer
    assert read_json(prepared / "ONLINE-COMPARE.json")["equal"]
    assert adopt(prepared) == receipt
    manifest = read_json(lib.root / RELATIVE)
    assert manifest["bindings"][0]["asset_id"] == asset
    assert not (lib.cache / "indexes").exists()
    # A fresh independent build needs only the canonical archive and its source manifest.
    recovered = tmp_path / "recovered"
    built = build_archive(lib.root, recovered, "danbooru")
    verify_archive(recovered)
    with sqlite3.connect(recovered / built["file"]) as db:
        assert db.execute("SELECT asset_id FROM post_versions WHERE post_id=1").fetchone()[0] == asset
    assert read_json(prepared / "ONLINE-VERIFY.json")["controlled_repair"]["table"] == "post_versions"
    with sqlite3.connect(existing / pointer["file"]) as db:
        assert db.execute("SELECT count(*) FROM leases WHERE purpose='archive-validation'").fetchone()[0] == 0


def test_later_post_publication_uses_current_rules_and_does_not_replay_obsolete_binding(lib, tmp_path):
    existing, prepared, _, _ = legacy_projection(lib, tmp_path)
    adopt(prepared)
    archive(lib, "danbooru", [record(md5=None, score=20)], "later", "2026-03-01T00:00:00+00:00", images=False)
    recovered = tmp_path / "later-recovery"
    built = build_archive(lib.root, recovered, "danbooru", reference_index=existing)
    verify_archive(recovered)
    assert compare_reference(recovered)["equal"]
    with sqlite3.connect(recovered / built["file"]) as db:
        assert db.execute("SELECT asset_id FROM post_versions WHERE post_id=1").fetchone()[0] is None


def test_known_changed_md5_is_not_relabelled_as_a_legacy_association(lib, tmp_path):
    _, prepared, _, _ = legacy_projection(lib, tmp_path, md5="f" * 32)
    with pytest.raises(IntegrityError, match="无法解释"):
        adopt(prepared)
    assert not (lib.root / RELATIVE).exists()


def test_changed_verified_database_and_changed_binding_manifest_are_rejected(lib, tmp_path):
    _, prepared, _, _ = legacy_projection(lib, tmp_path)
    built = read_json(prepared / "ONLINE-BUILD.json")
    with sqlite3.connect(prepared / built["file"]) as db:
        db.execute("UPDATE raw_metadata SET raw_zlib=?", (b"changed",))
    with pytest.raises(IntegrityError, match="完整验证文件已变化"):
        adopt(prepared)
    assert not (lib.root / RELATIVE).exists()


def test_binding_manifest_is_part_of_rebuild_identity(lib, tmp_path):
    _, prepared, _, _ = legacy_projection(lib, tmp_path)
    adopt(prepared)
    manifest = read_json(lib.root / RELATIVE)
    manifest["bindings"][0]["asset_id"] = "0" * 64
    atomic_json(lib.root / RELATIVE, manifest)
    with pytest.raises(IntegrityError, match="历史关联归档"):
        verify_archive(prepared)


def test_interrupted_adoption_recovers_only_through_full_verification(lib, tmp_path, monkeypatch):
    from studio_lake import archive_bindings

    _, prepared, _, _ = legacy_projection(lib, tmp_path)
    apply = archive_bindings.apply_to_preparation

    def interrupt(*args):
        apply(*args)
        raise RuntimeError("interrupted after the private transaction")

    monkeypatch.setattr(archive_bindings, "apply_to_preparation", interrupt)
    with pytest.raises(RuntimeError, match="interrupted"):
        adopt(prepared)
    assert read_json(prepared / "ONLINE-BUILD.json")["state"] == "built"
    monkeypatch.setattr(archive_bindings, "apply_to_preparation", apply)
    assert adopt(prepared)["resumed_with_full_verification"]
    assert read_json(prepared / "ONLINE-COMPARE.json")["equal"]


def test_comparison_uses_only_schemas_referenced_by_the_protected_snapshot(lib, tmp_path):
    import json
    import pyarrow as pa
    from studio_lake.library import Batch
    from studio_lake.api_metadata import normalize_api

    archive(lib, "danbooru", [record()], "base")
    existing = tmp_path / "existing"
    build_archive(lib.root, existing, "danbooru")
    verify_archive(existing)
    activate_archive(lib.root, existing)
    prepared = tmp_path / "prepared"
    build_archive(lib.root, prepared, "danbooru", reference_index=existing)
    batch = Batch(lib, "later-schema", {"kind": "fixture"})
    value = record(2)
    batch.add_source(pa.table({"post_json": [json.dumps(value)], "extra_column": [1]}))
    batch.observations.append(
        normalize_api(value, "later-schema", 0, "api_json", 0, "2026-03-01T00:00:00+00:00", "exact")
    )
    batch.commit()
    verify_archive(prepared)
    compared = compare_reference(prepared)
    assert compared["equal"]
    assert compared["tables"]["source_schemas"]["existing"][0] == 1
