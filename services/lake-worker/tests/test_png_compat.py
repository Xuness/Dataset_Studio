"""PNG recovery retains metadata evidence and never weakens pixel-data validation."""

import base64
from concurrent.futures import ThreadPoolExecutor
import hashlib
import io
import json
import struct
import zlib

import pytest
from PIL import Image, ImageCms, ImageFile, PngImagePlugin

from studio_lake.image_policy import prepare_image
from studio_lake import png_compat
from studio_lake.png_compat import (MAX_ICC_BYTES, MAX_METADATA_BYTES, MAX_RECOVERY_CHUNKS,
                                    PngCompatibilityError, inspect_path, open_image, scan)
from studio_lake.updates.media import paths
from studio_lake.updates.runner import Runner
from studio_lake.util import digest
from test_image_policy import policy
from test_updates import FakeSite, ImageResponse, online, post, setup


def png():
    stream = io.BytesIO()
    Image.new("RGBA", (16, 12), (40, 80, 120, 76)).save(stream, format="PNG")
    return stream.getvalue()


def chunk(kind, payload, bad=False):
    crc = zlib.crc32(kind + payload) & 0xFFFFFFFF
    return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", crc ^ int(bad))


def add(data, kind, payload, bad=False):
    return data[:33] + chunk(kind, payload, bad) + data[33:]


def profile(large=False):
    value = ImageCms.ImageCmsProfile(ImageCms.createProfile("sRGB")).tobytes()
    if large:
        value += b"\0" * (PngImagePlugin.MAX_TEXT_CHUNK + 300 - len(value))
        value = struct.pack(">I", len(value)) + value[4:]
    return value


def metadata(kind):
    if kind == b"iCCP":
        return b"ICC\0\0" + zlib.compress(profile())
    if kind == b"eXIf":
        exif = Image.Exif()
        exif[270] = "retained description"
        return exif.tobytes()[6:]
    if kind == b"zTXt":
        return b"Comment\0\0" + zlib.compress(b"retained text")
    if kind == b"iTXt":
        return b"Comment\0\1\0\0\0" + zlib.compress("保留文字".encode())
    return b"Comment\0retained text"


@pytest.mark.parametrize("kind", [b"iCCP", b"eXIf", b"tEXt", b"zTXt", b"iTXt"])
def test_metadata_crc_repair_retains_exact_chunk_payload_and_pixels(kind, tmp_path):
    payload = metadata(kind)
    original = add(png(), kind, payload, bad=True)
    original_digest = digest(original)
    path = tmp_path / "source.png"
    path.write_bytes(original)
    header = inspect_path(path)
    assert (header.width, header.height, header.frames) == (16, 12, 1)
    stored, _, details = prepare_image(original, policy("webp", lossless=True, max_edge=None))
    evidence = details["source_png_compatibility"]
    assert evidence["original_bytes_changed"] is False
    changed = evidence["changes"][0]
    assert changed["actions"] == ["metadata_crc_recomputed_for_decode"]
    assert base64.b64decode(changed["original_chunk"]["base64"]) == chunk(kind, payload, True)
    assert details["download_sha256"] == original_digest == digest(path.read_bytes())
    with Image.open(io.BytesIO(stored)) as decoded:
        assert decoded.convert("RGBA").getpixel((0, 0)) == (40, 80, 120, 76)
    info = details["source_image_info"]
    assert ("icc_profile" if kind == b"iCCP" else "exif" if kind == b"eXIf" else "Comment") in info


