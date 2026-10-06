"""Metadata-only posts are selectable without pretending they are stored image objects."""

import hashlib

import pytest
import pyarrow as pa
import pyarrow.parquet as pq

from conftest import png
from studio_lake.hf_metadata import normalize_hf
from studio_lake.library import Batch
from studio_lake.metadata import asset
from studio_lake.updates.__main__ import dispatch
from studio_lake.updates.archive import online
from studio_lake.updates.catalog import read_page
from studio_lake.updates.sites import UpdateError
from test_tag_collection import setup, scope
from test_updates import Images, job


def seed(lib, state, runner):
    task = job(state, lib, scope(any_=["a", "b"]))
    assert runner.run(task["id"])["state"] == "completed"


@pytest.mark.parametrize("site", ["danbooru", "yandere", "gelbooru"])
def test_metadata_catalog_and_local_selection_download_without_another_api_request(tmp_path, site):
    lib, state, remote, images, runner = setup(tmp_path, site)
    seed(lib, state, runner)
    before = len(remote.calls)
    page = dispatch(state, "catalog", dict(library_id=lib.info["library_id"],
                                           query={"all": ["a", "c"]}, missing_media=True, limit=1))
    assert [r["post_id"] for r in page["items"]] == [11]
    assert page["items"][0]["has_media"] is False
    second = read_page(lib, dict(query={"all": ["a", "c"]}, version=page["version"], after=page["next_after"], limit=1))
    assert [r["post_id"] for r in second["items"]] == [50]
    selected = dict(kind="tags", source="local", query={}, post_ids=[50], version=page["version"])
    task = job(state, lib, selected, "original")
    done = runner.run(task["id"])
    assert done["state"] == "completed", done
    assert done["counts"] == {"stored": 1}
    assert len(remote.calls) == before and images.calls == 1
    assert read_page(lib, dict(query={}, post_ids=[50]))["items"][0]["has_media"] is True
    assert read_page(lib, dict(query={}, post_ids=[50], version=page["version"]))["items"][0]["has_media"] is False
    assert lib.setting("update_new_metadata_cursor") is None


def test_retained_metadata_page_does_not_mix_later_tag_observations(tmp_path):
    lib, state, remote, _, runner = setup(tmp_path)
    seed(lib, state, runner)
    old = read_page(lib, dict(query={"all": ["a", "c"]}, limit=1))
    for row in remote.records:
        if row["id"] == 50:
            row["tag_string"] = "a changed"
    refresh = job(state, lib, dict(kind="ids", ids=[50]))
    assert runner.run(refresh["id"])["state"] == "completed"
    retained = read_page(lib, dict(query={"all": ["a", "c"]}, limit=1, after=old["next_after"], version=old["version"]))
    assert retained["items"][0]["post_id"] == 50
    latest = read_page(lib, dict(query={"all": ["a", "c"]}))
    assert [r["post_id"] for r in latest["items"]] == [11]


def test_negative_local_query_is_bounded_and_budget_resumes(tmp_path):
    lib, state, remote, _, runner = setup(tmp_path)
    seed(lib, state, runner)
    before = len(remote.calls)
    first = read_page(lib, dict(query={"none": ["a"]}), scan_limit=2)
    assert first["items"] == [] and first["next_after"] == 50 and first["scanned"] == 2
    task = job(state, lib, dict(kind="tags", source="local", query={"none": ["a"]}), item_budget=2)
    paused = runner.run(task["id"])
    assert paused["state"] == "paused"
    assert paused["cursor"]["metadata_records"] == 2
    for _ in range(4):
        state.action(task["id"], "resume")
        done = runner.run(task["id"])
        if done["state"] != "paused":
            break
    assert done["state"] == "completed"
    assert done["counts"] == {"metadata": 1}
    assert len(remote.calls) == before


@pytest.mark.parametrize("args", [{"limit": 0}, {"limit": True}, {"after": False}, {"post_ids": [0]}, {"start_id": 0}, {"missing_media": "yes"}])
def test_catalog_rejects_ambiguous_bounds_and_flags(tmp_path, args):
    lib, _, _, _, _ = setup(tmp_path)
    with pytest.raises(UpdateError):
        read_page(lib, args)


def test_local_task_never_silently_substitutes_another_generation(tmp_path):
    lib, state, remote, _, runner = setup(tmp_path)
    task = job(state, lib, dict(kind="tags", source="local", query={}, version="online-v2:other:0"))
    done = runner.run(task["id"])
    assert done["state"] == "needs_review" and done["error_code"] == "SOURCE_CHANGED"
    assert not remote.calls


