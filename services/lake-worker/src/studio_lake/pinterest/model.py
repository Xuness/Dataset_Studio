"""Frozen Pinterest definitions. Unsupported capabilities fail before a job is created."""

import re
import uuid
from urllib.parse import unquote, urlsplit

from . import COLLECTOR
from ..canonical import canonical
from ..updates.sites import UpdateError

TERMINAL = frozenset(("completed", "completed_with_gaps", "cancelled"))
SITE_LIMITS = dict(download_concurrency=2, api_requests_per_second=0.5, image_requests_per_second=1.0)
DEFAULT_BUDGET = dict(api_requests=100, admitted_pins=100, admitted_boards=20, detail_requests=100,
                      download_bytes=1024**3, wall_seconds=3600)
PAGE_KINDS = frozenset(("board_page", "section_page", "board_sections", "board_more_ideas", "related_pins", "search_page", "topic_page"))
NETWORK_KINDS = PAGE_KINDS | {"pin_detail", "pin_enrichment", "board_resolve", "section_resolve"}
ENTRYPOINTS = frozenset(("board_more_ideas", "related_pins", "pin_boards", "topic_boards"))
SEED_KINDS = ("pin", "board", "section", "search_pins", "search_boards", "topic")


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


def source_id(value, kind):
    if kind == "pin":
        return pin_id(value)
    if not isinstance(value, str) or len(value) > 2048:
        invalid("Pinterest source IDs and links must be bounded strings")
    value = value.strip()
    if kind in ("search_pins", "search_boards"):
        if not value or len(value) > 512 or any(ord(c) < 32 for c in value):
            invalid("Search queries must contain 1–512 printable characters")
        return value
    if kind != "topic" and re.fullmatch(r"[1-9][0-9]{0,19}", value):
        return value
    try:
        url = urlsplit(value)
        parts = [unquote(p) for p in url.path.strip("/").split("/")]
        if (url.scheme == "https" and url.hostname in {"pinterest.com", "www.pinterest.com"}
                and not url.username and not url.password and url.port in (None, 443)
                and len(parts) == (2 if kind == "board" else 3)
                and ((kind == "topic" and parts[0] == "ideas" and re.fullmatch(r"[1-9][0-9]{0,19}", parts[-1]))
                     or (kind != "topic" and parts[0] not in {"pin", "ideas", "search", "settings"}))
                and all(p and p not in (".", "..") and not re.search(r"[/?#\\\x00-\x20]", p) for p in parts)):
            return "https://www.pinterest.com/" + "/".join(parts) + "/"
    except ValueError:
        pass
    invalid("Enter a Pinterest " + kind + " ID or its complete board/section link")


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
        invalid("Specify 1–500 Pinterest seeds")
    ids = []
    for seed in seeds:
        fields(seed, ("kind", "id"))
        if seed["kind"] not in SEED_KINDS:
            invalid("Unsupported Pinterest seed kind")
        item = dict(kind=seed["kind"], id=source_id(seed["id"], seed["kind"]))
        if item not in ids:
            ids.append(item)
    scope = value.get("scope", dict(media_types=["image"], ai_policy="record_only"))
    if scope != dict(media_types=["image"], ai_policy="record_only"):
        invalid("Only static images with AI information recorded are currently supported")
    supplied_discovery = fields(value.get("discovery", {}), (),
        ("entrypoints", "max_depth", "include_sections", "max_pending_downloads", "entry_requests"))
    discovery = dict(entrypoints=[], max_depth=0, include_sections=True, max_pending_downloads=128,
                     entry_requests={k: 100 for k in sorted(PAGE_KINDS)})
    discovery.update(supplied_discovery)
    if (not isinstance(discovery["entrypoints"], list)
            or any(not isinstance(k, str) or k not in ENTRYPOINTS for k in discovery["entrypoints"])):
        invalid("Unsupported Pinterest discovery entrypoint")
    discovery["entrypoints"] = sorted(set(discovery["entrypoints"]))
    integer(discovery["max_depth"], 0, 3)
    if discovery["entrypoints"] and discovery["max_depth"] == 0:
        invalid("Optional discovery requires a positive maximum depth")
    integer(discovery["max_pending_downloads"], 1, 1000)
    if type(discovery["include_sections"]) is not bool:
        invalid("include_sections must be a boolean")
    fields(discovery["entry_requests"], (), PAGE_KINDS)
    discovery["entry_requests"] = {k: integer(discovery["entry_requests"].get(k, 100), 0, 10000) for k in sorted(PAGE_KINDS)}
    metadata = fields(value.get("metadata", {}), (), ("detail_enrichment", "sample_size"))
    enrichment = metadata.get("detail_enrichment", "sample")
    if enrichment not in ("none", "sample", "all"):
        invalid("Unknown detail enrichment policy")
    sample_size = integer(metadata.get("sample_size", 3), 1, 20)
    media = dict(image_policy=dict(profile="original", existing="match_profile", allow_sample=False),
                 retain_original=True, reuse=dict(mode="revalidate", max_age_hours=0))
    supplied_media = fields(value.get("media", media), ("image_policy", "retain_original", "reuse"))
    if (supplied_media["image_policy"] != media["image_policy"] or supplied_media["retain_original"] is not True
            or type(supplied_media["image_policy"].get("allow_sample")) is not bool):
        invalid("Pinterest preserves original bytes using the original image profile")
    reuse = fields(supplied_media["reuse"], ("mode", "max_age_hours"))
    if reuse["mode"] not in ("revalidate", "historical"):
        invalid("Unsupported Pinterest reuse policy")
    integer(reuse["max_age_hours"], 0, 8760)
    media = supplied_media
    supplied = fields(value.get("run_budget", {}), (), DEFAULT_BUDGET)
    budget = {**DEFAULT_BUDGET, **supplied}
    for key in budget:
        integer(budget[key], 0 if key in ("admitted_boards", "detail_requests") else 1,
                7 * 86400 if key == "wall_seconds" else 9007199254740991)
    result = dict(version=1, collector=COLLECTOR, library_id=library, access=dict(mode="anonymous", language=language),
                  seeds=ids, scope=scope, discovery=discovery,
                  metadata=dict(detail_enrichment=enrichment, sample_size=sample_size), media=media, run_budget=budget)
    if len(canonical(result).encode()) > 64 * 1024:
        invalid("Pinterest definition exceeds its byte budget")
    return result
