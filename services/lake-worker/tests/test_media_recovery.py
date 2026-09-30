"""Single-file color errors and recoverable CDN failures must not strand a lake."""

import io
import json
from concurrent.futures import ThreadPoolExecutor
from types import SimpleNamespace

import pytest
from PIL import Image, ImageCms

from studio_lake.image_policy import prepare_image
from studio_lake.updates import rate
from studio_lake.updates.media import paths
from studio_lake.updates.runner import Runner
from studio_lake.util import read_json
from test_image_policy import policy
from test_updates import FakeSite, ImageResponse, Images, job, png, post, setup


@pytest.mark.parametrize("mode", ["L", "LA"])
@pytest.mark.parametrize("fmt", ["webp", "png", "jpeg"])
def test_grayscale_with_rgb_profile_retains_color_and_alpha(mode, fmt):
    icc = ImageCms.ImageCmsProfile(ImageCms.createProfile("sRGB")).tobytes()
    image = Image.new(mode, (12, 9), 127 if mode == "L" else (127, 43))
    original = io.BytesIO()
    image.save(original, format="JPEG" if mode == "L" else "PNG", icc_profile=icc)
    selected = policy(fmt, max_edge=None, **({"lossless": True} if fmt == "webp" else {}))
    stored, _, details = prepare_image(original.getvalue(), selected)
    assert details["color_profile_action"] == "expanded_grayscale_to_rgb"
    assert details["processing_version"] == 5
    with Image.open(io.BytesIO(stored)) as decoded:
        assert decoded.info["icc_profile"] == icc
        if fmt != "jpeg":
            assert decoded.convert("RGBA").getpixel((0, 0)) == (127, 127, 127, 43 if mode == "LA" else 255)
        else:
            expected = 127 if mode == "L" else 233
            assert all(abs(channel - expected) <= 2 for channel in decoded.getpixel((0, 0)))


def test_bad_color_transform_is_one_review_item_and_other_images_publish(tmp_path):
    lib, state = setup(tmp_path, "gelbooru")
    stream = io.BytesIO()
    # A LAB profile cannot describe CMYK pixels. Do not silently discard it.
    icc = ImageCms.ImageCmsProfile(ImageCms.createProfile("LAB")).tobytes()
    Image.new("CMYK", (16, 12), (0, 0, 0, 64)).save(stream, format="JPEG", icc_profile=icc)
    payloads = {11: stream.getvalue(), 12: png("blue"), 13: png("red")}
    records = [post("gelbooru", pid, data) for pid, data in payloads.items()]
    for record in records:
        record["file_url"] = f"https://example.invalid/{record['id']}.jpg"
    by_url = {r["file_url"]: payloads[r["id"]] for r in records}

    class MixedImages:
        def get(self, url, **_):
            return ImageResponse(by_url[url])

    task = state.create({"library_id": lib.info["library_id"],
                         "range": {"kind": "id_range", "start": 11, "end": 14},
                         "media": policy()}, "color-failure")
    runner = Runner(state, {"gelbooru": FakeSite("gelbooru", records)}, image_http=MixedImages())
    done = runner.run(task["id"])
    assert done["state"] == "needs_review" and done["error_code"] == "UPDATE_MEDIA_INCOMPLETE", done
    assert done["counts"] == {"stored": 2, "needs_review": 1}
    bad = state.items(task["id"], status="needs_review")["items"][0]
    assert bad["post_id"] == 11 and bad["reason"] == "image_color_profile_error"
    directory, key = paths(lib, task, bad)
    assert (directory / (key + ".downloaded")).read_bytes() == payloads[11]
    assert not runner.resources.reservations and not (state.root / "errors.jsonl").exists()


def test_404_is_retryable_and_manual_retry_reuses_saved_metadata(tmp_path):
    lib, state = setup(tmp_path, "gelbooru")
    data = png("blue")
    remote = FakeSite("gelbooru", [post("gelbooru", 11, data)])

    class RecoveringImages(Images):
        def get(self, *args, **kwargs):
            result = super().get(*args, **kwargs)
            if self.calls <= 2:
                result.status_code = 404
            return result

    images = RecoveringImages(data)
    task = job(state, lib, {"kind": "ids", "ids": [11]}, "original")
    runner = Runner(state, {"gelbooru": remote}, image_http=images)
    failed = runner.run(task["id"])
    assert failed["state"] == "waiting_retry" and failed["counts"] == {"failed": 1}, failed
    item = state.items(task["id"])["items"][0]
    assert item["reason"] == "image_http_404" and item["retry_at"] > 0
    calls = len(remote.calls)
    queued = state.action(task["id"], "retry")
    assert queued["counts"] == {"pending": 1}
    # A second identical 404 must become a new durable result after resetting
    # the retry budget; an old attempt-1 manifest must not leave it pending.
    with ThreadPoolExecutor(max_workers=1) as executor:
        future = executor.submit(runner.run, task["id"])
        try:
            repeated = future.result(timeout=10)
        finally:
            runner.stop.set()
    assert repeated["counts"] == {"failed": 1} and repeated["state"] == "waiting_retry"
    runner.stop.clear()
    state.action(task["id"], "retry")
    done = runner.run(task["id"])
    assert done["state"] == "completed" and done["counts"] == {"stored": 1}, done
    assert images.calls == 3 and len(remote.calls) == calls
    with lib.journal() as db:
        sources = [json.loads(r[0])["source"] for r in db.execute("SELECT manifest_json FROM commits")]
    results = [s for s in sources if s.get("update_role") == "media"]
    assert [s["update_execution"] for s in results] == [0, 1, 2]