def test_same_observation_without_md5_reuses_its_proven_asset(tmp_path):
    lib, state, remote, images, runner = setup(tmp_path)
    remote.records[0].pop("md5")
    first = job(state, lib, dict(kind="ids", ids=[11]), "original")
    assert runner.run(first["id"])["state"] == "completed"
    before = len(remote.calls)
    task = job(state, lib, dict(kind="tags", source="local", query={}, post_ids=[11]), "original")
    assert runner.run(task["id"])["counts"] == {"reused": 1}
    assert images.calls == 1 and len(remote.calls) == before


def test_cached_missing_locator_refreshes_only_that_post_once(tmp_path):
    lib, state, remote, images, runner = setup(tmp_path)
    url = remote.records[0].pop("file_url")
    seed(lib, state, runner)
    remote.records[0]["file_url"] = url
    before = len(remote.calls)
    task = job(state, lib, dict(kind="tags", source="local", query={}, post_ids=[11]), "original")
    done = runner.run(task["id"])
    assert done["state"] == "completed", done
    assert done["counts"] == {"stored": 1}
    assert len(remote.calls) == before + 1 and images.calls == 1
    assert remote.calls[-1]["tags"].startswith("id:11 ")


def test_expired_cached_image_address_refreshes_metadata_once(tmp_path):
    lib, state, remote, _, runner = setup(tmp_path)
    seed(lib, state, runner)
    before = len(remote.calls)

    class ExpiredOnce(Images):
        def get(self, *args, **kwargs):
            reply = super().get(*args, **kwargs)
            if self.calls == 1:
                reply.status_code = 404
            return reply

    images = ExpiredOnce(png("blue"))
    runner.image_http = images
    task = job(state, lib, dict(kind="tags", source="local", query={}, post_ids=[11]), "original")
    done = runner.run(task["id"])
    assert done["state"] == "completed", done
    assert done["counts"] == {"stored": 1}
    assert len(remote.calls) == before + 1 and images.calls == 2


@pytest.mark.parametrize("existing_media", [True, False])
def test_imported_metadata_reuses_proven_image_or_refreshes_only_missing_locator(tmp_path, existing_media):
    lib, state, remote, images, runner = setup(tmp_path, "yandere")
    data = png("blue")
    record = dict(image_id=11, extra=dict(id=11, tags=["a", "b"], tags_character=[], rating="s",
                                        md5=hashlib.md5(data).hexdigest(), width=12, height=9))
    batch = Batch(lib, "legacy-hf-fixture", dict(kind="fixture"))
    source = pa.BufferOutputStream()
    pq.write_table(pa.Table.from_pylist([record]), source)
    # Real HF inputs already carry Parquet's nested-list field names.
    batch.add_source(pq.ParquetFile(pa.BufferReader(source.getvalue())).read())
    observation = normalize_hf(record, batch.key, 0, "hf_yandere_v1", 0)
    batch.observations.append(observation)
    if existing_media:
        sha, ext = batch.add_blob(data, "png", lambda _: None)
        batch.assets.append(asset(observation, sha, ext, len(data), "original", {"selected_url_kind": "original"}))
    batch.commit()
    task = job(state, lib, dict(kind="tags", source="local", query={}, post_ids=[11]), "original")
    done = runner.run(task["id"])
    assert done["state"] == "completed", done
    assert done["counts"] == ({"reused": 1} if existing_media else {"stored": 1})
    assert len(remote.calls) == (0 if existing_media else 1)
    assert images.calls == (0 if existing_media else 1)


def test_catalog_does_not_require_an_image_or_duplicate_observations(tmp_path):
    lib, state, _, _, runner = setup(tmp_path)
    seed(lib, state, runner)
    with online(lib) as (db, _):
        original = next(db.execute("SELECT count(*) FROM observations"))[0]
        assert next(db.execute("SELECT count(*) FROM objects"))[0] == 0
    page = read_page(lib, dict(query={"any": ["a", "b"], "none": ["blocked"]}))
    assert [r["post_id"] for r in page["items"]] == [11, 50, 9000]
    task = job(state, lib, dict(kind="tags", source="local", query={"any": ["a", "b"], "none": ["blocked"]}))
    assert runner.run(task["id"])["counts"] == {"metadata": 3}
    with online(lib) as (db, _):
        assert next(db.execute("SELECT count(*) FROM observations"))[0] == original
