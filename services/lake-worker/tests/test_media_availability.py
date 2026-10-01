"""Source moderation status is metadata, not proof that an original is unavailable."""

import json
import zlib

import pytest

from studio_lake.updates import media
from studio_lake.updates.archive import online
from studio_lake.updates.runner import Runner
from test_image_policy import policy
from test_updates import FakeSite, Images, job, png, post, setup


@pytest.mark.parametrize("site", ["danbooru", "yandere", "gelbooru"])
@pytest.mark.parametrize("profile", ["original", "custom"])
def test_original_urls_determine_acquisition_and_source_flags_survive_publication(tmp_path, site, profile):
    lib, state = setup(tmp_path, site)
    data, records = png("blue"), []
    for deleted, banned in [(False, False), (True, False), (False, True), (True, True)]:
        for has_url in [True, False]:
            record = post(site, 11 + len(records), data)
            record.update(is_deleted=deleted, is_banned=banned)
            if site != "danbooru":
                record["status"] = "deleted" if deleted else "active"
            if not has_url:
                record.pop("file_url")
                # A sample is not an original when the task did not allow it.
                record["large_file_url"] = record["sample_url"] = "https://example.invalid/sample.png"
            records.append(record)
    video = {**post(site, 19, data), "file_ext": "mp4", "image": "original.mp4",
             "is_deleted": True, "is_banned": True, "status": "deleted"}
    records.append(video)
    selected = policy() if profile == "custom" else {"profile": "original", "allow_sample": False}
    task = state.create({"library_id": lib.info["library_id"],
                         "range": {"kind": "id_range", "start": 11, "end": 20},
                         "media": selected}, "flagged-originals")
    images = Images(data)
    done = Runner(state, {site: FakeSite(site, records)}, image_http=images).run(task["id"])
    assert done["state"] == "completed_with_exclusions", done
    assert done["counts"] == {"stored": 4, "unavailable": 5} and images.calls == 4
    missing = state.items(task["id"], status="unavailable")["items"]
    assert {r["post_id"]: r["reason"] for r in missing} == {
        12: "no_image_url", 14: "no_image_url", 16: "no_image_url", 18: "no_image_url",
        19: "unsupported_media",
    }
    with online(lib) as (db, _):
        observations = list(db.execute("SELECT post_id,is_deleted,is_banned FROM observations ORDER BY post_id"))
        assert observations == [(r["id"], r["is_deleted"], r["is_banned"]) for r in records]
        raws = [json.loads(zlib.decompress(r[0])) for r in db.execute("SELECT raw_zlib FROM raw_metadata")]
        assert sorted(raws, key=lambda r: r["id"]) == records
        assets = list(db.execute("SELECT post_id,stored_ext,details_json FROM assets ORDER BY post_id"))
        assert [r[0] for r in assets] == [11, 13, 15, 17]
        for _, ext, details in assets:
            assert ext == ("png" if profile == "original" else "webp")
            assert json.loads(details)["original_md5_verified"] is True
            assert json.loads(details)["selected_url_kind"] == "original"


def test_retry_legacy_flag_exclusions_refreshes_metadata_and_downloads_available_originals(tmp_path, monkeypatch):
    lib, state = setup(tmp_path, "danbooru")
    data = png("red")
    records = [post("danbooru", pid, data) for pid in [11, 12, 13]]
    records[0].update(is_deleted=True, is_banned=False)
    records[1].update(is_deleted=False, is_banned=True)
    records[2].update(is_deleted=True, is_banned=True)
    records[2].pop("file_url")
    remote, images = FakeSite("danbooru", records), Images(data)
    task = job(state, lib, {"kind": "ids", "ids": [11, 12, 13]}, "original")
    runner = Runner(state, {"danbooru": remote}, image_http=images)

    def legacy_inspection(_lib, _job, item, _site, _allow_external=False):
        record = json.loads(item["record_json"])
        return {"state": "unavailable",
                "reason": "source_deleted" if record["is_deleted"] else "source_restricted"}

    # Publish real old-policy receipts, so retry must drain and supersede them.
    with monkeypatch.context() as patch:
        patch.setattr(media, "inspect_download", legacy_inspection)
        assert runner.run(task["id"])["counts"] == {"unavailable": 3}
    assert images.calls == 0
    original_calls = len(remote.calls)
    queued = state.action(task["id"], "retry")
    assert queued["counts"] == {"pending_metadata": 3}
    done = runner.run(task["id"])
    assert done["state"] == "completed_with_exclusions", done
    assert done["counts"] == {"stored": 2, "unavailable": 1} and images.calls == 2
    assert len(remote.calls) > original_calls
    assert state.items(task["id"], status="unavailable")["items"][0]["reason"] == "no_image_url"
    with online(lib) as (db, _):
        expected = {r["id"]: (r["is_deleted"], r["is_banned"]) for r in records}
        for pid, deleted, banned in db.execute("SELECT post_id,is_deleted,is_banned FROM observations"):
            assert (deleted, banned) == expected[pid]
    # Another retry targets only the remaining gap, preserving published assets.
    state.action(task["id"], "retry")
    assert runner.run(task["id"])["counts"] == {"stored": 2, "unavailable": 1}
    assert images.calls == 2


@pytest.mark.parametrize("status,expected", [(403, "needs_review"), (404, "failed")])
def test_flagged_original_still_records_actual_http_failures(tmp_path, status, expected):
    lib, state = setup(tmp_path, "danbooru")
    data = png("red")
    record = {**post("danbooru", 11, data), "is_deleted": True, "is_banned": True}

    class DeniedImages(Images):
        def get(self, *args, **kwargs):
            response = super().get(*args, **kwargs)
            response.status_code = status
            return response

    images = DeniedImages(data)
    task = job(state, lib, {"kind": "ids", "ids": [11]}, "original")
    done = Runner(state, {"danbooru": FakeSite("danbooru", [record])}, image_http=images).run(task["id"])
    assert done["counts"] == {expected: 1} and images.calls == 1
    assert state.items(task["id"])["items"][0]["reason"] == f"image_http_{status}"
    with online(lib) as (db, _):
        assert next(db.execute("SELECT count(*) FROM assets"))[0] == 0
