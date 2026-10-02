
import pytest

from pixiv_fixtures import add_images, add_work, new_batch, sample
from studio_lake.library import Library
from studio_lake.media_lake.online import rebuild
from studio_lake.media_lake.reader import Reader
from studio_lake.util import IntegrityError


def test_media_occurrences_deduplicate_bytes_and_keep_old_watermark(tmp_path):
    lib, state = sample(tmp_path)
    assert type(Library(lib.config)) is type(lib)
    records, seq, captured = add_work(lib, state)
    lib.sync_online()
    with Reader(lib.root, lib.cache) as reader:
        old = reader.version
        assert reader.objects()["items"] == []
        assert len(reader.media("12345")["items"]) == 2
    sha, data, end = add_images(lib, state, records)
    assert end == seq + 1
    with Reader(lib.root, lib.cache) as reader:
        assert reader.version == old
    lib.sync_online(chunk_rows=1)
    with Reader(lib.root, lib.cache) as reader:
        assert len(reader.objects()["items"]) == 1
        page = reader.media("12345", limit=1)
        second = reader.media("12345", cursor=page["next_cursor"], limit=1)
        first_asset, second_asset = page["items"][0]["bindings"][0], second["items"][0]["bindings"][0]
        assert first_asset["sha256"] == second_asset["sha256"] == sha
        assert first_asset["asset_id"] != second_asset["asset_id"]
        assert reader.blob(sha) == data
        assert reader.raw(captured["capture_id"])["json"].encode() == captured["raw_body"]
    with Reader(lib.root, lib.cache, version=old) as reader:
        assert reader.objects()["items"] == []
        assert not reader.media("12345")["items"][0]["bindings"]
    assert lib.verify(deep=True)["objects"] == 1


@pytest.mark.parametrize("phase", ["preparing", "facts:captures", "facts:media_entries", "projection", "before_watermark"])
def test_publication_failure_is_hidden_and_replays(tmp_path, phase):
    lib, state = sample(tmp_path)
    lib.sync_online()
    records, seq, _ = add_work(lib, state)

    def stop(stage, sequence):
        if stage == phase:
            raise RuntimeError("injected")

    with pytest.raises(RuntimeError, match="injected"):
        lib.sync_online(chunk_rows=1, stop=stop)
    with Reader(lib.root, lib.cache) as reader:
        assert reader.seq == 1
        with pytest.raises(KeyError):
            reader.work("12345")
    assert lib.sync_online(chunk_rows=1)["served_seq"] == seq
    assert lib.sync_online()["published"] == 0
    with Reader(lib.root, lib.cache) as reader:
        assert len(reader.media("12345")["items"]) == len(records["media_entries"])


def test_current_manifest_never_uses_previous_page_bytes(tmp_path):
    lib, state = sample(tmp_path)
    records, _, _ = add_work(lib, state)
    add_images(lib, state, records)
    lib.sync_online()
    with Reader(lib.root, lib.cache) as reader:
        old = reader.version
    add_work(lib, state, at="2026-10-03T03:00:00.000000Z", pages=1)
    lib.sync_online()
    with Reader(lib.root, lib.cache) as reader:
        assert len(reader.media("12345")["items"]) == 1
        assert reader.media("12345")["items"][0]["bindings"] == []
    # A late older response cannot override the new manifest.
    add_work(lib, state, at="2026-10-03T00:30:00.000000Z", pages=3)
    lib.sync_online()
    with Reader(lib.root, lib.cache) as reader:
        assert len(reader.media("12345")["items"]) == 1
    with Reader(lib.root, lib.cache, version=old) as reader:
        assert len(reader.media("12345")["items"]) == 2
        assert all(item["bindings"] for item in reader.media("12345")["items"])


@pytest.mark.parametrize("declared", [2, 3])
def test_incomplete_manifest_cannot_replace_known_complete_members(tmp_path, declared):
    lib, state = sample(tmp_path)
    add_work(lib, state, pages=2)
    add_work(lib, state, pages=1, declared=declared, at="2026-10-03T03:00:00.000000Z")
    lib.sync_online()
    with Reader(lib.root, lib.cache) as reader:
        assert reader.work("12345")["manifest_state"] == "needs_refresh"
        assert len(reader.media("12345")["items"]) == 2


def test_archive_rebuild_uses_same_publication_and_preserves_raw(tmp_path):
    lib, state = sample(tmp_path)
    records, _, captured = add_work(lib, state)
    sha, data, end = add_images(lib, state, records)
    lib.sync_online()
    output = tmp_path / "rebuilt"
    result = rebuild(lib, output)
    assert result["state"] == "verified" and result["served_seq"] == end
    assert not (output / "ONLINE.json").exists()
    from studio_lake.util import atomic_json
    atomic_json(output / "ONLINE.json", result["pointer"])
    with Reader(lib.root, output) as reader:
        assert reader.blob(sha) == data
        assert reader.raw(captured["capture_id"])["json"].encode() == captured["raw_body"]
        assert len(reader.media("12345")["items"]) == 2


def test_immutable_conflict_is_rejected_before_journal_acceptance(tmp_path):
    lib, state = sample(tmp_path)
    records, seq, captured = add_work(lib, state)
    batch = new_batch(lib, state)
    batch.add("captures", [{**captured, "endpoint": "changed"}])
    with lib.writer_lock(), pytest.raises(IntegrityError, match="Conflicting immutable"):
        batch.commit()
    assert len(lib.commits()) == seq


def test_archive_fence_and_checkpoint_compare_and_swap(tmp_path):
    lib, state = sample(tmp_path)
    _, _, captured = add_work(lib, state)
    batch = new_batch(lib, state)
    batch.replay["checkpoint_advances"] = [dict(stream_key="author:10109777:directory", expected_revision=0,
         next_revision=1, next_cursor={}, exhausted=True, capture_id=captured["capture_id"])]
    with lib.writer_lock():
        manifest = batch.seal()
        with pytest.raises(RuntimeError, match="stale"):
            lib.accept_manifest(batch.path, manifest, fence=lambda _: (_ for _ in ()).throw(RuntimeError("stale")))
        seq = lib.accept_manifest(batch.path, manifest, fence=lambda _: None)
        assert lib.accept_manifest(lib.root / "segments" / batch.id, manifest, fence=lambda _: (_ for _ in ()).throw(RuntimeError("stale"))) == seq
    another = new_batch(lib, state)
    another.replay["checkpoint_advances"] = batch.replay["checkpoint_advances"]
    with lib.writer_lock(), pytest.raises(IntegrityError, match="compare-and-swap"):
        another.commit()
    assert len(lib.commits()) == seq