def test_large_icc_is_bounded_restored_and_preserved_without_global_changes(tmp_path):
    icc = profile(large=True)
    original = add(png(), b"iCCP", b"ICC\0\0" + zlib.compress(icc))
    path = tmp_path / "large-icc.png"
    path.write_bytes(original)
    limits = (PngImagePlugin.MAX_TEXT_CHUNK, PngImagePlugin.MAX_TEXT_MEMORY, ImageFile.LOAD_TRUNCATED_IMAGES)
    with pytest.raises(ValueError, match="MAX_TEXT_CHUNK"):
        Image.open(io.BytesIO(original))
    header = inspect_path(path)
    assert header.memory_bytes >= len(icc)
    stored, _, details = prepare_image(original, policy("webp", lossless=True))
    with Image.open(io.BytesIO(stored)) as decoded:
        assert decoded.info["icc_profile"] == icc
    assert base64.b64decode(details["source_image_info"]["icc_profile"]["base64"]) == icc
    assert details["source_png_compatibility"]["changes"][0]["actions"] == ["icc_decoded_separately"]
    assert limits == (PngImagePlugin.MAX_TEXT_CHUNK, PngImagePlugin.MAX_TEXT_MEMORY, ImageFile.LOAD_TRUNCATED_IMAGES)
    converted, _, _ = prepare_image(original, policy("png"))
    decoded, _ = open_image(converted)
    with decoded:
        decoded.load()
        assert decoded.info["icc_profile"] == icc


@pytest.mark.parametrize("kind", [b"IHDR", b"IDAT", b"IEND", b"PLTE", b"tRNS", b"gAMA", b"acTL", b"fcTL", b"fdAT"])
def test_pixel_palette_colour_and_animation_crc_failures_remain_errors(kind):
    data = png()
    offset = 8
    if kind in {b"PLTE", b"tRNS", b"gAMA", b"acTL", b"fcTL", b"fdAT"}:
        payload = struct.pack(">II", 1, 0) if kind == b"acTL" else b"\0" * 4
        damaged = add(data, kind, payload, bad=True)
    else:
        while data[offset + 4:offset + 8] != kind:
            offset += struct.unpack(">I", data[offset:offset + 4])[0] + 12
        end = offset + struct.unpack(">I", data[offset:offset + 4])[0] + 12
        damaged = data[:end - 1] + bytes([data[end - 1] ^ 1]) + data[end:]
    with pytest.raises(PngCompatibilityError, match="checksum"):
        prepare_image(damaged, policy())


@pytest.mark.parametrize("damage", ["truncated", "nonempty_iend", "unknown_critical", "duplicate_header"])
def test_structural_errors_are_not_recovered(damage):
    data = png()
    data = {"truncated": data[:-1], "nonempty_iend": data[:-12] + chunk(b"IEND", b"garbage"),
            "unknown_critical": add(data, b"ABCD", b"unknown"),
            "duplicate_header": data[:33] + data[8:33] + data[33:]}[damage]
    with pytest.raises(PngCompatibilityError):
        prepare_image(data, policy())


@pytest.mark.parametrize("recipe", [policy("webp", lossless=True, max_edge=None),
                                    {"profile": "original"}, {"profile": "webp-2048-q95"}])
def test_trailing_bytes_retained_exactly_without_entering_the_decoder(recipe, tmp_path):
    # A second PNG signature and arbitrary binary data remain opaque provenance.
    trailing = b"editor-data\0\xff" + png() + b"\r\n"
    original = png() + trailing
    path = tmp_path / "trailing.png"
    path.write_bytes(original)
    inspect_path(path)
    stored, _, details = prepare_image(original, recipe)
    evidence = details["source_png_compatibility"]
    changed = evidence["changes"][0]
    assert evidence["version"] == 3 and evidence["original_bytes_changed"] is False
    assert changed["offset"] == len(png()) and changed["bytes"] == len(trailing)
    assert changed["sha256"] == digest(trailing)
    assert base64.b64decode(changed["original_bytes"]["base64"]) == trailing
    assert details["source_decode_sha256"] == digest(png())
    assert details["download_sha256"] == digest(path.read_bytes()) == digest(original)
    if recipe["profile"] == "original":
        assert stored == original
    else:
        with Image.open(io.BytesIO(stored)) as decoded:
            decoded.load()
            assert decoded.size == (16, 12)
            assert decoded.convert("RGBA").getpixel((0, 0))[3] == 76


