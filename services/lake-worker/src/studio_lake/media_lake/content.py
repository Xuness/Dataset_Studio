"""Content compatibility excludes counters, captions and other descriptive edits.

The fingerprint records source evidence, not a claim about unseen remote bytes.
Old captures remain sufficient to derive it without rewriting archived facts.
"""

import json

from .schema import canonical, utc
from ..raw_codec import decode
from ..util import digest


def revision(data):
    updated = data.get("uploadDate")
    if updated:
        try:
            updated = utc(updated)
        except (TypeError, ValueError, AttributeError):
            return None
    urls = data.get("urls") or {}
    if not isinstance(urls, dict):
        return None
    return digest(canonical(dict(version=1, work_id=data.get("id"), work_type=data.get("illustType"),
                                 page_count=data.get("pageCount"), updated_at=updated,
                                 width=data.get("width"), height=data.get("height"), original=urls.get("original"))).encode())


def observation_revision(db, detail):
    saved = json.loads(detail["source_fields_json"]).get("pixiv", {}).get("content_revision")
    if isinstance(saved, str) and len(saved) == 64:
        return saved
    row = db.execute("SELECT raw_zlib,raw_bytes,raw_sha256 FROM captures WHERE capture_id=?", (detail["capture_id"],)).fetchone()
    if not row:
        return None
    # A legacy observation has the same bounded raw capture as its new equivalent.
    try:
        body = json.loads(decode(*row, maximum_bytes=64 * 1024**2))["body"]
        return revision(body) if isinstance(body, dict) else None
    except (KeyError, TypeError, ValueError):
        return None


def compatible(db, detail, manifest):
    if not detail or not manifest or not manifest["detail_observation_id"]:
        return False
    if detail["observation_id"] == manifest["detail_observation_id"]:
        return True
    from .records import one

    basis = one(db, "SELECT * FROM work_observations WHERE observation_id=? AND work_id=?",
                (manifest["detail_observation_id"], detail["work_id"]))
    if not basis:
        return False
    current, previous = observation_revision(db, detail), observation_revision(db, basis)
    return current is not None and current == previous
