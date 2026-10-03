"""Real parser and account acceptance for Pixiv's server-rendered Next.js state."""
import html
import json

import pytest

from test_collection_authentication import request
from test_collection_media_accounts import Session, cookie
from test_collections import setup
from studio_lake.collectors.pixiv.http import Client
from studio_lake.updates.sites import UpdateError


def next_page(page):
    payload = json.dumps({"props": {"pageProps": page}}, ensure_ascii=False)
    return f'<script id="__NEXT_DATA__" type="application/json">{payload}</script>'.encode()


def logged_in_page(user_id="4242"):
    return dict(isLoggedIn=True, serverSerializedPreloadedState=json.dumps(dict(
        userData=dict(self=dict(id=user_id, name="synthetic &quot; profile"),
                      users={"9999": dict(id="9999")}), api=dict(token="SYNTHETIC_CSRF"))))


def test_next_page_authentication_and_legacy_format_remain_supported():
    legacy = html.escape(json.dumps(dict(userData=dict(id="4242"))), quote=True)
    for body in [next_page(logged_in_page()), next_page(logged_in_page(4242)),
                 f'<meta id="meta-global-data" content="{legacy}">'.encode()]:
        client = Client(None, dict(viewer_key="parser-fixture"), [cookie()], session=Session(body))
        try:
            context = client.probe()
            policy = json.loads(context["policy_json"])
            assert policy["login"] == "authenticated" and policy["user_id"] == "4242"
            assert not context["verified"] and policy["r18"] == policy["r18g"] == policy["ai_display"] == "unknown"
            assert len(client.session.calls) == 1
        finally:
            client.close()


@pytest.mark.parametrize("page", [
    None, [], {},
    {**logged_in_page(), "isLoggedIn": False},
    {**logged_in_page(), "isLoggedIn": "true"},
    {**logged_in_page(), "isLoggedIn": 1},
    {"isLoggedIn": True, "gaUserData": {"login": True, "userId": "4242"}},
    *[dict(isLoggedIn=True, serverSerializedPreloadedState=value)
      for value in [None, {}, "{", "null", "[]", "{}",
                    json.dumps(dict(userData=None)),
                    json.dumps(dict(userData=dict(self=None, users={"4242": dict(id="4242")}))),
                    json.dumps(dict(userData=dict(self="4242")))]],
    *[logged_in_page(value) for value in [None, "", "0", 0, -1, True, "abc", "4" * 21]],
])
def test_next_page_rejects_missing_or_unconfirmed_viewer(page):
    client = Client(None, dict(viewer_key="parser-fixture"), [cookie()], session=Session(next_page(page)))
    try:
        with pytest.raises(UpdateError) as rejected:
            client.probe()
        assert rejected.value.code == "COLLECTION_CREDENTIAL_REQUIRED"
    finally:
        client.close()


@pytest.mark.parametrize("body", [
    b'<script id="__NEXT_DATA__">{}</script>',
    b'<script id="__NEXT_DATA__">{invalid}</script>',
    b'<script id="__NEXT_DATA__">[]</script>',
    next_page(logged_in_page()).removesuffix(b"</script>"),
    next_page(logged_in_page()) + next_page(logged_in_page("9999")),
    b'<meta id="meta-global-data" content="[]">',
    b'<html>Login or challenge required</html>',
])
def test_unrecognizable_or_ambiguous_page_never_confirms_login(body):
    client = Client(None, dict(viewer_key="parser-fixture"), session=Session(body))
    try:
        with pytest.raises(UpdateError) as rejected:
            client.probe()
        assert rejected.value.code == "COLLECTION_CREDENTIAL_REQUIRED"
    finally:
        client.close()


def test_candidate_can_retry_and_save_real_parser_result(tmp_path):
    service, *_ = setup(tmp_path)
    args = request()
    session = Session(next_page(dict(isLoggedIn=False, serverSerializedPreloadedState=json.dumps(
        dict(userData=dict(self=None, users={"9999": dict(id="9999")}))))))
    client = Client(None, dict(viewer_key="candidate-fixture"), args["cookies"], session=session)
    try:
        with pytest.raises(UpdateError) as rejected:
            service.accounts.authenticate(args, client=client)
        assert rejected.value.code == "COLLECTION_CREDENTIAL_REQUIRED"
        with pytest.raises(UpdateError) as missing:
            service.accounts.row(args["account_id"])
        assert missing.value.code == "NOT_FOUND"
        session.body = next_page(logged_in_page())
        result = service.accounts.authenticate(args, client=client)
        assert result["account"]["state"] == "valid" and result["account"]["bound_user_id"] == "4242"
        assert service.accounts.authenticate(args, client=client) == result
        assert len(session.calls) == 2
        assert cookie()["value"] not in json.dumps(result)
    finally:
        client.close()
