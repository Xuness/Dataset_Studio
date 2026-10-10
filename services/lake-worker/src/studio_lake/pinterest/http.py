"""Bounded anonymous Pinterest resources; cookies never enter archived evidence."""

from contextlib import nullcontext
from dataclasses import asdict, dataclass
import hashlib
import json
import time
from urllib.parse import quote, urlsplit

import requests

from . import model
from ..canonical import canonical, utc
from ..updates import rate
from ..updates.sites import UpdateError
from ..util import retry_after_seconds

MAX_RESPONSE = 8 * 1024**2
CLIENT_VERSION = "pinterest-web-20261010"


@dataclass(frozen=True)
class Response:
    body: bytes
    endpoint: str
    parameters: dict
    status: int
    observed_at: str
    context: dict
    retry_after: float = 0

    def metadata(self):
        return {k: v for k, v in asdict(self).items() if k != "body"}


class Client:
    def __init__(self, root, context, *, cancelled=lambda: False, session=None):
        self.rate_root, self.name = root, "pinterest"
        # An anonymous job keeps one explicit session identity across execution slices/restarts.
        self.context = {**context, "session_instance": context["session_id"], "anonymous_cookie_policy": "fixed-csrf-only"}
        self.cancelled, self.injected = cancelled, session is not None
        self.session = session or requests.Session()
        if session is None:
            self.session.trust_env = False
            token = hashlib.sha256(context["session_id"].encode()).hexdigest()
            self.csrf_token = token
            self.session.cookies.set("csrftoken", token, domain="www.pinterest.com", path="/", secure=True)
            self.session.headers.update({"User-Agent": "Mozilla/5.0 Dataset-Studio/0.2",
                "Accept": "application/json, text/javascript, */*; q=0.01", "Accept-Language": context["language"],
                "X-Requested-With": "XMLHttpRequest", "X-Pinterest-AppState": "active",
                "X-Pinterest-PWS-Handler": "www/pin/[id].js", "Accept-Encoding": "gzip, deflate",
                "Referer": "https://www.pinterest.com/"})

    def pin(self, identity, *, expanded=False):
        identity = model.pin_id(identity)
        options = dict(id=identity, field_set_key="detailed")
        if expanded:
            options.update(field_set_key="auth_web_main_pin", add_fields="pin.gen_ai_topics", fetch_visual_search_objects=True)
        source = "/pin/" + identity + "/"
        endpoint = "/resource/PinResource/get/"
        return self.resource(endpoint, options, source)

    def request(self, kind, entry):
        endpoint, options, source = request_parameters(kind, entry)
        return self.resource(endpoint, options, source)

    def resource(self, endpoint, options, source):
        parameters = dict(options=options, source_url=source, context={})
        query = None
        if endpoint.startswith("/resource/"):
            # A fixed resource URL can keep returning a premature empty result.
            # Retain the exact freshness parameter with the response evidence.
            parameters["_"] = str(time.time_ns())
            query = dict(source_url=source, data=canonical(dict(options=options, context={})), _=parameters["_"])
        lane = nullcontext() if self.injected or self.rate_root is None else rate.admission(
            self.rate_root, self.name, self.cancelled, delay=1 / model.SITE_LIMITS["api_requests_per_second"])
        cooldown = 0
        try:
            with lane:
                if self.cancelled():
                    raise UpdateError("CANCELLED", "Pinterest collection paused")
                if not self.injected:
                    # Do not silently acquire a different anonymous cookie scope between pages or slices.
                    self.session.cookies.clear()
                    self.session.cookies.set("csrftoken", self.csrf_token, domain="www.pinterest.com", path="/", secure=True)
                with self.session.get("https://www.pinterest.com" + endpoint,
                        params=query,
                        headers={"X-Pinterest-Source-Url": source, "Accept": "application/json" if endpoint.startswith("/resource/") else "text/html"},
                        timeout=(10, 35), stream=True, allow_redirects=False) as response:
                    body, started = bytearray(), time.monotonic()
                    length = response.headers.get("Content-Length", "")
                    if str(length).isdigit() and int(length) > MAX_RESPONSE:
                        raise UpdateError("PINTEREST_RESPONSE_LIMIT", "Pinterest response exceeds its byte budget")
                    for chunk in response.iter_content(64 * 1024):
                        if self.cancelled():
                            raise UpdateError("CANCELLED", "Pinterest collection paused")
                        if len(body) + len(chunk) > MAX_RESPONSE or time.monotonic() - started > 120:
                            raise UpdateError("PINTEREST_RESPONSE_LIMIT", "Pinterest response exceeds its transfer budget")
                        body.extend(chunk)
                    if response.status_code in (429, 503):
                        cooldown = min(retry_after_seconds(response.headers) or 60, 86400)
                    # Headers are allowlisted; never persist Set-Cookie, request cookies or credentials.
                    evidence = {**self.context, "client_version": CLIENT_VERSION,
                                "response_language": response.headers.get("Content-Language"),
                                "response_country": response.headers.get("X-Pinterest-Country")}
                    try:
                        payload = json.loads(body)
                        context = payload.get("client_context") if isinstance(payload, dict) else None
                        if isinstance(context, dict):
                            for key in ("country_from_ip", "language", "locale", "app_version"):
                                if isinstance(context.get(key), str) and len(context[key]) <= 128:
                                    evidence["source_" + ("country" if key == "country_from_ip" else key)] = context[key]
                    except (ValueError, UnicodeError):
                        pass
                    return Response(bytes(body), endpoint, parameters, response.status_code, utc(), evidence, cooldown)
        except requests.RequestException:
            raise UpdateError("PINTEREST_NETWORK", "Pinterest connection failed; saved work is retained", retry_after=30) from None
        finally:
            if cooldown and self.rate_root is not None and not self.injected:
                rate.cooldown(self.rate_root, self.name, cooldown)

    def close(self):
        self.session.close()


