"""Revisioned execution tuning; image recipes and job membership stay immutable."""

import copy
import json
import math

from .sites import UpdateError

MIB = 1024**2
SITES = ("danbooru", "yandere", "gelbooru")
DEFAULTS = {
    "version": 1,
    "scan_mode": "pipeline",
    "active_lakes": 3,
    "sites": {
        site: {
            "download_concurrency": 4,
            "api_requests_per_second": 0.9,
            "image_requests_per_second": 2.0,
        }
        for site in SITES
    },
    "encode_concurrency": 2,
    "buffer_images": 64,
    "metadata_prefetch_records": 5000,
    "spool_mib": 8192,
    "reserve_mib": 2048,
    "max_download_mib": 128,
    "max_image_pixels": 100_000_000,
    "decode_memory_mib": 2048,
    "publish_items": 32,
    "publish_mib": 64,
    "publish_interval_seconds": 10.0,
    "download_mib_per_second": None,
}


def validate(value):
    if not isinstance(value, dict) or set(value) - set(DEFAULTS):
        raise UpdateError("INVALID_INPUT", "Unknown pipeline settings")
    result = copy.deepcopy(DEFAULTS)
    result.update({k: v for k, v in value.items() if k != "sites"})

    def number(container, key, low, high, *, integer=True, optional=False):
        n = container[key]
        if optional and n is None:
            return
        if (
            isinstance(n, bool)
            or not isinstance(n, int if integer else (int, float))
            or not low <= n <= high
            or not math.isfinite(n)
        ):
            raise UpdateError(
                "INVALID_INPUT",
                f"Pipeline {key} must be {'an integer' if integer else 'a number'} in {low}..{high}",
            )

    number(result, "version", 1, 1)
    if result["scan_mode"] not in {"pipeline", "metadata_first"}:
        raise UpdateError("INVALID_INPUT", "Unknown metadata scan mode")
    for key, low, high in (
        ("active_lakes", 1, 3),
        ("encode_concurrency", 1, 16),
        ("buffer_images", 1, 512),
        ("metadata_prefetch_records", 200, 1_000_000),
        ("spool_mib", 64, 1_048_576),
        ("reserve_mib", 0, 1_048_576),
        ("max_download_mib", 1, 4096),
        ("max_image_pixels", 1, 1_000_000_000),
        ("decode_memory_mib", 64, 1_048_576),
        ("publish_items", 1, 512),
        ("publish_mib", 1, 65536),
    ):
        number(result, key, low, high)
    number(result, "publish_interval_seconds", 0.1, 300, integer=False)
    number(result, "download_mib_per_second", 0.05, 4096, integer=False, optional=True)
    sites = value.get("sites", {})
    if not isinstance(sites, dict) or set(sites) - set(SITES):
        raise UpdateError("INVALID_INPUT", "Unknown pipeline site")
    for site, values in sites.items():
        if not isinstance(values, dict) or set(values) - set(DEFAULTS["sites"][site]):
            raise UpdateError("INVALID_INPUT", "Unknown site pipeline setting")
        result["sites"][site].update(values)
    for values in result["sites"].values():
        number(values, "download_concurrency", 1, 16)
        number(values, "api_requests_per_second", 0.05, 10, integer=False)
        number(values, "image_requests_per_second", 0.05, 50, integer=False, optional=True)
    if result["spool_mib"] < 4 * result["max_download_mib"]:
        raise UpdateError("INVALID_INPUT", "SSD spool must allow at least four maximum-sized image buffers")
    return result


def read(state):
    with state.db() as db:
        row = db.execute("SELECT value FROM settings WHERE key='pipeline_v1'").fetchone()
    saved = json.loads(row[0]) if row else {"revision": 0, "value": {}}
    return {
        "revision": saved["revision"],
        "value": validate(saved["value"]),
        "defaults": copy.deepcopy(DEFAULTS),
    }


def save(state, value, expected_revision):
    value = validate(value)
    if isinstance(expected_revision, bool) or not isinstance(expected_revision, int) or expected_revision < 0:
        raise UpdateError("INVALID_INPUT", "Expected pipeline revision required")
    with state.db() as db:
        db.execute("BEGIN IMMEDIATE")
        row = db.execute("SELECT value FROM settings WHERE key='pipeline_v1'").fetchone()
        revision = json.loads(row[0])["revision"] if row else 0
        if revision != expected_revision:
            raise UpdateError("REVISION_CONFLICT", "Pipeline settings changed; reload before saving")
        saved = {"revision": revision + 1, "value": value}
        db.execute(
            "INSERT INTO settings VALUES('pipeline_v1',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            (json.dumps(saved),),
        )
    return {**saved, "defaults": copy.deepcopy(DEFAULTS)}
