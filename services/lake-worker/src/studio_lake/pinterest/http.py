"""Bounded anonymous Pinterest resources; cookies never enter archived evidence."""

from contextlib import nullcontext
from dataclasses import asdict, dataclass
import secrets
import time
import uuid

import requests

from . import model
from ..canonical import canonical, utc
from ..updates import rate
from ..updates.sites import UpdateError
from ..util import retry_after_seconds

MAX_RESPONSE = 8 * 1024**2
CLIENT_VERSION = "pinterest-web-20261009"


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
        self.context = {**context, "session_instance": str(uuid.uuid4())}
        self.cancelled, self.injected = cancelled, session is not None
        self.session = session or requests.Session()
        if session is None:
            self.session.trust_env = False
            self.session.cookies.set("csrftoken", secrets.token_hex(32), domain="www.pinterest.com", path="/", secure=True)
            self.session.headers.update({"User-Agent": "Mozilla/5.0 Dataset-Studio/0.2",
                "Accept": "application/json, text/javascript, */*; q=0.01", "Accept-Language": context["language"],
                "X-Requested-With": "XMLHttpRequest", "X-Pinterest-AppState": "active",
                "X-Pinterest-PWS-Handler": "www/pin/[id].js", "Referer": "https://www.pinterest.com/"})

    def pin(self, identity):
        identity = model.pin_id(identity)
        options = dict(id=identity, field_set_key="detailed")
        source = "/pin/" + identity + "/"
        endpoint = "/resource/PinResource/get/"
        parameters = dict(options=options, source_url=source, context={})
        lane = nullcontext() if self.injected or self.rate_root is None else rate.admission(
            self.rate_root, self.name, self.cancelled, delay=1 / model.SITE_LIMITS["api_requests_per_second"])
        cooldown = 0
        try:
            with lane:
                if self.cancelled():
                    raise UpdateError("CANCELLED", "Pinterest collection paused")
                with self.session.get("https://www.pinterest.com" + endpoint,
                        params=dict(source_url=source, data=canonical(dict(options=options, context={}))),
                        headers={"X-Pinterest-Source-Url": source}, timeout=(10, 35), stream=True, allow_redirects=False) as response:
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
                    return Response(bytes(body), endpoint, parameters, response.status_code, utc(), evidence, cooldown)
        except requests.RequestException:
            raise UpdateError("PINTEREST_NETWORK", "Pinterest connection failed; saved work is retained", retry_after=30) from None
        finally:
            if cooldown and self.rate_root is not None and not self.injected:
                rate.cooldown(self.rate_root, self.name, cooldown)

    def close(self):
        self.session.close()


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
