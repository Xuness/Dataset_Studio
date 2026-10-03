import html
import io
import json
import shutil
import zipfile

from PIL import Image
import pytest
import requests

from test_collections import Client, HTTP, key, setup
from studio_lake.collectors.pixiv import normalize
from studio_lake.collectors.pixiv.http import Client as WebClient, MediaSessions, Response
from studio_lake.collections.runner import Runner
from studio_lake.media_lake.reader import Reader
from studio_lake.updates import dispatch, relocation
from studio_lake.updates.resources import Resources
from studio_lake.updates.sites import UpdateError
from studio_lake.util import read_json


class Session(requests.Session):
    def __init__(self, body, status=200, headers=None):
        super().__init__()
        self.body, self.status, self.response_headers = body, status, headers or {}
        self.calls = []

    def get(self, url, **kwargs):
        self.calls.append((url, kwargs))
        body, status, headers = self.body, self.status, self.response_headers

        class Reply:
            status_code = status

            def __enter__(self):
                return self

            def __exit__(self, *_):
                pass

            def iter_content(self, size):
                yield body

        reply = Reply()
        reply.headers = headers
        return reply


def cookie():
    return dict(name="PHPSESSID", value="4242_FAKE_TEST_ONLY", domain=".pixiv.net", path="/",
                secure=True, http_only=True, expires_unix=None)


def test_session_identity_requires_server_confirmation_and_media_has_no_cookie(tmp_path):
    account = dict(viewer_key="offline-account", mode="session")
    session = Session(b'<html><meta id="meta-global-data" content="{}"></html>')
    client = WebClient(None, account, [cookie()], session=session)
    with pytest.raises(UpdateError) as missing:
        client.probe()
    assert missing.value.code == "COLLECTION_CREDENTIAL_REQUIRED"
    global_data = html.escape(json.dumps(dict(userData=dict(id="4242"))), quote=True)
    session.body = f'<meta id="meta-global-data" content="{global_data}">'.encode()
    context = client.probe()
    policy = json.loads(context["policy_json"])
    assert policy["user_id"] == "4242" and policy["login"] == "authenticated"
    assert not context["verified"] and policy["r18"] == "unknown"
    assert all(call[1]["allow_redirects"] is False for call in session.calls)
    media = MediaSessions()
    assert not list(media.get().cookies)
    media.close()
    client.close()


@pytest.mark.parametrize("status,body,code", [(403, b"challenge", "COLLECTION_CREDENTIAL_REQUIRED"),
                                               (302, b"", "COLLECTION_CREDENTIAL_REQUIRED"),
                                               (429, b"", "COLLECTION_REMOTE_UNAVAILABLE"),
                                               (200, b"<html>login</html>", "COLLECTION_CREDENTIAL_REQUIRED")])
def test_web_errors_never_turn_into_empty_public_results(status, body, code):
    client = WebClient(None, dict(viewer_key="fake"), session=Session(body, status, {"Retry-After": "17"}))
    with pytest.raises(UpdateError) as error:
        client.request("author_directory", dict(author_id="10109777"))
    assert error.value.code == code
    if status == 429:
        assert error.value.retry_after == 17
    assert len(client.session.calls) == 1


