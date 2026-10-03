"""Bounded Web-AJAX sessions. A public session is explicit and never an auth fallback."""

from contextlib import nullcontext
from dataclasses import dataclass
from html.parser import HTMLParser
import json
import threading
import time

import requests

from . import normalize
from ...updates import rate
from ...updates.sites import UpdateError
from ...util import IntegrityError, retry_after_seconds

MAX_RESPONSE = 32 * 1024**2


@dataclass(frozen=True)
class Response:
    body: bytes
    endpoint: str
    parameters: dict
    status: int
    observed_at: str
    content_encoding: str


class LoginPage(HTMLParser):
    """Read the server's viewer identity from legacy and current Pixiv pages."""

    def __init__(self):
        super().__init__()
        self.data = None
        self.next_data = None
        self.next_parts = None
        self.next_seen = False

    def handle_starttag(self, tag, attrs):
        values = dict(attrs)
        if tag == "meta" and values.get("id") == "meta-global-data":
            self.data = json.loads(values.get("content", "{}"))
        if tag == "script" and values.get("id") == "__NEXT_DATA__":
            if self.next_seen:
                raise ValueError("Duplicate Pixiv page state")
            self.next_seen = True
            self.next_parts = []

    def handle_data(self, data):
        if self.next_parts is not None:
            self.next_parts.append(data)

    def handle_endtag(self, tag):
        if tag == "script" and self.next_parts is not None:
            self.next_data = json.loads("".join(self.next_parts))
            self.next_parts = None

    def user_id(self):
        if self.next_seen:
            if not isinstance(self.next_data, dict):
                return None
            props = self.next_data.get("props")
            page = props.get("pageProps") if isinstance(props, dict) else None
            if not isinstance(page, dict) or page.get("isLoggedIn") is not True:
                return None
            serialized = page.get("serverSerializedPreloadedState")
            if not isinstance(serialized, str):
                return None
            state = json.loads(serialized)
            users = state.get("userData") if isinstance(state, dict) else None
            # Other entries under userData.users are public profiles; only self
            # identifies this session. Never infer identity from the cookie value.
            user = users.get("self") if isinstance(users, dict) else None
        else:
            user = self.data.get("userData") if isinstance(self.data, dict) else None
        return normalize.source_id(user["id"]) if isinstance(user, dict) and user.get("id") else None


