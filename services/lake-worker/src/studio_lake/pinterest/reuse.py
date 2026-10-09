"""Source-compatible historical acquisitions; reuse always keeps its own explicit evidence."""

from datetime import datetime, timezone
import hashlib
import json

import requests

from ..canonical import canonical, utc
from ..online_storage import connect
from ..updates import rate
from ..updates.sites import UpdateError
from ..updates.transfer import strong_etag
from ..util import IntegrityError, contained, read_json, stable_id


def key(lib, entry):
    pointer = read_json(lib.cache / "ONLINE.json")
    db = connect(contained(lib.cache, pointer["file"]))
    try:
        row = db.execute("SELECT m.content_revision,c.policy_json FROM media_manifests m JOIN visibility_contexts c USING(context_id) WHERE m.manifest_id=? AND m.context_id=? AND m.complete=1",
                         (entry["manifest_id"], entry["context_id"])).fetchone()
    finally:
        db.close()
    if not row:
        raise IntegrityError("Pinterest reuse requires an accepted complete manifest")
    context = json.loads(row[1])
    scope = {k: context.get(k) for k in ("mode", "language", "response_language", "response_country", "client_version", "source_country", "account_id", "credential_revision")}
    return stable_id("pinterest-reuse-v1", entry["normalized_url"], entry["role"], row[0], "original", scope)


def object_available(lib, saved, resources, cancelled):
    obj = lib.lookup_object(saved["download_sha256"])
    if not obj or obj["length"] != saved["download_bytes"] or obj["length"] > resources.max_download_bytes:
        return False
    sha, remaining = hashlib.sha256(), obj["length"]
    try:
        with contained(lib.root, obj["pack_path"]).open("rb") as source:
            source.seek(obj["offset"])
            while remaining:
                if cancelled():
                    raise UpdateError("CANCELLED", "Pinterest reuse paused")
                chunk = source.read(min(256 * 1024, remaining))
                if not chunk:
                    raise UpdateError("PINTEREST_STORED_OBJECT_CHANGED", "A previously archived Pinterest object is truncated")
                remaining -= len(chunk)
                sha.update(chunk)
    except OSError:
        raise UpdateError("PINTEREST_STORED_OBJECT_CHANGED", "A previously archived Pinterest object is unavailable") from None
    if sha.hexdigest() != saved["download_sha256"]:
        raise UpdateError("PINTEREST_STORED_OBJECT_CHANGED", "A previously archived Pinterest object failed its byte check")
    return True


def previous(state, lib, job, task, entry, reuse_key, download_key, sessions, resources, cancelled):
    policy = json.loads(job["definition_json"])["media"]["reuse"]
    if not policy["max_age_hours"] or task["download_generation"]:
        return None, None
    with state.db() as db:
        row = db.execute("SELECT value_json FROM pinterest_reuse WHERE lake_id=? AND reuse_key=?", (job["lake_id"], reuse_key)).fetchone()
    if not row:
        return None, None
    old = json.loads(row[0])
    checked = old.get("source_checked_at", old["acquired_at"])
    age = (datetime.now(timezone.utc) - datetime.fromisoformat(checked.replace("Z", "+00:00"))).total_seconds()
    if (not 0 <= age <= policy["max_age_hours"] * 3600 or (old["width"], old["height"]) != (entry["width"], entry["height"])
            or not object_available(lib, old, resources, cancelled)):
        return None, None
    evidence, response = "historical_reuse", None
    if policy["mode"] == "revalidate":
        etag = strong_etag(old.get("cdn_etag"))
        if not etag:
            return None, None
        if sessions.injected is None:
            rate.wait_start(state.root, "pinterest", cancelled, lambda: resources.config["sites"]["pinterest"]["image_requests_per_second"])
        try:
            response = sessions.get().get(entry["normalized_url"], headers={"If-None-Match": etag, "Accept-Encoding": "identity"},
                timeout=(10, 45), stream=True, allow_redirects=False)
        except requests.RequestException:
            raise UpdateError("PINTEREST_NETWORK", "Pinterest CDN validation failed", retry_after=30) from None
        if response.status_code != 304:
            # The ordinary validated transfer consumes this very response, including a changed 200 or an error.
            return None, response
        with response:
            returned = response.headers.get("ETag")
            if returned is not None and returned != etag:
                return dict(state="needs_review", reason="cdn_etag_changed_on_304"), None
        checked, evidence = utc(), "http_validated"
    value = {**old, "acquisition_id": stable_id("pinterest-acquisition-v1", job["id"], download_key),
        "context_id": entry["context_id"], "source_url": entry["source_url"], "normalized_url": entry["normalized_url"],
        "acquired_at": utc(), "evidence": evidence, "source_checked_at": checked,
        "details_json": canonical(dict(previous_acquisition_id=old["acquisition_id"], previous_context_id=old["context_id"],
            previous_acquired_at=old["acquired_at"], source_checked_at=checked, transfer_bytes=0)),
        "download_key": download_key, "reuse_key": reuse_key, "archive_reuse": True}
    return value, None


class Prefetched:
    def __init__(self, session, response):
        self.session, self.response = session, response

    def get(self, *args, **kwargs):
        if self.response is not None:
            value, self.response = self.response, None
            return value
        return self.session.get(*args, **kwargs)

    def close(self):
        if self.response is not None:
            self.response.__exit__(None, None, None)
        self.session.close()