def test_protected_credentials_account_binding_and_revision_fence(tmp_path):
    service, _, _, _ = setup(tmp_path)
    account = service.accounts.save(dict(request_key=key(), expected_revision=None, account_id=key(),
                                         label="session fixture", mode="session", cookies=[cookie()]))
    assert account["state"] == "unverified"
    with pytest.raises(UpdateError):
        service.accounts.session(account["id"])
    with service.state.db() as db:
        saved = db.execute("SELECT secret_blob FROM collection_accounts WHERE id=?", (account["id"],)).fetchone()[0]
        assert cookie()["value"].encode() not in saved
        fingerprints = list(db.execute("SELECT request_hash FROM collection_requests"))
        assert all(cookie()["value"] not in row[0] for row in fingerprints)

    class Probe:
        def probe(self):
            return normalize.visibility(service.accounts.row(account["id"])["viewer_key"], login="authenticated", user_id="4242")

    request = dict(request_key=key(), expected_revision=account["revision"])
    verified = service.accounts.probe(account["id"], request, client=Probe())
    assert verified["account"]["bound_user_id"] == "4242"
    assert service.accounts.probe(account["id"], request, client=Probe())["visibility"] == verified["visibility"]
    old_revision = verified["account"]["revision"]
    replacement = service.accounts.save(dict(request_key=key(), expected_revision=old_revision, account_id=account["id"],
                                             label="session fixture", mode="session", cookies=[{**cookie(), "value": "DIFFERENT_FAKE_TEST_ONLY"}]))
    assert not service.accounts.renew(account["id"], old_revision, [cookie()])

    class Different:
        def probe(self):
            return normalize.visibility("fake", login="authenticated", user_id="4243")

    with pytest.raises(UpdateError) as mismatch:
        service.accounts.probe(account["id"], dict(request_key=key(), expected_revision=replacement["revision"]), client=Different())
    assert mismatch.value.code == "COLLECTION_SCOPE_CHANGED"


def test_ugoira_preserves_zip_frames_and_browsable_poster(tmp_path):
    service, runner, job, _ = setup(tmp_path)

    class AnimationClient(Client):
        def request(self, kind, payload):
            response = super().request(kind, payload)
            body = json.loads(response.body)["body"]
            if kind == "work_detail":
                body.update(illustType=2, pageCount=1)
            elif kind == "media_manifest":
                body = dict(originalSrc="https://i.pximg.net/fixture.zip", frames=[dict(file=f"{i}.png", delay=100 + i) for i in range(2)])
            return Response(json.dumps(dict(error=False, body=body)).encode(), response.endpoint, {}, 200, response.observed_at, "identity")

    output = io.BytesIO()
    with zipfile.ZipFile(output, "w") as archive:
        for i, color in enumerate(("blue", "red")):
            image = io.BytesIO()
            Image.new("RGB", (12, 9), color).save(image, format="PNG")
            archive.writestr(f"{i}.png", image.getvalue())
    runner.client_factory = AnimationClient
    runner.image_http = Session(output.getvalue(), headers={"Content-Length": str(len(output.getvalue()))})
    result = runner.run(job["id"], time_slice=60)
    assert result["state"] == "completed", result
    assert result["progress"]["media"]["downloaded"] == 1
    with Reader(tmp_path / "archive", tmp_path / "online") as reader:
        assert len(reader.objects()["items"]) == 1
        media = reader.media("12345")["items"][0]
        assert media["kind"] == "ugoira"
        assert {r["representation"] for r in media["bindings"]} == {"original", "poster"}
        assert list(reader.db.execute("SELECT file_name,delay_ms FROM animation_frames ORDER BY ordinal")) == [("0.png", 100), ("1.png", 101)]


def test_historical_reuse_is_recorded_without_network_or_new_objects(tmp_path):
    service, runner, job, request = setup(tmp_path)
    assert runner.run(job["id"], time_slice=60)["state"] == "completed"
    request["request_key"] = key()
    request["definition"]["media"]["reuse"] = dict(mode="historical_if_same_locator", max_age_hours=8760)
    next_job = service.create_job(request)["job"]

    class NoDownload:
        def get(self, *_, **__):
            raise AssertionError("Historical reuse must not download bytes")

    runner.image_http = NoDownload()
    result = runner.run(next_job["id"], time_slice=60)
    assert result["progress"]["media"]["historical_reused"] == 2, result
    assert result["progress"]["media"]["downloaded"] == 0
    assert result["progress"]["objects"]["stored"] == 0
    assert result["progress"]["download_bytes"] == 0


