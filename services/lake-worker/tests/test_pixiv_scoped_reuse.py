"""Pixiv literal filters and metadata/media independence within author or work scopes."""

import copy
from dataclasses import replace
import json

from studio_lake.collections import incremental, receipts
from studio_lake.media_lake.reader import Reader
from studio_lake.media_lake.schema import utc
from test_collection_continuous import FreshClient, fresh_setup, next_job
from test_collections import key


class TaggedClient(FreshClient):
    ids = ["12345", "23456"]
    requests = []

    def request(self, kind, payload):
        reply = super().request(kind, payload)
        if kind == "work_detail":
            value = json.loads(reply.body)
            value["body"]["tags"] = dict(tags=[dict(tag="blue hair" if payload["work_id"] == "12345" else "landscape")])
            reply = replace(reply, body=json.dumps(value).encode())
        return replace(reply, observed_at=utc())


def scoped_setup(root, tags):
    service, runner, initial, request = fresh_setup(root)
    service.action(initial["id"], dict(request_key=key(), expected_revision=initial["revision"], action="cancel"))
    runner.run(initial["id"])
    runner.client_factory = TaggedClient
    request = copy.deepcopy(request)
    request["definition"]["scope"]["tags"] = tags
    job, request = next_job(service, request)
    TaggedClient.requests.clear()
    return service, runner, job, request


def test_literal_space_tag_filters_media_and_retains_other_work_details(tmp_path):
    service, runner, job, request = scoped_setup(tmp_path, dict(all=["blue hair"], any=[], none=[]))
    first = runner.run(job["id"], time_slice=60)
    assert first["state"] == "completed", first
    assert first["progress"]["works"]["details"] == 2
    assert first["progress"]["works"]["excluded"] == 1
    assert first["progress"]["media"]["downloaded"] == 2
    with Reader(tmp_path / "archive", tmp_path / "online") as reader:
        assert reader.work("23456")["observation"] is not None
    second, request = next_job(service, request)
    TaggedClient.requests.clear()
    same = runner.run(second["id"], time_slice=60)
    assert same["state"] == "completed", same
    assert [kind for kind, _ in TaggedClient.requests] == ["author_profile", "author_directory"]
    assert same["progress"]["works"]["retained"] == 2
    assert same["progress"]["works"]["excluded"] == 1
    request["definition"]["scope"]["tags"] = dict(all=["landscape"], any=[], none=[])
    third, _ = next_job(service, request)
    TaggedClient.requests.clear()
    other = runner.run(third["id"], time_slice=60)
    assert other["state"] == "completed", other
    assert [kind for kind, _ in TaggedClient.requests] == ["author_profile", "author_directory", "media_manifest"]
    assert other["progress"]["works"]["retained"] == 2
    assert other["progress"]["media"]["downloaded"] == 2


def test_metadata_only_work_snapshot_downloads_media_without_metadata_http(tmp_path):
    service, runner, first, request = fresh_setup(tmp_path, metadata_only=True)
    assert runner.run(first["id"], time_slice=60)["state"] == "completed"
    request["definition"]["seeds"] = dict(kind="works", ids=["12345"])
    request["definition"]["media"].update(retain_original=True, ugoira="archive_with_poster")
    request["definition"]["media"]["image_policy"]["profile"] = "original"
    second, _ = next_job(service, request)
    FreshClient.requests.clear()
    done = runner.run(second["id"], time_slice=60)
    assert done["state"] == "completed", done
    assert FreshClient.requests == []
    assert done["progress"]["works"]["retained"] == 1
    assert done["progress"]["media"]["downloaded"] == 2
    assert done["progress"]["media"]["planned"] == 2
    lib = service.state.library(first["library_id"])
    receipts.reconcile(service, lib)
    assert service.job(second["id"])["progress"] == done["progress"]


def test_retained_manifest_planning_is_paged_and_recoverable(tmp_path, monkeypatch):
    class ManyPages(FreshClient):
        requests = []
        def request(self, kind, payload):
            reply = super().request(kind, payload)
            value = json.loads(reply.body)
            if kind == "work_detail":
                value["body"]["pageCount"] = 70
            elif kind == "media_manifest":
                value["body"] = [dict(width=12, height=9, urls=dict(original=f"https://i.pximg.net/12345_p{i}.png")) for i in range(70)]
            return replace(reply, body=json.dumps(value).encode())

    service, runner, first, request = fresh_setup(tmp_path, metadata_only=True)
    runner.client_factory = ManyPages
    assert runner.run(first["id"], time_slice=60)["state"] == "completed"
    request["definition"]["seeds"] = dict(kind="works", ids=["12345"])
    request["definition"]["media"].update(retain_original=True, ugoira="archive_with_poster")
    request["definition"]["media"]["image_policy"]["profile"] = "original"
    second, request = next_job(service, request)
    lib = service.state.library(first["library_id"])
    context = service.accounts.session(first["account_id"])[2]
    retained = incremental.reusable_work(lib, "12345", request["definition"], context)
    a = incremental.manifest_page(lib, retained, request["definition"], context, -1)
    b = incremental.manifest_page(lib, retained, request["definition"], context, a["next_after"])
    assert len(a["members"]) == 64 and len(b["members"]) == 6 and b["next_after"] is None
    ManyPages.requests.clear()
    old = incremental.apply_summary
    interrupted = False
    def fail(db, job, task, summary, counters):
        nonlocal interrupted
        old(db, job, task, summary, counters)
        if summary.get("retained_manifest_page") and not interrupted:
            interrupted = True
            raise OSError("planned page control interrupted")
    with monkeypatch.context() as patch:
        patch.setattr(incremental, "apply_summary", fail)
        outcome = runner.run(second["id"], time_slice=60)
        assert outcome["state"] in {"needs_review", "waiting_resources"}
    current = service.job(second["id"])
    service.action(second["id"], dict(request_key=key(), expected_revision=current["revision"], action="resume"))
    for _ in range(6):
        outcome = runner.run(second["id"], time_slice=60)
        if outcome["state"] == "completed":
            break
    assert outcome["state"] == "completed", outcome
    assert ManyPages.requests == []
    assert outcome["progress"]["media"]["downloaded"] == 70
    assert outcome["progress"]["media"]["planned"] == 70


def test_missing_tag_list_is_a_gap_for_filtered_capture(tmp_path):
    class UnknownTags(TaggedClient):
        ids = ["12345"]
        def request(self, kind, payload):
            reply = super().request(kind, payload)
            if kind == "work_detail":
                value = json.loads(reply.body)
                value["body"].pop("tags")
                return replace(reply, body=json.dumps(value).encode())
            return reply
    service, runner, job, _ = scoped_setup(tmp_path, dict(none=["landscape"]))
    runner.client_factory = UnknownTags
    result = runner.run(job["id"], time_slice=60)
    assert result["state"] == "completed_with_gaps", result
    assert result["progress"]["media"]["downloaded"] == 0
    assert service.tasks(job["id"], dict(kind="work_detail"))["items"][0]["reason"] == "COLLECTION_TAGS_UNKNOWN"
