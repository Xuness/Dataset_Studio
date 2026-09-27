"""Bounded public API requests; authentication never enters request evidence."""

from dataclasses import dataclass
from decimal import Decimal
import json
import threading
import time

import requests

from ..api_metadata import SITE_KINDS, normalize_api
from ..util import split_json_posts, retry_after_seconds


class UpdateError(Exception):
    def __init__(self, code, message, retry_after=0):
        super().__init__(message)
        self.code, self.retry_after = code, retry_after


@dataclass(frozen=True)
class Response:
    body: bytes
    status: int
    request: dict
    retry_after: float = 0


def split_response(site, body):
    if site != "gelbooru":
        return split_json_posts(body)
    text = body.decode("utf-8-sig")
    decoder = json.JSONDecoder(parse_float=Decimal)
    data = decoder.decode(text)
    if not isinstance(data, dict) or "@attributes" not in data:
        raise ValueError("Gelbooru response envelope is invalid")
    if "post" not in data:
        if int(data["@attributes"].get("count", -1)) == 0:
            return []
        raise ValueError("Gelbooru response omitted a nonempty post list")
    # Walk top-level JSON tokens so escaped field names/numbers/unknown fields retain exact spelling.
    pos = text.index("{") + 1
    while True:
        while text[pos].isspace() or text[pos] == ",":
            pos += 1
        key, pos = decoder.raw_decode(text, pos)
        while text[pos].isspace():
            pos += 1
        if text[pos] != ":":
            raise ValueError("invalid envelope")
        pos += 1
        while text[pos].isspace():
            pos += 1
        start = pos
        _, pos = decoder.raw_decode(text, pos)
        if key == "post":
            return split_json_posts(text[start:pos].encode())


