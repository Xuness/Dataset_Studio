"""PNG recovery retains metadata evidence and never weakens pixel-data validation."""

import base64
from concurrent.futures import ThreadPoolExecutor
import io
import json
import struct
import zlib

import pytest
from PIL import Image, ImageCms, ImageFile, PngImagePlugin

from studio_lake.image_policy import prepare_image
from studio_lake.png_compat import (MAX_ICC_BYTES, MAX_METADATA_BYTES, MAX_RECOVERY_CHUNKS,
                                    PngCompatibilityError, inspect_path, open_image, scan)
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


@pytest.mark.parametrize("kind", [b"IHDR", b"IDAT", b"IEND", b"PLTE", b"tRNS", b"acTL", b"fcTL", b"fdAT"])
def test_pixel_palette_transparency_and_animation_crc_failures_remain_errors(kind):
    data = png()
    offset = 8
    if kind in {b"PLTE", b"tRNS", b"acTL", b"fcTL", b"fdAT"}:
        payload = struct.pack(">II", 1, 0) if kind == b"acTL" else b"\0" * 4
        damaged = add(data, kind, payload, bad=True)
    else:
        while data[offset + 4:offset + 8] != kind:
            offset += struct.unpack(">I", data[offset:offset + 4])[0] + 12
        end = offset + struct.unpack(">I", data[offset:offset + 4])[0] + 12
        damaged = data[:end - 1] + bytes([data[end - 1] ^ 1]) + data[end:]
    with pytest.raises(PngCompatibilityError, match="checksum"):
        prepare_image(damaged, policy())


@pytest.mark.parametrize("damage", ["truncated", "trailing", "unknown_critical", "duplicate_header"])
def test_structural_errors_are_not_recovered(damage):
    data = png()
    data = {"truncated": data[:-1], "trailing": data + b"garbage",
            "unknown_critical": add(data, b"ABCD", b"unknown"),
            "duplicate_header": data[:33] + data[8:33] + data[33:]}[damage]
    with pytest.raises(PngCompatibilityError):
        prepare_image(data, policy())


def test_profile_expansion_metadata_and_repair_counts_are_bounded():
    oversized = add(png(), b"iCCP", b"ICC\0\0" + zlib.compress(b"x" * (MAX_ICC_BYTES + 1)))
    with pytest.raises(PngCompatibilityError, match="4 MiB"):
        prepare_image(oversized, policy())
    # The budget check must happen before reading the oversized metadata payload.
    prefix = png()[:33] + struct.pack(">I4s", MAX_METADATA_BYTES + 1, b"tEXt")
    with pytest.raises(PngCompatibilityError, match="16 MiB"):
        scan(io.BytesIO(prefix), len(prefix) + MAX_METADATA_BYTES + 5)
    data = png()
    for _ in range(MAX_RECOVERY_CHUNKS + 1):
        data = add(data, b"tEXt", b"Comment\0text", bad=True)
    with pytest.raises(PngCompatibilityError, match="recovery budget"):
        prepare_image(data, policy())


def test_original_and_animation_policies_keep_download_bytes():
    data = add(png(), b"tEXt", metadata(b"tEXt"), bad=True)
    original, _, details = prepare_image(data, {"profile": "original"})
    assert original == data and details["source_png_compatibility"]
    stream = io.BytesIO()
    Image.new("RGBA", (8, 8), "red").save(stream, format="PNG", save_all=True,
        append_images=[Image.new("RGBA", (8, 8), "blue")], duration=100, loop=0)
    animated = add(stream.getvalue(), b"tEXt", metadata(b"tEXt"), bad=True)
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
                13: png()[:-1]}
    records = [{**post("gelbooru", pid, data), "file_url": f"https://example.invalid/{pid}.png"}
               for pid, data in payloads.items()]
    by_url = {r["file_url"]: payloads[r["id"]] for r in records}

    class Images:
        def get(self, url, **_):
            return ImageResponse(by_url[url])

    task = state.create({"library_id": lib.info["library_id"],
                         "range": {"kind": "id_range", "start": 11, "end": 14},
                         "media": policy()}, "png-recovery")
    runner = Runner(state, {"gelbooru": FakeSite("gelbooru", records)}, image_http=Images())
    done = runner.run(task["id"])
    assert done["counts"] == {"stored": 2, "needs_review": 1}, done
    assert not runner.resources.reservations
    with online(lib) as (db, _):
        details = [json.loads(r[0]) for r in db.execute("SELECT details_json FROM assets")]
    assert len(details) == 2 and all(d["source_png_compatibility"] for d in details)
