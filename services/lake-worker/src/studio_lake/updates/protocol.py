"""Validated frozen job definitions shared by CLI and the Studio bridge."""

from datetime import datetime, timezone
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError
import json

from .sites import UpdateError, Site
from ..image_policy import media_policy, ImagePolicyError


def timestamp(text):
    try:
        dt = datetime.fromisoformat(text.replace("Z", "+00:00"))
        if dt.tzinfo is None:
            raise ValueError()
        return dt.astimezone(timezone.utc)
    except (ValueError, TypeError, AttributeError):
        raise UpdateError("INVALID_INPUT", "Timestamp requires an explicit timezone") from None


def number(value, minimum=1, maximum=2**63 - 1):
    if isinstance(value, bool) or not isinstance(value, int) or not minimum <= value <= maximum:
        raise UpdateError("INVALID_INPUT", "Integer is outside the allowed range")
    return value


def definition(value):
    if not isinstance(value, dict) or set(value) - {
        "library_id",
        "range",
        "media",
        "page_budget",
        "item_budget",
    }:
        raise UpdateError("INVALID_INPUT", "Unknown job definition field")
    value = json.loads(json.dumps(value))
    if not isinstance(value.get("library_id"), str) or not value["library_id"]:
        raise UpdateError("INVALID_INPUT", "Library identity required")
    r = value.get("range")
    if not isinstance(r, dict):
        raise UpdateError("INVALID_INPUT", "A bounded update range is required")
    kind = r.get("kind")
    for optional in ("start_id", "end_id", "observed_before", "missing_media"):
        if r.get(optional) is None:
            r.pop(optional, None)
    fields = {
        "input": {"kind", "input_id"},
        "ids": {"kind", "ids"},
        "id_range": {"kind", "start", "end"},
        "new": {"kind", "after_id"},
        "changes": {"kind", "after", "start_id", "end_id"},
        "created": {"kind", "start", "end", "timezone", "start_id", "end_id"},
        "updated": {"kind", "start", "end", "timezone", "start_id", "end_id"},
        "local": {"kind", "start_id", "end_id", "observed_before", "missing_media"},
    }
    if kind not in fields or set(r) - fields[kind]:
        raise UpdateError("INVALID_INPUT", "Unsupported range or unknown range field")
    if kind == "input":
        if not isinstance(r.get("input_id"), str) or len(r["input_id"]) != 32:
            raise UpdateError("INVALID_INPUT", "A sealed input ID is required")
    elif kind == "ids":
        ids = r.get("ids")
        if not isinstance(ids, list) or not 1 <= len(ids) <= 10000:
            raise UpdateError("INVALID_INPUT", "Supply 1–10000 IDs or use a paged range")
        r["ids"] = sorted({number(v) for v in ids})
    elif kind == "id_range":
        if number(r.get("start")) >= number(r.get("end")):
            raise UpdateError("INVALID_INPUT", "ID range must be [start,end)")
    elif kind == "new":
        if r.get("after_id") is not None:
            number(r["after_id"], 0)
    else:
        number(r.setdefault("start_id", 1))
        if r.get("end_id") is not None and number(r["end_id"]) <= r["start_id"]:
            raise UpdateError("INVALID_INPUT", "Invalid ID scan bounds")
        if kind in {"created", "updated"}:
            if timestamp(r.get("start")) >= timestamp(r.get("end")):
                raise UpdateError("INVALID_INPUT", "Date range must be [start,end)")
            try:
                ZoneInfo(r.get("timezone", "UTC"))
            except (ZoneInfoNotFoundError, TypeError, ValueError):
                raise UpdateError("INVALID_INPUT", "Unknown timezone") from None
        if kind == "changes":
            number(r.get("after"), 0)
        if kind == "local":
            if r.get("observed_before"):
                timestamp(r["observed_before"])
            if "missing_media" in r and not isinstance(r["missing_media"], bool):
                raise UpdateError("INVALID_INPUT", "missing_media must be boolean")
    try:
        value["media"] = media_policy(value.get("media", {"profile": "metadata_only"}))
    except ImagePolicyError as error:
        raise UpdateError("INVALID_INPUT", str(error)) from None
    number(value.setdefault("page_budget", 1000), 1, 100000)
    number(value.setdefault("item_budget", 100000), 1, 10000000)
    return value


def validate_site(site, spec):
    if site not in Site.URLS:
        raise UpdateError("INVALID_INPUT", "Unknown site")
    kind = spec["range"]["kind"]
    if kind == "changes" and site != "yandere":
        raise UpdateError("UPDATE_UNSUPPORTED", "Only Yandere has a verified numeric change sequence")
