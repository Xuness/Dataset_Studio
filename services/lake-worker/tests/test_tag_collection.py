"""Partial source discovery must preserve all evidence without broadening image selection."""

import json

import pytest

from conftest import png
from studio_lake.updates.archive import online
from studio_lake.updates.bootstrap import create
from studio_lake.updates.protocol import definition
from studio_lake.updates.sites import UpdateError
from studio_lake.updates.state import SCHEMA_VERSION, State
from studio_lake.util import FileLock
from test_empty_lakes import arguments
from test_updates import FakeSite, Images, Resources, Runner, job, post
from update_fixtures import remove_pinterest_schema


class TagSite(FakeSite):
    def __init__(self, site, records, *, page_size=200, ignore_tag=False):
        super().__init__(site, records)
        self.size, self.ignore_tag = page_size, ignore_tag

    def capabilities(self):
        return {**super().capabilities(), "page_size": self.size}

    def request(self, params, cancelled=lambda: False, resource="posts"):
        if resource != "posts":
            return super().request(params, cancelled, resource)
        if self.observer:
            self.observer(api_requests_delta=1)
        anchor = next((t for t in params.get("tags", "").split(" ") if t and ":" not in t), None)
        original = self.records
        try:
            if anchor and not self.ignore_tag:
                key = "tag_string" if self.name == "danbooru" else "tags"
                self.records = [r for r in original if anchor in r[key].split(" ")]
            return super().request(params, cancelled, resource)
        finally:
            self.records = original


def setup(root, site="danbooru", *, page_size=200, ignore_tag=False):
    state = State(root / "control")
    lake = create(state, arguments(root, site))
    lib = state.library(lake["id"])
    data = png("blue")
    rows = []
    for pid, tags in ((11, "a b c"), (50, "a c"), (5000, "a b blocked"), (9000, "b c")):
        row = post(site, pid, data)
        row["tag_string" if site == "danbooru" else "tags"] = tags
        rows.append(row)
    remote = TagSite(site, rows, page_size=page_size, ignore_tag=ignore_tag)
    images = Images(data)
    runner = Runner(state, {site: remote}, Resources(reserve_bytes=0), images)
    return lib, state, remote, images, runner


def scope(*, all_=(), any_=(), none=(), end=10000):
    return dict(kind="tags", query={"all": list(all_), "any": list(any_), "none": list(none)},
                start_id=1, end_id=end)


@pytest.mark.parametrize("site", ["danbooru", "gelbooru", "yandere"])
def test_and_uses_one_remote_tag_and_archives_nonmatching_metadata(tmp_path, site):
    lib, state, remote, images, runner = setup(tmp_path, site)
    lib.set_setting("update_new_metadata_cursor", 7)
    task = job(state, lib, scope(all_=["a", "b", "c"], none=["blocked"]), "original")
    done = runner.run(task["id"])
    assert done["state"] == "completed", done
    assert done["counts"] == {"excluded": 2, "stored": 1}
    assert done["cursor"]["tag_anchors"] == ["a"]
    assert all(params["tags"].split(" ")[0] == "a" for params in remote.calls)
    assert images.calls == 1
    with online(lib) as (db, _):
        assert list(db.execute("SELECT post_id,asset_id IS NOT NULL FROM post_versions WHERE valid_until IS NULL ORDER BY post_id")) == [(11, 1), (50, 0), (5000, 0)]
        assert next(db.execute("SELECT count(*) FROM raw_metadata"))[0] == 3
    assert lib.setting("update_new_metadata_cursor") == 7


def test_or_streams_deduplicate_media_admission_and_resume_at_saved_bounds(tmp_path):
    lib, state, remote, images, runner = setup(tmp_path, page_size=2)
    task = job(state, lib, scope(any_=["a", "b"], none=["blocked"]), "original", page_budget=1)
    for attempt in range(12):
        done = runner.run(task["id"])
        if done["state"] != "paused":
            break
        assert not done["cursor"]["metadata_complete"]
        state.action(task["id"], "resume")
    assert done["state"] == "completed", done
    assert done["counts"] == {"excluded": 1, "stored": 3}
    assert images.calls == 3
    assert len({json.dumps(p, sort_keys=True) for p in remote.calls}) == len(remote.calls)
    assert done["cursor"]["tag_branch"] == 2


def test_ignored_remote_predicate_keeps_evidence_without_advancing(tmp_path):
    lib, state, _, images, runner = setup(tmp_path, ignore_tag=True)
    task = job(state, lib, scope(all_=["a"]), "original")
    done = runner.run(task["id"])
    assert done["state"] == "needs_review" and done["error_code"] == "UPDATE_PAGE_INVALID"
    assert done["cursor"]["next_id"] == 1 and not done["cursor"]["metadata_complete"]
    assert images.calls == 0
    with online(lib) as (db, _):
        assert next(db.execute("SELECT count(*) FROM observations"))[0] == 0
    assert any((p / "response_body.bin").is_file() for p in (lib.root / "segments").iterdir())


def test_saved_unsealed_response_recovers_all_metadata_and_original_selection(tmp_path, monkeypatch):
    lib, state, remote, images, runner = setup(tmp_path)
    task = job(state, lib, scope(all_=["a", "b"], none=["blocked"], end=5001), "original")
    monkeypatch.setenv("DANBOORU_STORE_FAIL_AT", "after_update_response_saved")
    first = runner.run(task["id"])
    assert first["state"] == "needs_review"
    monkeypatch.delenv("DANBOORU_STORE_FAIL_AT")
    state.action(task["id"], "resume")
    done = runner.run(task["id"])
    assert done["state"] == "completed", done
    assert images.calls == 1 and len(remote.calls) == 1
    with online(lib) as (db, _):
        assert next(db.execute("SELECT count(*) FROM observations"))[0] == 3


def test_conflicting_query_finishes_without_network(tmp_path):
    lib, state, remote, images, runner = setup(tmp_path)
    task = job(state, lib, scope(all_=["a"], none=["a"]))
    assert runner.run(task["id"])["state"] == "completed"
    assert remote.calls == [] and images.calls == 0


@pytest.mark.parametrize("query", [{"none": ["a"]}, {"all": ["rating:g"]}, {"all": ["a b"]}, {"all": ["-a"]}, {"any": ["*"]}])
def test_literal_query_requires_bounded_positive_discovery(query):
    with pytest.raises(UpdateError):
        definition(dict(library_id="fixture", range=dict(kind="tags", query=query)))


def test_old_runner_cannot_share_new_tag_control_and_upgrade_preserves_jobs(tmp_path):
    lib, state, _, _, _ = setup(tmp_path)
    legacy = job(state, lib, dict(kind="ids", ids=[11]))
    with state.db() as db:
        remove_pinterest_schema(db)
        db.execute("PRAGMA user_version=11")
    with FileLock(state.root / "runner.lock", timeout=0):
        with pytest.raises(UpdateError, match="旧版"):
            State(state.root)
    reopened = State(state.root)
    assert reopened.job(legacy["id"])["definition"] == legacy["definition"]
    with reopened.db() as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == SCHEMA_VERSION