def unfinished_icc(icc):
    compressor = zlib.compressobj()
    # All ICC bytes are emitted, but the zlib end/checksum is absent.
    return b"ICC\0\0" + compressor.compress(icc) + compressor.flush(zlib.Z_SYNC_FLUSH)


@pytest.mark.parametrize("large", [False, True])
@pytest.mark.parametrize("recipe", [policy("webp", lossless=True), {"profile": "original"}])
def test_complete_icc_in_unfinished_stream_is_restored_with_original_evidence(large, recipe, tmp_path):
    icc = profile(large)
    payload = unfinished_icc(icc)
    original = add(png(), b"iCCP", payload)
    path = tmp_path / "unfinished-icc.png"
    path.write_bytes(original)
    limits = (PngImagePlugin.MAX_TEXT_CHUNK, PngImagePlugin.MAX_TEXT_MEMORY, ImageFile.LOAD_TRUNCATED_IMAGES)
    assert inspect_path(path).icc == icc
    stored, _, details = prepare_image(original, recipe)
    changed = details["source_png_compatibility"]["changes"][0]
    assert "incomplete_icc_stream_complete_profile_retained" in changed["actions"]
    assert base64.b64decode(changed["original_chunk"]["base64"]) == chunk(b"iCCP", payload)
    assert base64.b64decode(details["source_image_info"]["icc_profile"]["base64"]) == icc
    if recipe["profile"] == "original":
        assert stored == original
    else:
        with Image.open(io.BytesIO(stored)) as decoded:
            decoded.load()
            assert decoded.info["icc_profile"] == icc
            assert decoded.convert("RGBA").getpixel((0, 0)) == (40, 80, 120, 76)
    assert path.read_bytes() == original
    assert limits == (PngImagePlugin.MAX_TEXT_CHUNK, PngImagePlugin.MAX_TEXT_MEMORY, ImageFile.LOAD_TRUNCATED_IMAGES)


@pytest.mark.parametrize("damage", ["short_profile", "wrong_size", "signature", "table", "tag_bounds",
                                    "chunk_crc", "compressed_payload", "zlib_checksum", "zlib_trailing"])