def test_retry_unsupported_media_rechecks_locally_without_api_or_duplicate_checkpoint(tmp_path):
    lib, state = setup(tmp_path, "gelbooru")
    data = png("red")
    record = {**post("gelbooru", 11, data), "file_ext": "mp4", "image": "original.mp4",
              "file_url": "https://example.invalid/11.mp4"}
    remote = FakeSite("gelbooru", [record])
    images = Images(data)
    runner = Runner(state, {"gelbooru": remote}, image_http=images)
    task = job(state, lib, {"kind": "ids", "ids": [11]}, "original")
    assert runner.run(task["id"])["counts"] == {"unavailable": 1}
    calls = len(remote.calls)
    state.action(task["id"], "retry")
    with ThreadPoolExecutor(max_workers=1) as executor:
        future = executor.submit(runner.run, task["id"])
        try:
            done = future.result(timeout=10)
        finally:
            runner.stop.set()
    assert done["state"] == "completed_with_exclusions" and done["counts"] == {"unavailable": 1}
    assert len(remote.calls) == calls and images.calls == 0


def test_retry_refreshes_exhausted_or_missing_metadata_but_not_local_processing_errors(tmp_path):
    lib, state = setup(tmp_path, "gelbooru")
    task = job(state, lib, {"kind": "ids", "ids": [11]}, "original")
    state.update(task["id"], state="needs_review")
    cases = [
        (11, "needs_review", "image_http_404", 1, "obs", "pending"),
        (12, "failed", "image_http_404", 8, "obs", "pending_metadata"),
        (13, "needs_review", "image_color_profile_error", 1, "obs", "pending"),
        (14, "unavailable", "unsupported_media", 1, "obs", "pending"),
        (15, "unavailable", "not_returned_by_api", 0, None, "pending_metadata"),
        (16, "unavailable", "no_image_url", 1, "obs", "pending_metadata"),
        (17, "needs_review", "metadata_not_available", 1, "obs", "pending_metadata"),
        (18, "stored", None, 1, "obs", "stored"),
    ]
    with state.db() as db:
        for pid, status, reason, attempts, observation, _ in cases:
            db.execute("INSERT INTO items(job_id,post_id,observation_id,record_json,state,reason,attempts) "
                       "VALUES(?,?,?,'{\"id\":1}',?,?,?)", (task["id"], pid, observation, status, reason, attempts))
    state.action(task["id"], "retry")
    assert {i["post_id"]: i["state"] for i in state.items(task["id"])["items"]} == {r[0]: r[-1] for r in cases}


def test_image_404_burst_cools_only_image_lane_and_retains_longer_cooldown(tmp_path, monkeypatch):
    clock = [1000.0]
    monkeypatch.setattr(rate, "time", SimpleNamespace(time=lambda: clock[0]))
    marker = tmp_path / "rate-image-gelbooru.json"
    marker.write_text(json.dumps({"next_at": 1001, "cooldown_until": 1200}))
    for _ in range(rate.NOT_FOUND_THRESHOLD):
        rate.image_not_found(tmp_path, "gelbooru")
    saved = read_json(marker)
    assert saved["cooldown_until"] == 1200 and saved["next_at"] == 1001
    assert saved["not_found_count"] == 0
    assert not (tmp_path / "rate-gelbooru.json").exists()
    clock[0] = 1300
    for _ in range(rate.NOT_FOUND_THRESHOLD):
        rate.image_not_found(tmp_path, "gelbooru")
    assert read_json(marker)["cooldown_until"] == 1360
    rate.image_not_found(tmp_path, "gelbooru")
    clock[0] += rate.NOT_FOUND_WINDOW + 1
    rate.image_not_found(tmp_path, "gelbooru")
    assert read_json(marker)["not_found_count"] == 1
