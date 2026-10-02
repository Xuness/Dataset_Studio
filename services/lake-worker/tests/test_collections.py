import io
import json
import uuid

from PIL import Image
import pytest

from studio_lake.collectors.pixiv.http import Response
from studio_lake.collections.model import sample_definition
from studio_lake.collections.runner import Runner
from studio_lake.collections.service import Service
from studio_lake.media_lake.reader import Reader
from studio_lake.updates.resources import Resources
from studio_lake.updates.sites import UpdateError
from studio_lake.updates.state import State


def key():
    return str(uuid.uuid4())


class Client:
    requests = []

    def __init__(self, root, account, cookies, **kwargs):
        self.rate_root, self.name = root, "pixiv"

    def request(self, kind, payload):
        self.requests.append((kind, payload))
        if kind == "author_profile":
            body = dict(userId=payload["author_id"], name="紺屋 fixture")
        elif kind == "author_directory":
            body = dict(illusts={"12345": None}, manga={})
        elif kind == "work_detail":
            body = dict(id=payload["work_id"], userId="10109777", title="Fixture", description="",
                        illustType=0, pageCount=2, xRestrict=0, aiType=1, tags=dict(tags=[dict(tag="blue hair")]))
        elif kind == "media_manifest":
            body = [dict(width=12, height=9, urls=dict(original=f"https://i.pximg.net/12345_p{i}.png")) for i in range(2)]
        else:
            raise AssertionError(kind)
        return Response(json.dumps(dict(error=False, body=body)).encode(), "/fixture/" + kind, {}, 200,
                        "2026-10-03T00:00:00.000000Z", "identity")

    def close(self):
        pass


class HTTP:
    calls = 0

    def get(self, url, **kwargs):
        self.calls += 1
        data = io.BytesIO()
        Image.new("RGB", (12, 9), "blue").save(data, format="PNG")
        value = data.getvalue()

        class Reply:
            status_code = 200
            headers = {"Content-Length": str(len(value))}

            def __enter__(self):
                return self

            def __exit__(self, *_):
                pass

            def iter_content(self, size):
                yield value

        return Reply()


def setup(root, *, metadata_only=False):
    state = State(root / "control")
    service = Service(state)
    lake = service.create_lake(dict(request_key=key(), site="pixiv", media_root=str(root / "archive"), index_root=str(root / "online")))
    account = service.accounts.save(dict(request_key=key(), expected_revision=None, account_id=key(), label="public fixture", mode="anonymous"))
    definition = sample_definition(lake["library_id"], account["id"], ["10109777"], metadata_only=metadata_only)
    request = dict(request_key=key(), definition=definition)
    job = service.create_job(request)
    resources = Resources(reserve_bytes=0)
    runner = Runner(state, resources=resources, client_factory=Client, image_http=HTTP())
    return service, runner, job["job"], request


def test_author_collection_full_offline_roundtrip(tmp_path):
    service, runner, job, request = setup(tmp_path)
    result = runner.run(job["id"], time_slice=60)
    assert result["state"] == "completed_with_gaps", result
    assert result["progress"]["closure"] == dict(discovery_exhausted=True, directories_complete=True, manifests_complete=True, visibility_verified=False)
    assert result["progress"]["authors"]["scanned"] == 1
    assert result["progress"]["works"]["details"] == 1
    assert result["progress"]["media"]["downloaded"] == 2
    assert result["progress"]["media"]["published"] == 2
    assert result["progress"]["objects"]["stored"] == 1
    assert result["progress"]["download_bytes"] > 0
    replay = service.create_job(request)
    assert replay["replayed"] and replay["job"]["state"] == result["state"]
    with Reader(tmp_path / "archive", tmp_path / "online") as reader:
        items = reader.media("12345")["items"]
        assert len(items) == 2
        assert items[0]["bindings"][0]["asset_id"] != items[1]["bindings"][0]["asset_id"]
        assert items[0]["bindings"][0]["sha256"] == items[1]["bindings"][0]["sha256"]
    assert runner.resources.snapshot()["staging_reserved_bytes"] == 0


def test_metadata_only_does_not_claim_downloads(tmp_path):
    service, runner, job, _ = setup(tmp_path, metadata_only=True)
    result = runner.run(job["id"], time_slice=60)
    assert result["state"] == "completed_with_gaps", result
    assert result["progress"]["media"]["downloaded"] == 0
    assert result["progress"]["objects"]["stored"] == 0
    with Reader(tmp_path / "archive", tmp_path / "online") as reader:
        assert len(reader.media("12345")["items"]) == 2
        assert reader.objects()["items"] == []


def test_command_revision_idempotency_and_no_cancel_reactivation(tmp_path):
    service, _, job, _ = setup(tmp_path)
    args = dict(request_key=key(), expected_revision=job["revision"], action="pause")
    paused = service.action(job["id"], args)
    assert paused["job"]["state"] == "paused"
    assert service.action(job["id"], args)["replayed"]
    with pytest.raises(UpdateError) as conflict:
        service.action(job["id"], {**args, "action": "cancel"})
    assert conflict.value.code == "COLLECTION_IDEMPOTENCY_CONFLICT"
    cancelled = service.action(job["id"], dict(request_key=key(), expected_revision=paused["job"]["revision"], action="cancel"))
    with pytest.raises(UpdateError) as conflict:
        service.action(job["id"], dict(request_key=key(), expected_revision=cancelled["job"]["revision"], action="resume"))
    assert conflict.value.code == "COLLECTION_SCOPE_CHANGED"


def test_budget_resume_keeps_frozen_plan(tmp_path):
    service, runner, job, request = setup(tmp_path, metadata_only=True)
    request["request_key"] = key()
    request["definition"]["run_budget"]["api_requests"] = 2
    job = service.create_job(request)["job"]
    first = runner.run(job["id"], time_slice=60)
    assert first["state"] == "waiting_budget", first
    assert first["progress"]["authors"]["scanned"] == 1
    service.action(job["id"], dict(request_key=key(), expected_revision=first["revision"], action="resume"))
    second = runner.run(job["id"], time_slice=60)
    assert second["state"] == "completed_with_gaps", second
    assert second["progress"]["works"]["details"] == 1


def test_legacy_lakes_response_cannot_leak_pixiv_enum(tmp_path):
    from studio_lake.updates.__main__ import dispatch

    service, _, _, _ = setup(tmp_path)
    assert dispatch(service.state, "lakes", {})["items"] == []
    assert len(dispatch(service.state, "collection_lakes", {})["items"]) == 1