def test_unfinished_icc_recovery_still_rejects_incomplete_or_damaged_profiles(damage):
    icc = profile()
    if damage == "short_profile":
        icc = icc[:-16]
    elif damage == "wrong_size":
        icc = struct.pack(">I", len(icc) + 1) + icc[4:]
    elif damage == "signature":
        icc = icc[:36] + b"nope" + icc[40:]
    elif damage == "table":
        icc = icc[:128] + b"\xff" * 4 + icc[132:]
    elif damage == "tag_bounds":
        icc = icc[:136] + struct.pack(">I", len(icc)) + icc[140:]
    payload = unfinished_icc(icc)
    if damage == "compressed_payload":
        payload = payload[:len(payload) // 2]
    elif damage == "zlib_checksum":
        payload = metadata(b"iCCP")
        payload = payload[:-1] + bytes([payload[-1] ^ 1])
    elif damage == "zlib_trailing":
        payload = metadata(b"iCCP") + b"extra"
    original = add(png(), b"iCCP", payload, bad=damage == "chunk_crc")
    with pytest.raises(PngCompatibilityError):
        prepare_image(original, policy())


def test_large_trailing_data_is_hashed_outside_the_metadata_budget(monkeypatch):
    monkeypatch.setattr(png_compat, "MAX_METADATA_BYTES", 256)
    monkeypatch.setattr(png_compat, "MAX_INLINE_EVIDENCE", 64)
    trailing = bytes(range(256)) * 8
    stored, _, details = prepare_image(png() + trailing, policy("webp", lossless=True, max_edge=None))
    changed = details["source_png_compatibility"]["changes"][0]
    assert changed["bytes"] == len(trailing) and changed["sha256"] == digest(trailing)
    assert "original_bytes" not in changed and len(json.dumps(details)) < len(trailing)
    with Image.open(io.BytesIO(stored)) as decoded:
        assert decoded.convert("RGBA").getpixel((0, 0)) == (40, 80, 120, 76)


def test_trailing_data_shares_the_recovery_count_budget():
    data = png()
    for _ in range(MAX_RECOVERY_CHUNKS):
        data = add(data, b"tEXt", b"Comment\0text", bad=True)
    with pytest.raises(PngCompatibilityError, match="recovery budget"):
        prepare_image(data + b"extra", policy())


def test_profile_expansion_metadata_and_repair_counts_are_bounded():
    oversized = add(png(), b"iCCP", b"ICC\0\0" + zlib.compress(b"x" * (MAX_ICC_BYTES + 1)))
    with pytest.raises(PngCompatibilityError, match="4 MiB"):
        prepare_image(oversized, policy())
    # The budget check must happen before reading an oversized colour payload.
    prefix = png()[:33] + struct.pack(">I4s", MAX_METADATA_BYTES + 1, b"iCCP")
    with pytest.raises(PngCompatibilityError, match="16 MiB"):
        scan(io.BytesIO(prefix), len(prefix) + MAX_METADATA_BYTES + 5)
    data = png()
    for _ in range(MAX_RECOVERY_CHUNKS + 1):
        data = add(data, b"tEXt", b"Comment\0text", bad=True)
    with pytest.raises(PngCompatibilityError, match="recovery budget"):
        prepare_image(data, policy())


@pytest.mark.parametrize("same", [True, False])
def test_duplicate_icc_keeps_the_first_profile_and_records_the_rest(same):
    first = profile()
    second = first if same else ImageCms.ImageCmsProfile(ImageCms.createProfile("LAB")).tobytes()
    data = add(add(png(), b"iCCP", b"Other\0\0" + zlib.compress(second)), b"iCCP", b"ICC\0\0" + zlib.compress(first))
    stored, _, details = prepare_image(data, policy("webp", lossless=True, max_edge=None))
    changed = details["source_png_compatibility"]["changes"]
    assert [c["actions"] for c in changed] == [["duplicate_icc_removed_for_decode"]]
    assert changed[0]["matches_retained_profile"] is same
    assert changed[0]["profile_sha256"] == digest(second)
    with Image.open(io.BytesIO(stored)) as decoded:
        assert decoded.info["icc_profile"] == first
    damaged = add(add(png(), b"iCCP", b"Other\0\0garbage"), b"iCCP", b"ICC\0\0" + zlib.compress(first))
    _, _, details = prepare_image(damaged, policy())
    assert "profile_error" in details["source_png_compatibility"]["changes"][0]


def test_opaque_chunks_are_removed_for_decode_when_damaged_or_oversized(monkeypatch):
    damaged = add(png(), b"prVt", b"private editor state", bad=True)
    stored, _, details = prepare_image(damaged, policy("webp", lossless=True, max_edge=None))
    changed = details["source_png_compatibility"]["changes"][0]
    assert changed["actions"] == ["opaque_chunk_crc_mismatch_removed_for_decode"]
    assert base64.b64decode(changed["original_chunk"]["base64"]) == chunk(b"prVt", b"private editor state", True)
    monkeypatch.setattr(png_compat, "MAX_METADATA_BYTES", 256)
    monkeypatch.setattr(png_compat, "MAX_INLINE_EVIDENCE", 64)
    text = b"Comment\0" + b"x" * 1024
    large = add(png(), b"tEXt", text)
    _, _, details = prepare_image(large, policy())
    changed = details["source_png_compatibility"]["changes"][0]
    assert changed["actions"] == ["oversized_opaque_chunk_removed_for_decode"]
    assert changed["original_chunk_bytes"] == len(text) + 12
    assert changed["original_chunk_sha256"] == hashlib.sha256(chunk(b"tEXt", text)).hexdigest()
    assert "Comment" not in details["source_image_info"]
    with pytest.raises(PngCompatibilityError, match="budget"):
        prepare_image(add(png(), b"iCCP", b"ICC\0\0" + bytes(range(256)) * 2), policy())


def test_original_and_animation_policies_keep_download_bytes():
    data = add(png(), b"tEXt", metadata(b"tEXt"), bad=True)
    original, _, details = prepare_image(data, {"profile": "original"})
    assert original == data and details["source_png_compatibility"]
    stream = io.BytesIO()
    Image.new("RGBA", (8, 8), "red").save(stream, format="PNG", save_all=True,
        append_images=[Image.new("RGBA", (8, 8), "blue")], duration=100, loop=0)
    animated = add(stream.getvalue(), b"tEXt", metadata(b"tEXt"), bad=True) + b"editor-data"
    result, _, details = prepare_image(animated, policy())
    assert result == animated and details["animation_preserved"] and details["frames"] == 2


def test_parallel_recovery_does_not_modify_other_decoders():
    limits = (PngImagePlugin.MAX_TEXT_CHUNK, ImageFile.LOAD_TRUNCATED_IMAGES)
    data = add(png(), b"iCCP", b"ICC\0\0" + zlib.compress(profile(True)), bad=True)
    with ThreadPoolExecutor(max_workers=4) as pool:
        results = list(pool.map(lambda _: prepare_image(data, policy()), range(12)))
    assert len({digest(r[0]) for r in results}) == 1
    assert limits == (PngImagePlugin.MAX_TEXT_CHUNK, ImageFile.LOAD_TRUNCATED_IMAGES)


def test_pipeline_header_admission_uses_same_png_compatibility_as_encoding(tmp_path):
    lib, state = setup(tmp_path, "gelbooru")
    payloads = {11: add(png(), b"tEXt", metadata(b"tEXt"), bad=True),
                12: add(png(), b"iCCP", b"ICC\0\0" + zlib.compress(profile(True))),
                13: png()[:-1], 14: png() + b"editor-data",
                15: add(png(), b"iCCP", unfinished_icc(profile())),
                16: add(add(png(), b"iCCP", metadata(b"iCCP")), b"iCCP", metadata(b"iCCP")),
                17: png()[:-1] + b"?"}
    records = [{**post("gelbooru", pid, data), "file_url": f"https://example.invalid/{pid}.png"}
               for pid, data in payloads.items()]
    records[-1]["md5"] = None
    by_url = {r["file_url"]: payloads[r["id"]] for r in records}

    class Images:
        def get(self, url, **_):
            return ImageResponse(by_url[url])

    task = state.create({"library_id": lib.info["library_id"],
                         "range": {"kind": "id_range", "start": 11, "end": 18},
                         "media": policy()}, "png-recovery")
    runner = Runner(state, {"gelbooru": FakeSite("gelbooru", records)}, image_http=Images())
    done = runner.run(task["id"])
    assert done["counts"] == {"stored": 5, "unavailable": 1, "needs_review": 1}, done
    assert not runner.resources.reservations
    with online(lib) as (db, _):
        details = [json.loads(r[0]) for r in db.execute("SELECT details_json FROM assets")]
    assert len(details) == 5 and all(d["source_png_compatibility"] for d in details)
    # A verified source failure is excluded with its original retained for a later local retry.
    excluded = state.items(task["id"], status="unavailable")["items"][0]
    assert (excluded["post_id"], excluded["reason"]) == (13, "image_source_incompatible")
    directory, key = paths(lib, task, excluded)
    assert (directory / (key + ".downloaded")).read_bytes() == payloads[13]
    # Unverified bytes may be a damaged transfer: review it and download again on retry.
    review = state.items(task["id"], status="needs_review")["items"][0]
    assert (review["post_id"], review["reason"]) == (17, "image_source_incompatible")
    directory, key = paths(lib, task, review)
    assert not (directory / (key + ".downloaded")).exists()
    with lib.journal() as db:
        sources = [json.loads(r[0])["source"] for r in db.execute("SELECT manifest_json FROM commits")]
    details = {r["post_id"]: r.get("detail") for s in sources if s.get("update_role") == "media"
               for r in s["results"]}
    assert details[13].startswith("PngCompatibilityError: ")
