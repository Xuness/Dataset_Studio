from update_fixtures import remove_collection_schema
import copy
import io
import json
import os
import subprocess
import sys
from pathlib import Path

import pytest
from PIL import Image, ImageCms

from studio_lake.image_policy import ImagePolicyError, encoding, media_policy, prepare_image, profile_id
from studio_lake.ingest import prepare_image as legacy_image
from studio_lake.updates.sites import UpdateError
from studio_lake.updates.state import State
from studio_lake.util import FileLock
from test_updates import FakeSite, Images, Resources, Runner, online, png, post, setup


def policy(fmt="webp", **options):
    base = {"version": 1, "format": fmt, "max_edge": 64, "animation": "preserve",
            "alpha": "flatten" if fmt == "jpeg" else "preserve"}
    if fmt == "jpeg":
        base["background"] = "#FFFFFF"
    return media_policy({"profile": "custom", "existing": "match_profile", "encoding": {**base, **options}})


def pixels(mode="RGB", size=(160, 80), color="red", **kwargs):
    output = io.BytesIO()
    Image.new(mode, size, color).save(output, format="PNG", **kwargs)
    return output.getvalue()


@pytest.mark.parametrize("fmt,options", [("webp", {"quality": 72}), ("webp", {"lossless": True}),
                                        ("jpeg", {"quality": 85, "subsampling": "420"}),
                                        ("png", {"compress_level": 9})])
def test_codec_dimensions_details_and_no_upscale(fmt, options):
    p = policy(fmt, **options)
    stored, ext, details = prepare_image(pixels(), p)
    with Image.open(io.BytesIO(stored)) as im:
        assert im.format.lower() == fmt
        assert im.size == (64, 32)
    assert ext == ("jpg" if fmt == "jpeg" else fmt)
    assert details["encoding"] == p["encoding"]
    assert details["storage_profile"] == profile_id(p)
    assert details["codec_version"]
    tiny, _, _ = prepare_image(pixels(size=(12, 6)), p)
    with Image.open(io.BytesIO(tiny)) as im:
        assert im.size == (12, 6)
    full, _, _ = prepare_image(pixels(), policy(fmt, max_edge=None, **options))
    with Image.open(io.BytesIO(full)) as im:
        assert im.size == (160, 80)


def test_transparency_preserve_flatten_and_reject():
    data = pixels("RGBA", (20, 10), (230, 120, 20, 0))
    stored, _, _ = prepare_image(data, policy("webp", lossless=True, max_edge=None))
    with Image.open(io.BytesIO(stored)) as im:
        assert im.convert("RGBA").getpixel((0, 0)) == (230, 120, 20, 0)
    for fmt in ("png", "jpeg"):
        stored, _, _ = prepare_image(data, policy(fmt, alpha="flatten", background="#00FF00"))
        with Image.open(io.BytesIO(stored)) as im:
            assert all(abs(a-b) <= 2 for a, b in zip(im.convert("RGB").getpixel((0, 0)), (0, 255, 0)))
    with pytest.raises(ImagePolicyError, match="transparent_image_rejected"):
        prepare_image(data, policy(alpha="reject"))
    prepare_image(pixels("RGBA", color=(1, 2, 3, 255)), policy(alpha="reject"))


def test_animation_is_explicit_and_exif_orientation_is_applied():
    stream = io.BytesIO()
    Image.new("RGB", (24, 12), "red").save(stream, format="GIF", save_all=True,
        append_images=[Image.new("RGB", (24, 12), "blue")], duration=100, loop=0)
    data = stream.getvalue()
    stored, ext, details = prepare_image(data, policy(max_edge=8))
    assert stored == data and ext == "gif" and details["encoding_applied"] is False
    stored, ext, details = prepare_image(data, policy(animation="first_frame", max_edge=8))
    with Image.open(io.BytesIO(stored)) as im:
        assert im.size == (8, 4) and getattr(im, "n_frames", 1) == 1
    assert details["frame_selected"] == 0
    exif = Image.Exif()
    exif[274] = 6
    stored, _, details = prepare_image(pixels(size=(20, 10), exif=exif), policy("png", max_edge=None))
    with Image.open(io.BytesIO(stored)) as im:
        assert im.size == (10, 20)
    assert details["source_width"] == 20


def test_recipe_identity_normalizes_defaults_and_is_independent_of_acquisition():
    p = policy()
    assert profile_id(p) == profile_id({**p, "existing": "keep", "allow_sample": True})
    assert encoding({"format": "webp", "version": 1, "max_edge": 64}) == p["encoding"]
    for changed in (policy(quality=80), policy(max_edge=65), policy(lossless=True),
                    policy(animation="first_frame"), policy(alpha="reject"), policy(method=0)):
        assert profile_id(changed) != profile_id(p)
    for profile in ("original", "webp-2048-q95"):
        data = png("blue")
        assert prepare_image(data, {"profile": profile}) == legacy_image(data, profile)


