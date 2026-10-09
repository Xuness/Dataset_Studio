"""Frozen Pinterest definitions. Unsupported capabilities fail before a job is created."""

import re
import uuid
from urllib.parse import urlsplit

from . import COLLECTOR
from ..canonical import canonical
from ..updates.sites import UpdateError

TERMINAL = frozenset(("completed", "completed_with_gaps", "cancelled"))
SITE_LIMITS = dict(download_concurrency=2, api_requests_per_second=0.5, image_requests_per_second=1.0)
DEFAULT_BUDGET = dict(api_requests=100, admitted_pins=100, admitted_boards=0, detail_requests=100,
                      download_bytes=1024**3, wall_seconds=3600)


def invalid(message):
    raise UpdateError("INVALID_INPUT", message)


def fields(value, required=(), optional=()):
    if not isinstance(value, dict) or set(required) - value.keys() or value.keys() - set(required) - set(optional):
        invalid("Unknown or missing Pinterest fields")
    return value


def identity(value):
    try:
        if str(uuid.UUID(value)) != value:
            raise ValueError()
    except (ValueError, TypeError, AttributeError):
        invalid("A canonical UUID is required")
    return value


def integer(value, low=0, high=9007199254740991):
    if type(value) is not int or not low <= value <= high:
        invalid("Pinterest integer is outside its allowed range")
    return value


def pin_id(value):
    if not isinstance(value, str):
        invalid("Pin IDs must be strings")
    value = value.strip()
    if re.fullmatch(r"[1-9][0-9]{0,19}", value):
        return value
    try:
        url = urlsplit(value)
        match = re.fullmatch(r"/pin/([1-9][0-9]{0,19})/?", url.path)
        if (url.scheme == "https" and url.hostname in {"pinterest.com", "www.pinterest.com"}
                and not url.username and not url.password and url.port in (None, 443) and match):
            return match[1]
    except ValueError:
        pass
    invalid("Enter a decimal Pin ID or a https://www.pinterest.com/pin/<id>/ link")


def definition(value):
    fields(value, ("version", "collector", "library_id", "seeds"),
           ("access", "scope", "discovery", "metadata", "media", "run_budget"))
    integer(value["version"], 1, 1)
    if value["collector"] != COLLECTOR:
        invalid("Unsupported Pinterest collector")
    library = identity(value["library_id"])
    access = fields(value.get("access", {}), (), ("mode", "language"))
    language = access.get("language", "zh-TW")
    if access.get("mode", "anonymous") != "anonymous" or not isinstance(language, str) or not re.fullmatch(r"[a-z]{2}(?:-[A-Za-z]{2,8})?", language):
        invalid("Only anonymous access with a language code is currently supported")
    seeds = value["seeds"]
    if not isinstance(seeds, list) or not 1 <= len(seeds) <= 500:
        invalid("Specify 1–500 Pin seeds")
    ids = []
    for seed in seeds:
        fields(seed, ("kind", "id"))
        if seed["kind"] != "pin":
            invalid("Only specified Pin seeds are currently supported")
        ids.append(pin_id(seed["id"]))
    scope = value.get("scope", dict(media_types=["image"], ai_policy="record_only"))
    if scope != dict(media_types=["image"], ai_policy="record_only"):
        invalid("Only static images with AI information recorded are currently supported")
    discovery = value.get("discovery", dict(entrypoints=[], max_depth=0))
    if discovery != dict(entrypoints=[], max_depth=0):
        invalid("Discovery entrypoints are not enabled in this collector version")
    metadata = fields(value.get("metadata", {}), (), ("detail_enrichment",))
    enrichment = metadata.get("detail_enrichment", "sample")
    if enrichment not in ("none", "sample", "all"):
        invalid("Unknown detail enrichment policy")
    if enrichment == "all":
        invalid("Extended metadata enrichment is not enabled in this collector version")
    media = dict(image_policy=dict(profile="original", existing="match_profile", allow_sample=False),
                 retain_original=True, reuse=dict(mode="revalidate", max_age_hours=0))
    if value.get("media", media) != media:
        invalid("This version preserves original bytes and does not reuse acquisitions across runs")
    supplied = fields(value.get("run_budget", {}), (), DEFAULT_BUDGET)
    budget = {**DEFAULT_BUDGET, **supplied}
    for key in budget:
        integer(budget[key], 0 if key == "admitted_boards" else 1,
                7 * 86400 if key == "wall_seconds" else 9007199254740991)
    result = dict(version=1, collector=COLLECTOR, library_id=library, access=dict(mode="anonymous", language=language),
                  seeds=[dict(kind="pin", id=v) for v in sorted(set(ids), key=lambda v: (len(v), v))],
                  scope=scope, discovery=discovery, metadata=dict(detail_enrichment=enrichment), media=media, run_budget=budget)
    if len(canonical(result).encode()) > 64 * 1024:
        invalid("Pinterest definition exceeds its byte budget")
    return result