class Site:
    URLS = {
        "danbooru": "https://danbooru.donmai.us/posts.json",
        "yandere": "https://yande.re/post.json",
        "gelbooru": "https://gelbooru.com/index.php",
    }

    def __init__(self, name, credentials=None, http=None, delay=1.1, rate_root=None):
        if name not in self.URLS:
            raise UpdateError("INVALID_INPUT", "Unsupported update site")
        self.name, self.credentials = name, credentials or {}
        self.http = http or requests.Session()
        self.http.trust_env = False
        self.http.headers["User-Agent"] = "Dataset-Studio/0.1 (local dataset archiver)"
        self.delay, self.last = delay, 0.0
        self.gate = threading.Lock()
        self.rate_root = rate_root

    def capabilities(self):
        return {
            "site": self.name,
            "adapter_version": 1,
            "page_size": 100 if self.name == "gelbooru" else 200,
            "id_ranges": True,
            "id_lists": True,
            "created_range": "bounded_id_scan",
            "updated_range": True,
            "change_sequence": self.name == "yandere",
            "full_change_history": False,
            "deletion_discovery": "explicit_post_status",
            "credential_set": bool(self.credentials),
        }

    def params(self, lower, upper, limit, *, ids=None, change_after=None, change_through=None):
        if self.name == "gelbooru":
            p = {"page": "dapi", "s": "post", "q": "index", "json": 1, "limit": limit, "pid": 0}
            p["tags"] = f"id:>{lower - 1} id:<{upper} sort:id:asc"
            if ids is not None:
                if len(ids) != 1:
                    raise ValueError("Gelbooru explicit ID requests are single-ID")
                p["id"] = ids[0]
                p.pop("tags")
            if change_after is not None:
                p["cid"] = change_after
            return p
        order = "order:id" if self.name == "yandere" else "order:id_asc"
        token = ",".join(map(str, ids)) if ids is not None else f"{lower}..{upper - 1}"
        visibility = "deleted:all holds:all pending:all" if self.name == "yandere" else "status:any"
        tags = f"id:{token} {order} {visibility}"
        if change_after is not None:
            if self.name != "yandere":
                raise UpdateError("UPDATE_UNSUPPORTED", "This site has no verified change sequence")
            tags += (
                f" change:{change_after + 1}..{change_through}"
                if change_through is not None
                else f" change:>{change_after}"
            )
        return {"limit": limit, "tags": tags}

    def request(self, parameters, cancelled=lambda: False, resource="posts"):
        if self.rate_root is not None:
            from .rate import admission

            with admission(self.rate_root, self.name, cancelled, self.delay):
                return self._request(parameters, cancelled, resource)
        return self._request(parameters, cancelled, resource)

    def _request(self, parameters, cancelled, resource):
        url = self.URLS[self.name]
        if resource == "tag_summary" and self.name == "yandere":
            url = "https://yande.re/tag/summary.json"
        elif resource == "tags" and self.name == "yandere":
            url = "https://yande.re/tag.json"
        elif resource not in {"posts", "tags"}:
            raise UpdateError("INVALID_INPUT", "Unknown API resource")
        with self.gate:
            while time.monotonic() < self.last + self.delay:
                if cancelled():
                    raise UpdateError("CANCELLED", "Update paused")
                time.sleep(min(0.1, max(0, self.last + self.delay - time.monotonic())))
            self.last = time.monotonic()
        params, auth = dict(parameters), None
        credentials = dict(self.credentials)
        if self.name == "gelbooru":
            if not credentials.get("api_key") or not credentials.get("user_id"):
                raise UpdateError("UPDATE_CREDENTIAL_REQUIRED", "Gelbooru requires user_id and API key")
            params.update({k: credentials[k] for k in ("user_id", "api_key")})
        elif self.name == "danbooru" and credentials.get("api_key"):
            auth = (credentials.get("login", ""), credentials["api_key"])
        try:
            with self.http.get(url, params=params, auth=auth, timeout=(10, 45), stream=True) as r:
                parts, total = [], 0
                for part in r.iter_content(65536):
                    if cancelled():
                        raise UpdateError("CANCELLED", "Update paused")
                    total += len(part)
                    if total > 16 * 1024**2:
                        raise UpdateError("UPDATE_RESPONSE_LIMIT", "API response exceeds 16 MiB")
                    parts.append(part)
                body = b"".join(parts)
                # Authentication errors may echo query parameters. Never archive credentials.
                for key in ("api_key", "password"):
                    secret = credentials.get(key)
                    if secret:
                        body = body.replace(str(secret).encode(), b"[credential-redacted]")
                return Response(
                    body,
                    r.status_code,
                    {"endpoint": url, "parameters": parameters},
                    retry_after_seconds(r.headers) or 0,
                )
        except requests.RequestException:
            raise UpdateError("UPDATE_NETWORK", "API transport failed; credentials and URL omitted") from None

    def parse(self, response):
        if response.status != 200:
            code = "UPDATE_CREDENTIAL_REQUIRED" if response.status in {401, 403} else "UPDATE_REMOTE_ERROR"
            raise UpdateError(code, f"API HTTP {response.status}", response.retry_after)
        try:
            rows = split_response(self.name, response.body)
        except (ValueError, TypeError, KeyError, IndexError):
            raise UpdateError(
                "UPDATE_RESPONSE_INVALID", "API response schema is invalid; raw retained"
            ) from None
        ids = [r.get("id") for _, r in rows]
        if any(not isinstance(i, int) or isinstance(i, bool) or i <= 0 for i in ids) or len(set(ids)) != len(
            ids
        ):
            raise UpdateError("UPDATE_RESPONSE_INVALID", "API returned invalid or duplicate post IDs")
        tag_field = "tag_string" if self.name == "danbooru" else "tags"
        if any(not isinstance(r.get(tag_field), str) for _, r in rows):
            raise UpdateError(
                "UPDATE_RESPONSE_INVALID", "Post tag field is missing or has changed type; raw retained"
            )
        return rows

    def normalize(self, row, key, ordinal, observed, tag_types=None):
        return normalize_api(row, key, ordinal, SITE_KINDS[self.name], ordinal, observed, tag_types=tag_types)

    def close(self):
        self.http.close()
