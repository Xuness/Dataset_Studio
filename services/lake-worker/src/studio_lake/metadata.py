import json
from datetime import datetime, timezone

import pyarrow as pa

from .util import now, stable_id, typed_value

NORMALIZATION_VERSION = 1

TEXT_FIELDS = [
    "rating",
    "tag_string",
    "tag_string_general",
    "tag_string_artist",
    "tag_string_character",
    "tag_string_copyright",
    "tag_string_meta",
    "source",
    "md5",
    "file_ext",
    "file_url",
    "large_file_url",
    "preview_file_url",
]
INT_FIELDS = [
    "score",
    "up_score",
    "down_score",
    "fav_count",
    "image_width",
    "image_height",
    "file_size",
    "tag_count",
    "uploader_id",
    "parent_id",
    "pixiv_id",
]
BOOL_FIELDS = ["is_deleted", "is_banned", "is_pending", "is_flagged"]
TIME_FIELDS = ["created_at", "updated_at"]


def normalization_columns(schema):
    wanted = {
        "id",
        *TEXT_FIELDS,
        *INT_FIELDS,
        *BOOL_FIELDS,
        *TIME_FIELDS,
        "raw_stored_ext",
        "raw_sha256",
        "raw_storage_profile",
    }
    return [c for c in schema.names if c in wanted]


def normalization_rows(table):
    result = [{} for _ in range(table.num_rows)]
    for name in normalization_columns(table.schema):
        column = table[name]
        if pa.types.is_timestamp(column.type) and column.type.unit == "ns":
            # Analytical timestamps have microsecond precision; source arrays/typed JSON retain ns.
            column = column.cast(pa.timestamp("us", tz=column.type.tz), safe=False)
        for row, value in zip(result, column.to_pylist()):
            row[name] = value
    return result


OBS_SCHEMA = pa.schema(
    [
        ("observation_id", pa.string()),
        ("post_id", pa.int64()),
        ("source_key", pa.string()),
        ("source_row", pa.int64()),
        ("archive_row", pa.int64()),
        ("source_kind", pa.string()),
        ("observed_at", pa.string()),
        ("time_quality", pa.string()),
        ("ingested_at", pa.string()),
        ("issues_json", pa.string()),
        ("publication_kind", pa.string()),
        ("publication_group", pa.string()),
        ("source_priority", pa.int64()),
    ]
    + [(x, pa.string()) for x in TEXT_FIELDS + TIME_FIELDS]
    + [(x, pa.int64()) for x in INT_FIELDS]
    + [(x, pa.bool_()) for x in BOOL_FIELDS]
)

ASSET_SCHEMA = pa.schema(
    [
        ("asset_id", pa.string()),
        ("observation_id", pa.string()),
        ("post_id", pa.int64()),
        ("sha256", pa.string()),
        ("source_md5", pa.string()),
        ("stored_ext", pa.string()),
        ("stored_bytes", pa.int64()),
        ("storage_profile", pa.string()),
        ("details_json", pa.string()),
    ]
)

OBJECT_SCHEMA = pa.schema(
    [
        ("sha256", pa.string()),
        ("member_name", pa.string()),
        ("offset", pa.int64()),
        ("length", pa.int64()),
        ("stored_ext", pa.string()),
    ]
)

EVENT_SCHEMA = pa.schema(
    [
        ("observation_id", pa.string()),
        ("status", pa.string()),
        ("reason", pa.string()),
        ("details_json", pa.string()),
        ("recorded_at", pa.string()),
    ]
)


def integer(value):
    if value is None:
        return None
    if isinstance(value, bool):
        raise ValueError("boolean is not an integer identifier/count")
    if isinstance(value, int):
        n = value
    elif isinstance(value, str) and value.strip().lstrip("+-").isdigit():
        n = int(value)
    elif isinstance(value, float) and value.is_integer() and abs(value) <= 2**53:
        n = int(value)
    else:
        raise ValueError("cannot normalize integer without ambiguity")
    if not -(2**63) <= n < 2**63:
        raise ValueError("outside signed int64; original value retained")
    return n


def iso_time(value):
    if value is None or value == "":
        return None
    dt = value if isinstance(value, datetime) else datetime.fromisoformat(str(value).replace("Z", "+00:00"))
    if dt.tzinfo is None:
        raise ValueError("timestamp has no timezone")
    return dt.astimezone(timezone.utc).isoformat()


def normalize(record, source_key, row, kind, archive_row, observed_at=None, time_quality="unknown"):
    issues = []
    out = dict(
        observation_id=stable_id("observation-v1", source_key, row),
        source_key=source_key,
        source_row=row,
        archive_row=archive_row,
        source_kind=kind,
        observed_at=observed_at,
        time_quality=time_quality,
        ingested_at=now(),
        publication_kind="api" if kind == "api_json" else "legacy",
        publication_group=None,
        source_priority=100
        if kind == "api_json"
        else 10
        if kind == "orphan_index"
        else 20
        if kind == "legacy_auxiliary"
        else 80,
    )
    for key in ["id"] + INT_FIELDS:
        try:
            out["post_id" if key == "id" else key] = integer(record.get(key))
        except (ValueError, TypeError, OverflowError) as e:
            out["post_id" if key == "id" else key] = None
            issues.append({"field": key, "reason": str(e)})
    if out["post_id"] is not None and out["post_id"] <= 0:
        issues.append({"field": "id", "reason": "non-positive post ID"})
        out["post_id"] = None
    for key in TEXT_FIELDS:
        value = record.get(key)
        out[key] = value if isinstance(value, str) or value is None else None
        if value is not None and not isinstance(value, str):
            issues.append({"field": key, "reason": "expected string"})
    for key in BOOL_FIELDS:
        value = record.get(key)
        out[key] = value if isinstance(value, bool) or value is None else None
        if value is not None and not isinstance(value, bool):
            issues.append({"field": key, "reason": "expected boolean"})
    for key in TIME_FIELDS:
        try:
            out[key] = iso_time(record.get(key))
        except (ValueError, TypeError) as e:
            out[key] = None
            issues.append({"field": key, "reason": str(e)})
    out["issues_json"] = json.dumps(issues, ensure_ascii=False)
    return out


def asset(observation, sha, ext, length, profile="legacy-preserved", details=None):
    return dict(
        asset_id=stable_id("asset-v1", observation["observation_id"], sha, profile),
        observation_id=observation["observation_id"],
        post_id=observation["post_id"],
        sha256=sha,
        source_md5=observation.get("md5"),
        stored_ext=ext,
        stored_bytes=length,
        storage_profile=profile,
        details_json=json.dumps(typed_value(details or {}), ensure_ascii=False),
    )