def test_color_profiles_preserved_or_converted_with_pixels():
    rgb = ImageCms.ImageCmsProfile(ImageCms.createProfile("sRGB")).tobytes()
    source = pixels(icc_profile=rgb)
    for fmt in ("webp", "jpeg", "png"):
        stored, _, _ = prepare_image(source, policy(fmt))
        with Image.open(io.BytesIO(stored)) as im:
            assert im.info["icc_profile"] == rgb
    lab = ImageCms.ImageCmsProfile(ImageCms.createProfile("LAB")).tobytes()
    output = io.BytesIO()
    Image.new("LAB", (12, 9), (160, 128, 128)).save(output, format="TIFF", icc_profile=lab)
    stored, _, _ = prepare_image(output.getvalue(), policy("png"))
    with Image.open(io.BytesIO(stored)) as im:
        assert im.mode == "RGB" and im.info["icc_profile"] != lab


@pytest.mark.parametrize("bad", [
    {"version": 2}, {"format": "avif"}, {"quality": 101}, {"quality": True},
    {"max_edge": 0}, {"max_edge": 40000}, {"method": 7}, {"unknown": 1},
    {"lossless": True, "quality": 95}, {"alpha": "flatten", "background": "white"},
    {"format": "png", "quality": 95}, {"format": "jpeg", "alpha": "preserve"},
])
def test_reject_invalid_or_inapplicable_parameters(bad):
    with pytest.raises(ImagePolicyError):
        policy(**bad)


@pytest.mark.parametrize("site", ["danbooru", "yandere", "gelbooru"])
def test_custom_versions_archive_reuse_and_resume(site, tmp_path):
    lib, state = setup(tmp_path, site)
    data = pixels(size=(120, 90))
    images = Images(data)
    runner = Runner(state, {site: FakeSite(site, [post(site, 11, data)])}, Resources(reserve_bytes=0), images)
    p = policy("png", max_edge=32)
    spec = {"library_id": lib.info["library_id"], "range": {"kind": "ids", "ids": [11]}, "media": p}
    task = state.create(spec, "first")
    assert runner.run(task["id"])["counts"] == {"stored": 1}
    spec["media"] = policy("png", max_edge=64)
    assert state.job(task["id"])["definition"]["media"] == p
    upgraded = state.create(spec, "upgrade")
    state.action(upgraded["id"], "pause")
    state = State(state.root)
    state.action(upgraded["id"], "resume")
    assert Runner(state, runner.sites, runner.resources, images).run(upgraded["id"])["counts"] == {"stored": 1}
    duplicate = state.create(spec, "reuse")
    assert runner.run(duplicate["id"])["counts"] == {"reused": 1}
    assert images.calls == 2
    with online(lib) as (db, _):
        assets = list(db.execute("SELECT storage_profile,details_json FROM assets ORDER BY commit_seq"))
        assert len(assets) == 2 and len({r[0] for r in assets}) == 2
        assert [json.loads(r[1])["stored_width"] for r in assets] == [32, 64]
    keep = copy.deepcopy(spec)
    keep["media"] = policy("jpeg")
    keep["media"]["existing"] = "keep"
    assert runner.run(state.create(keep, "keep")["id"])["counts"] == {"reused": 1}
    assert images.calls == 2


def test_schema3_handoff_requires_old_worker_to_stop_and_retains_jobs(tmp_path):
    lib, state = setup(tmp_path, "danbooru")
    spec = {"library_id": lib.info["library_id"], "range": {"kind": "ids", "ids": [11]}, "media": {"profile": "original"}}
    task = state.create(spec, "legacy")
    with state.db() as db:
        remove_collection_schema(db)
        db.execute("PRAGMA user_version=3")
    with FileLock(state.root / "runner.lock"):
        with pytest.raises(UpdateError, match="旧版"):
            State(state.root)
    upgraded = State(state.root)
    assert upgraded.job(task["id"])["definition"] == task["definition"]
    assert upgraded.create(spec, "legacy")["id"] == task["id"]
    with upgraded.db() as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == 11


def test_worker_entrypoint_isolated_from_other_repository(tmp_path):
    entry = Path(__file__).resolve().parents[1] / "worker.py"
    request = json.dumps({"protocol_version": 1, "command": "status", "arguments": {}})
    result = subprocess.run([sys.executable, "-I", str(entry), "--root", str(tmp_path / "controller"), "--mode", "rpc"],
        input=request, text=True, capture_output=True, check=True, cwd=tmp_path,
        env={**os.environ, "PYTHONPATH": str(tmp_path / "nonexistent-external-store")}, timeout=30)
    assert json.loads(result.stdout)["ok"] is True
    assert not any(name == "danbooru_store" or name.startswith("danbooru_store.") for name in sys.modules)
