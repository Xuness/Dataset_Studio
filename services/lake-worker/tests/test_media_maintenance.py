import pytest

from pixiv_fixtures import add_images, add_work, sample
from studio_lake.archive_rebuild import activate_archive, build_archive, cleanup_preparation, compare_reference, verify_archive
from studio_lake.media_lake.library import MediaLibrary
from studio_lake.media_lake.reader import Reader
from studio_lake.online_storage import connect
from studio_lake.util import IntegrityError, contained, read_json


def test_archive_only_reconstruction_after_complete_cache_loss(tmp_path):
    lib, state = sample(tmp_path)
    records, _, captured = add_work(lib, state)
    sha, data, seq = add_images(lib, state, records)
    lib.cache.rename(tmp_path / "offline-cache")
    # A missing online-index link is an explicit recovery precondition, not permission
    # to replace a surviving project's generation.
    (lib.root / "online-index.json").unlink()
    archive = MediaLibrary.archive(lib.root)
    assert archive.verify(deep=True)["objects"] == 1
    output = tmp_path / "recovered"
    plan = build_archive(lib.root, output, "pixiv", chunk_rows=1)
    assert plan["sequence"] == seq
    assert verify_archive(output)["raw_roundtrip_verified"]
    activate_archive(lib.root, output)
    with Reader(lib.root, output) as reader:
        assert reader.blob(sha) == data
        assert reader.raw(captured["capture_id"])["json"].encode() == captured["raw_body"]
        assert len(reader.media("12345")["items"]) == 2
    assert activate_archive(lib.root, output)["schema_version"] == 3
    with pytest.raises(IntegrityError):
        cleanup_preparation(output, tmp_path / "evidence")


def test_reconstruction_keeps_serving_pointer_and_compares_fixed_prefix(tmp_path):
    lib, state = sample(tmp_path)
    records, _, _ = add_work(lib, state)
    add_images(lib, state, records)
    lib.sync_online()
    before = (lib.cache / "ONLINE.json").read_bytes()
    output = tmp_path / "preparation"
    interrupted = False

    def stop(phase, _):
        nonlocal interrupted
        if phase == "projection" and not interrupted:
            interrupted = True
            raise RuntimeError("interrupted")

    with pytest.raises(RuntimeError, match="interrupted"):
        build_archive(lib.root, output, "pixiv", reference_index=lib.cache, stop=stop)
    plan = build_archive(lib.root, output, "pixiv", reference_index=lib.cache)
    # The reference may advance while its old snapshot remains protected.
    add_work(lib, state, pages=1, at="2026-10-03T03:00:00.000000Z")
    lib.sync_online()
    proof = verify_archive(output)
    assert proof["sequence"] == plan["sequence"]
    assert compare_reference(output)["equal"]
    with pytest.raises(IntegrityError):
        activate_archive(lib.root, output)
    assert (lib.cache / "ONLINE.json").read_bytes() == before
    receipt = cleanup_preparation(output, tmp_path / "proof")
    assert receipt["deleted_bytes"] > 0
    assert not (output / plan["file"]).exists()
    assert (tmp_path / "proof" / "ONLINE-VERIFY.json").is_file()


def test_verified_preparation_tamper_cannot_activate(tmp_path):
    lib, _ = sample(tmp_path)
    output = tmp_path / "preparation"
    build_archive(lib.root, output, "pixiv")
    verify_archive(output)
    (lib.root / "online-index.json").unlink()
    plan = read_json(output / "ONLINE-BUILD.json")
    db = connect(contained(output, plan["file"]))
    try:
        db.execute("UPDATE online_state SET value='changed' WHERE key='site'")
        db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    finally:
        db.close()
    with pytest.raises(IntegrityError):
        activate_archive(lib.root, output)
