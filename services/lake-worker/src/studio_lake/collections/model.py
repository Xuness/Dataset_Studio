"""Strict, transport-independent validation of frozen collection definitions."""

import copy
import math
import re
import uuid

from ..image_policy import ImagePolicyError, media_policy
from ..updates.sites import UpdateError


def invalid(message):
    raise UpdateError("INVALID_INPUT", message)


def fields(value, required, optional=()):
    if not isinstance(value, dict) or set(value) - set(required) - set(optional) or set(required) - set(value):
        invalid("Unknown or missing collection fields")
    return value


def identity(value):
    try:
        if str(uuid.UUID(value)) != value:
            raise ValueError()
    except (TypeError, ValueError, AttributeError):
        invalid("A canonical UUID is required")
    return value


def source_id(value):
    if not isinstance(value, str) or not re.fullmatch(r"[1-9][0-9]{0,19}", value):
        invalid("Pixiv identities must be decimal strings with at most 20 digits")
    return value


def integer(value, low=0, high=9007199254740991):
    if isinstance(value, bool) or not isinstance(value, int) or not low <= value <= high:
        invalid("Collection integer is outside its allowed range")
    return value


def boolean(value):
    if not isinstance(value, bool):
        invalid("Collection boolean required")
    return value


def choice(value, allowed):
    if value not in allowed:
        invalid("Unsupported collection option")
    return value


def choices(value, allowed, *, empty=False):
    if not isinstance(value, list) or len(value) > len(allowed) or (not value and not empty) or any(not isinstance(v, str) or v not in allowed for v in value) or len(set(value)) != len(value):
        invalid("Invalid collection option list")
    return sorted(value)


def definition(value):
    value = copy.deepcopy(fields(value, ("version", "collector", "library_id", "account_id", "seeds", "scope", "discovery", "media", "run_budget")))
    integer(value["version"], 1, 1)
    choice(value["collector"], ("pixiv_web_v1",))
    identity(value["library_id"])
    identity(value["account_id"])
    seeds = fields(value["seeds"], ("kind", "ids"))
    choice(seeds["kind"], ("authors", "works"))
    if not isinstance(seeds["ids"], list) or not 1 <= len(seeds["ids"]) <= 1000:
        invalid("Supply 1–1000 author or work identities")
    seeds["ids"] = sorted({source_id(v) for v in seeds["ids"]}, key=int)
    scope = fields(value["scope"], ("work_types", "ratings", "include_ai", "include_unknown_markers"))
    scope["work_types"] = choices(scope["work_types"], ("illustration", "manga", "ugoira"))
    scope["ratings"] = choices(scope["ratings"], ("all_ages", "r18", "r18g"))
    boolean(scope["include_ai"])
    boolean(scope["include_unknown_markers"])
    discovery = fields(value["discovery"], ("entrypoints", "max_depth", "recommendation_seeds_per_author"))
    discovery["entrypoints"] = choices(discovery["entrypoints"], ("bookmarks", "following", "recommendations"), empty=True)
    depth = integer(discovery["max_depth"], 0, 4)
    count = integer(discovery["recommendation_seeds_per_author"], 0, 32)
    if (not depth and (discovery["entrypoints"] or count)) or (depth and not discovery["entrypoints"]):
        invalid("Discovery depth and entrypoints disagree")
    if "recommendations" in discovery["entrypoints"] and count == 0:
        invalid("Recommendations require at least one deterministic seed")
    if seeds["kind"] == "works" and depth:
        invalid("Explicit work snapshots cannot expand authors")
    media = fields(value["media"], ("image_policy", "retain_original", "ugoira", "reuse"))
    try:
        media["image_policy"] = media_policy(media["image_policy"])
    except ImagePolicyError:
        invalid("Invalid image preservation recipe")
    policy = media["image_policy"]
    if policy["existing"] != "match_profile" or policy["allow_sample"]:
        raise UpdateError("COLLECTION_POLICY_UNSUPPORTED", "Collection v1 requires matching recipes and original media URLs")
    boolean(media["retain_original"])
    choice(media["ugoira"], ("archive_with_poster", "metadata_only"))
    if policy["profile"] == "original" and not media["retain_original"]:
        invalid("The original profile requires retaining original bytes")
    if policy["profile"] == "metadata_only" and (media["retain_original"] or media["ugoira"] != "metadata_only"):
        invalid("Metadata-only collection cannot request media representations")
    reuse = fields(media["reuse"], ("mode", "max_age_hours"))
    choice(reuse["mode"], ("revalidate", "historical_if_same_locator"))
    integer(reuse["max_age_hours"], 0, 8760)
    budget = fields(value["run_budget"], ("api_requests", "admitted_authors", "download_bytes", "wall_seconds"))
    for key, lo, hi in (("api_requests", 1, 1_000_000), ("admitted_authors", 1, 1_000_000),
                        ("download_bytes", 1048576, 109951162777600), ("wall_seconds", 60, 604800)):
        integer(budget[key], lo, hi)
    return value


PIPELINE_DEFAULTS = dict(pixiv=dict(download_concurrency=2, api_requests_per_second=1.0, image_requests_per_second=2.0),
                         metadata_concurrency=1, pending_media_limit=5000, publication_backlog_mib=256, time_slice_seconds=30)


def pipeline(value):
    result = copy.deepcopy(fields(value, PIPELINE_DEFAULTS))
    pixiv = fields(result["pixiv"], PIPELINE_DEFAULTS["pixiv"])
    integer(pixiv["download_concurrency"], 1, 16)
    for key, high in (("api_requests_per_second", 10), ("image_requests_per_second", 50)):
        rate = pixiv[key]
        if rate is None and key == "image_requests_per_second":
            continue
        if isinstance(rate, bool) or not isinstance(rate, (int, float)) or not math.isfinite(rate) or not .05 <= rate <= high:
            invalid("Pixiv request rate is outside its allowed range")
    integer(result["metadata_concurrency"], 1, 4)
    integer(result["pending_media_limit"], 1, 100_000)
    integer(result["publication_backlog_mib"], 16, 256)
    integer(result["time_slice_seconds"], 1, 300)
    return result


def sample_definition(library_id, account_id, ids, *, kind="authors", metadata_only=False):
    return definition(dict(version=1, collector="pixiv_web_v1", library_id=library_id, account_id=account_id,
         seeds=dict(kind=kind, ids=ids), scope=dict(work_types=["illustration", "manga", "ugoira"],
         ratings=["all_ages", "r18", "r18g"], include_ai=True, include_unknown_markers=True),
         discovery=dict(entrypoints=[], max_depth=0, recommendation_seeds_per_author=0),
         media=dict(image_policy=dict(profile="metadata_only" if metadata_only else "original", existing="match_profile", allow_sample=False),
                    retain_original=not metadata_only, ugoira="metadata_only" if metadata_only else "archive_with_poster",
                    reuse=dict(mode="revalidate", max_age_hours=0)),
         run_budget=dict(api_requests=500, admitted_authors=10, download_bytes=1024**3, wall_seconds=3600)))