def request_parameters(kind, entry):
    """Only frozen source inputs select endpoints; never accept a caller-controlled host."""
    subject = entry["subject_id"]
    source = "/"
    if kind in ("board_resolve", "section_resolve"):
        parts = urlsplit(subject).path.strip("/").split("/")
        if kind == "board_resolve":
            resource, options = "Board", dict(username=parts[0], slug=parts[1], field_set_key="detailed")
        else:
            resource, options = "BoardSection", dict(username=parts[0], board_slug=parts[1], section_slug=parts[2])
        source = "/" + "/".join(quote(p, safe="") for p in parts) + "/"
    elif kind == "board_page":
        resource, options = "BoardFeed", dict(board_id=subject, field_set_key="react_grid_pin", prepend=False, page_size=25)
    elif kind == "board_sections":
        resource, options = "BoardSections", dict(board_id=subject)
    elif kind == "section_page":
        resource, options = "BoardSectionPins", dict(section_id=subject)
    elif kind == "board_more_ideas":
        resource, options = "BoardContentRecommendation", dict(id=subject, type="board", add_vase=True)
    elif kind == "related_pins":
        resource, options = "RelatedPinFeed", dict(pin=subject, add_vase=True, pins_only=True)
        source = "/pin/" + subject + "/"
    elif kind == "search_page":
        resource, options = "BaseSearch", dict(query=subject, scope=entry["parameters"]["scope"], rs="typed")
        source = "/search/" + options["scope"] + "/?q=" + quote(subject, safe="")
    elif kind == "topic_page":
        path = urlsplit(subject).path
        if entry.get("cursor") is None:
            return path, dict(field_set_key="__PWS_INITIAL_PROPS__"), path
        resource = "BestPinsFeedAlt"
        options = dict(entry["parameters"]["topic_options"])
        source = path
    else:
        raise UpdateError("INVALID_INPUT", "Unsupported Pinterest resource kind")
    if kind in model.PAGE_KINDS:
        options["bookmarks"] = entry.get("cursor")
    return "/resource/" + resource + "Resource/get/", options, source


class MediaSessions:
    def __init__(self, injected=None):
        self.injected = injected
        self.session = injected or requests.Session()
        if injected is None:
            self.session.trust_env = False
            self.session.headers.update({"User-Agent": "Mozilla/5.0 Dataset-Studio/0.2", "Referer": "https://www.pinterest.com/"})

    def get(self):
        return self.session

    def close(self):
        self.session.close()