class Client:
    def __init__(self, root, account, cookies=(), *, cancelled=lambda: False, api_rate=1.0, session=None):
        self.rate_root, self.name = root, "pixiv"
        self.account, self.cancelled, self.api_rate = account, cancelled, api_rate
        self.injected = session is not None
        self.session = session or requests.Session()
        self.session.headers.update({"User-Agent": "Mozilla/5.0 Dataset-Studio/0.2", "Referer": "https://www.pixiv.net/", "Accept": "application/json"})
        # Explicit session cookies are scoped by Requests' cookie policy. Media uses another jar.
        for cookie in cookies:
            self.session.cookies.set(cookie["name"], cookie["value"], domain=cookie["domain"], path=cookie["path"],
                                     secure=True, expires=cookie["expires_unix"], rest={"HttpOnly": cookie["http_only"]})

    def close(self):
        self.session.close()

    def cookie_snapshot(self):
        values = []
        for cookie in self.session.cookies:
            domain = cookie.domain
            if domain == ".www.pixiv.net":
                domain = "www.pixiv.net"
            if domain not in {".pixiv.net", "pixiv.net", "www.pixiv.net", "accounts.pixiv.net"} or cookie.is_expired():
                continue
            values.append(dict(name=cookie.name, value=cookie.value, domain=domain, path=cookie.path or "/",
                               secure=True, http_only=cookie.has_nonstandard_attr("HttpOnly"), expires_unix=cookie.expires))
        return values

    def _request(self, endpoint, parameters=None, *, html=False):
        if self.cancelled():
            raise UpdateError("CANCELLED", "Collection paused")
        if not endpoint.startswith("/") or endpoint.startswith("//") or "?" in endpoint or "#" in endpoint:
            raise UpdateError("INVALID_INPUT", "Invalid Pixiv endpoint")
        parameters = parameters or {}
        cooldown = None
        lane = nullcontext() if self.injected or self.rate_root is None else rate.admission(self.rate_root, "pixiv", self.cancelled, delay=1 / self.api_rate)
        try:
            with lane:
                with self.session.get("https://www.pixiv.net" + endpoint, params=parameters, timeout=(10, 35), stream=True, allow_redirects=False) as response:
                    status = response.status_code
                    if status in {429, 503}:
                        cooldown = retry_after_seconds(response.headers) or 60
                        raise UpdateError("COLLECTION_REMOTE_UNAVAILABLE", "Pixiv request is cooling down", retry_after=cooldown)
                    if status in {401, 403} or 300 <= status < 400:
                        raise UpdateError("COLLECTION_CREDENTIAL_REQUIRED", "Pixiv requires login or interactive verification")
                    if status == 404:
                        raise UpdateError("COLLECTION_NOT_ACCESSIBLE", "Pixiv did not make this target accessible")
                    if status >= 500:
                        raise UpdateError("COLLECTION_REMOTE_UNAVAILABLE", "Pixiv is temporarily unavailable", retry_after=60)
                    if status != 200:
                        raise UpdateError("COLLECTION_RESPONSE_INVALID", "Unexpected Pixiv response status")
                    length = response.headers.get("Content-Length", "")
                    if length.isdigit() and int(length) > MAX_RESPONSE:
                        raise UpdateError("COLLECTION_LIMIT", "Pixiv response exceeds its byte budget")
                    body = bytearray()
                    started = time.monotonic()
                    for chunk in response.iter_content(64 * 1024):
                        if self.cancelled():
                            raise UpdateError("CANCELLED", "Collection paused")
                        if len(body) + len(chunk) > MAX_RESPONSE or time.monotonic() - started > 120:
                            raise UpdateError("COLLECTION_LIMIT", "Pixiv response exceeds its transfer budget")
                        body.extend(chunk)
                    result = Response(bytes(body), endpoint, parameters, status, normalize.utc(), response.headers.get("Content-Encoding", "identity"))
        except requests.RequestException:
            raise UpdateError("COLLECTION_REMOTE_UNAVAILABLE", "Pixiv connection failed", retry_after=30) from None
        finally:
            if cooldown is not None and self.rate_root is not None and not self.injected:
                # Release the API lane before persisting its shared cooldown.
                rate.cooldown(self.rate_root, "pixiv", cooldown)
        if not html:
            try:
                payload = json.loads(result.body)
            except (ValueError, UnicodeError):
                raise UpdateError("COLLECTION_CREDENTIAL_REQUIRED", "Pixiv returned an interactive page instead of JSON") from None
            if not isinstance(payload, dict) or not isinstance(payload.get("error"), bool) or "body" not in payload:
                error = UpdateError("COLLECTION_RESPONSE_INVALID", "Pixiv response envelope changed")
                error.response = result
                raise error
            if payload["error"]:
                error = UpdateError("COLLECTION_NOT_ACCESSIBLE", "Pixiv returned an unavailable or restricted target")
                error.response = result
                raise error
        return result

    def request(self, kind, payload):
        if kind in {"author_profile", "author_directory"}:
            identity = normalize.source_id(payload["author_id"])
            endpoint = f"/ajax/user/{identity}" if kind == "author_profile" else f"/ajax/user/{identity}/profile/all"
            return self._request(endpoint, {"full": "1"} if kind == "author_profile" else {})
        if kind == "work_detail":
            return self._request("/ajax/illust/" + normalize.source_id(payload["work_id"]))
        if kind == "media_manifest":
            detail = payload["detail"]
            suffix = "ugoira_meta" if detail["work_type"] == "ugoira" else "pages"
            return self._request(f"/ajax/illust/{normalize.source_id(detail['work_id'])}/{suffix}")
        if kind == "relationship_page":
            root = normalize.source_id(payload["root_id"])
            relation, offset = payload["relation"], payload["cursor"]
            if relation == "following":
                return self._request(f"/ajax/user/{root}/following", dict(offset=offset, limit=100, rest="show"))
            if relation == "bookmarks":
                return self._request(f"/ajax/user/{root}/illusts/bookmarks", dict(offset=offset, limit=100, rest="show", tag=""))
            if relation == "recommendations" and payload["root_kind"] == "work":
                return self._request(f"/ajax/illust/{root}/recommend/init", {"limit": 18})
        raise UpdateError("INVALID_INPUT", "Unknown Pixiv task")

    def probe(self):
        response = self._request("/", html=True)
        parser = LoginPage()
        try:
            parser.feed(response.body.decode("utf-8"))
            parser.close()
            user_id = parser.user_id()
        except (ValueError, TypeError, KeyError, UnicodeError, IntegrityError, RecursionError):
            user_id = None
        if user_id is None:
            raise UpdateError("COLLECTION_CREDENTIAL_REQUIRED", "Pixiv did not confirm an authenticated user")
        # Only the user ID is proven by the server-rendered global data. Display
        # controls remain unknown until a dedicated positive-control probe exists.
        return normalize.visibility(self.account["viewer_key"], login="authenticated", user_id=user_id, observed_at=response.observed_at)


class MediaSessions:
    """Thread-local, cookie-free image connections, compatible with the shared range transport."""
    def __init__(self, injected=None):
        self.injected, self.local = injected, threading.local()
        self.lock, self.sessions = threading.Lock(), []

    def get(self):
        if self.injected is not None:
            return self.injected
        if not hasattr(self.local, "session"):
            session = requests.Session()
            session.headers.update({"User-Agent": "Mozilla/5.0 Dataset-Studio/0.2", "Referer": "https://www.pixiv.net/"})
            self.local.session = session
            with self.lock:
                self.sessions.append(session)
        return self.local.session

    def close(self):
        for session in self.sessions:
            session.close()
