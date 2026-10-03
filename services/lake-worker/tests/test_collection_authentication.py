"""Candidate login is checked before it can replace an existing credential."""
import json

import pytest
import requests

from test_collections import key, setup
from test_collection_media_accounts import cookie
from studio_lake.collectors.pixiv import normalize
from studio_lake.collectors.pixiv.http import Client as WebClient
from studio_lake.updates.sites import UpdateError


class Probe:
    def __init__(self, user="4242", before=None, rotated=None):
        self.user, self.before, self.rotated, self.calls = user, before, rotated, 0

    def probe(self):
        self.calls += 1
        if self.before:
            self.before()
        if self.user is None:
            raise UpdateError("COLLECTION_CREDENTIAL_REQUIRED", "Unauthenticated fixture")
        return normalize.visibility("probe-candidate", login="authenticated", user_id=self.user)

    def cookie_snapshot(self):
        return self.rotated or [cookie()]


def request():
    return dict(request_key=key(), expected_revision=None, account_id=key(), label="browser fixture",
                mode="session", cookies=[cookie()])


def test_authenticate_new_account_protects_secrets_and_replays_without_network(tmp_path):
    service, *_ = setup(tmp_path)
    args = request()
    rotated = {**cookie(), "value": "ROTATED_NATIVE_FIXTURE"}
    probe = Probe(rotated=[rotated])
    result = service.accounts.authenticate(args, client=probe)
    account = result["account"]
    assert account["state"] == "valid" and account["bound_user_id"] == "4242"
    assert account["revision"] == 1 and result["visibility"]["coverage_verified"] is False
    assert result["visibility"]["r18"] == "unknown"
    row, saved, context = service.accounts.session(account["id"])
    assert saved == [rotated]
    assert context["viewer_key"] == row["viewer_key"]
    assert service.accounts.authenticate(args, client=probe) == result
    assert probe.calls == 1
    with service.state.db() as db:
        ledger = db.execute("SELECT * FROM collection_requests WHERE request_key=?", (args["request_key"],)).fetchone()
        assert ledger["request_hash"].startswith("dpapi:")
        assert cookie()["value"] not in json.dumps(dict(ledger))
    assert rotated["value"].encode() not in row["secret_blob"]
    assert cookie()["value"] not in json.dumps(result)
    with pytest.raises(UpdateError) as conflict:
        service.accounts.authenticate({**args, "cookies": [rotated]}, client=probe)
    assert conflict.value.code == "COLLECTION_IDEMPOTENCY_CONFLICT"


def test_failed_or_wrong_account_login_preserves_verified_credentials(tmp_path):
    service, *_ = setup(tmp_path)
    original = request()
    account = service.accounts.authenticate(original, client=Probe())["account"]
    before = service.accounts.row(account["id"])
    candidate = {**original, "request_key": key(), "expected_revision": account["revision"],
                 "cookies": [{**cookie(), "value": "NEW_BROWSER_CANDIDATE"}]}
    for user, code in [(None, "COLLECTION_CREDENTIAL_REQUIRED"), ("9999", "COLLECTION_SCOPE_CHANGED")]:
        with pytest.raises(UpdateError) as rejected:
            service.accounts.authenticate({**candidate, "request_key": key()}, client=Probe(user))
        assert rejected.value.code == code
        assert service.accounts.row(account["id"]) == before
        assert service.accounts.session(account["id"])[1] == [cookie()]
    brand_new = request()
    with pytest.raises(UpdateError):
        service.accounts.authenticate(brand_new, client=Probe(None))
    with pytest.raises(UpdateError) as absent:
        service.accounts.row(brand_new["account_id"])
    assert absent.value.code == "NOT_FOUND"


def test_authentication_cas_rechecks_after_probe_and_cannot_undo_clear(tmp_path):
    service, *_ = setup(tmp_path)
    args = request()
    first = service.accounts.authenticate(args, client=Probe())["account"]
    def clear():
        service.accounts.clear(first["id"], dict(request_key=key(), expected_revision=first["revision"]))
    candidate = {**args, "request_key": key(), "expected_revision": first["revision"]}
    with pytest.raises(UpdateError) as conflict:
        service.accounts.authenticate(candidate, client=Probe(before=clear))
    assert conflict.value.code == "REVISION_CONFLICT"
    state = service.accounts.public(first["id"])
    assert state["state"] == "cleared" and not state["credential_set"]
    stale = Probe()
    with pytest.raises(UpdateError):
        service.accounts.authenticate({**candidate, "request_key": key()}, client=stale)
    assert stale.calls == 0


def test_same_account_reauthentication_keeps_viewer_and_is_one_revision(tmp_path):
    service, *_ = setup(tmp_path)
    args = request()
    first = service.accounts.authenticate(args, client=Probe())["account"]
    viewer = service.accounts.row(first["id"])["viewer_key"]
    updated = service.accounts.authenticate({**args, "request_key": key(), "expected_revision": first["revision"]}, client=Probe())
    assert updated["account"]["revision"] == first["revision"] + 1
    assert service.accounts.row(first["id"])["viewer_key"] == viewer


def test_native_canonical_cookie_domain_reaches_only_pixiv_https():
    client = WebClient(None, dict(viewer_key="native-fixture", mode="session"), [{**cookie(), "domain": "pixiv.net"}])
    try:
        def header(url):
            return client.session.prepare_request(requests.Request("GET", url)).headers.get("Cookie", "")
        assert "PHPSESSID=" in header("https://www.pixiv.net/")
        assert "PHPSESSID=" in header("https://accounts.pixiv.net/")
        for url in ("http://www.pixiv.net/", "https://www.pixiv.net.evil.invalid/", "https://i.pximg.net/image.jpg"):
            assert not header(url)
    finally:
        client.close()