def test_paginated_registries_and_shared_dispatch_fairness(tmp_path):
    service, runner, job, _ = setup(tmp_path)
    for _ in range(3):
        service.accounts.save(dict(request_key=key(), expected_revision=None, account_id=key(), label="public", mode="anonymous"))
    first = service.dispatch("accounts", dict(limit=2))
    second = service.dispatch("accounts", dict(limit=2, cursor=first["next_cursor"]))
    assert len({row["id"] for row in first["items"] + second["items"]}) == 4
    assert second["next_cursor"] is None
    with pytest.raises(UpdateError):
        service.dispatch("lakes", dict(cursor=first["next_cursor"]))
    with service.state.db() as db:
        db.execute("INSERT INTO lakes VALUES('legacy','yandere',?,?,'')", (str(tmp_path / "legacy"), str(tmp_path / "legacy-online")))
        db.execute("INSERT INTO jobs(id,lake_id,request_key,definition,state,cursor,created_at,updated_at) VALUES('legacy-job','legacy','legacy-key','{}','queued','{}','0','0')")
    candidates = dispatch.candidates(service.state, (), 2)
    assert {r["family"] for r in candidates} == {"collection", "update"}
    dispatch.submitted(service.state, "legacy")
    assert dispatch.candidates(service.state, (), 1)[0]["id"] == job["id"]
    dispatch.submitted(service.state, job["library_id"])
    assert dispatch.candidates(service.state, (), 1)[0]["id"] == "legacy-job"
    from studio_lake.updates.runner import Runner as SharedRunner
    shared = SharedRunner(service.state)
    assert shared.resources is shared.collections.resources and shared.stop is shared.collections.stop


def test_pixiv_relocation_preserves_snapshot_and_resumes_collection(tmp_path):
    service, runner, job, request = setup(tmp_path)
    runner.run(job["id"], time_slice=60)
    lib = service.state.library(job["library_id"])
    with Reader(lib.root, lib.cache) as reader:
        version = reader.version
    move = relocation.prepare(service.state, job["library_id"])
    assert move["phase"] == "prepared"
    media, index = tmp_path / "moved-media", tmp_path / "moved-online"
    shutil.copytree(lib.root, media)
    shutil.copytree(lib.cache, index)
    relocation.apply(service.state, move["id"], str(media), str(index))
    relocation.finish(service.state, move["id"])
    assert read_json(media / "online-index.json")["schema_version"] == 3
    with Reader(media, index, version=version) as reader:
        assert len(reader.objects()["items"]) == 1
    request["request_key"] = key()
    next_job = service.create_job(request)["job"]
    next_runner = Runner(service.state, resources=Resources(reserve_bytes=0), client_factory=Client, image_http=HTTP())
    assert next_runner.run(next_job["id"], time_slice=60)["state"] == "completed"


def test_changed_source_retains_diagnostic_hash_and_reclaims_bad_bytes(tmp_path):
    service, runner, job, _ = setup(tmp_path)
    image = io.BytesIO()
    Image.new("RGB", (13, 9), "red").save(image, format="PNG")
    runner.image_http = Session(image.getvalue(), headers={"Content-Length": str(len(image.getvalue()))})
    result = runner.run(job["id"], time_slice=60)
    assert result["state"] == "completed_with_gaps", result
    assert result["progress"]["media"]["gaps"] == 2
    assert result["progress"]["objects"]["stored"] == 0
    scratch = service.state.root / "collection-spool" / job["id"]
    assert not list(service.state.root.rglob("*.downloaded"))
    assert all(row["summary"]["download"]["bytes"] == len(image.getvalue()) for row in service.tasks(job["id"], dict(kind="media_download"))["items"])
    service.action(job["id"], dict(request_key=key(), expected_revision=result["revision"], action="cancel"))
    assert runner.run(job["id"])["state"] == "cancelled"
    assert not scratch.exists()
