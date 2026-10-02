"""One schema definition for validation, Arrow archives and online publication."""

from datetime import datetime, timezone
from functools import lru_cache
import json
from pathlib import Path

import apsw
import pyarrow as pa

from ..util import IntegrityError, digest

FACTS = (
    "visibility_contexts", "captures", "authors", "author_observations", "works",
    "work_observations", "work_tags", "media_manifests", "media_entries", "animation_frames",
    "objects", "assets", "discovery_snapshots", "discovery_members",
)
DERIVED_COLUMNS = {"row_id", "object_row", "commit_seq", "first_seq"}
BOOL_COLUMNS = {"verified", "locked", "complete", "page_complete", "traversal_exhausted"}
NORMALIZER = "pixiv-web-v1"
MAX_BATCH_ROWS = 100_000
MAX_BATCH_METADATA_BYTES = 64 * 1024**2


def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False)


def utc(value=None):
    date = datetime.now(timezone.utc) if value is None else datetime.fromisoformat(value.replace("Z", "+00:00"))
    if date.tzinfo is None:
        raise IntegrityError("A source timestamp requires a timezone")
    return date.astimezone(timezone.utc).isoformat(timespec="microseconds").replace("+00:00", "Z")


def sql(name):
    return Path(__file__).with_name(name + ".sql").read_text(encoding="utf-8")


@lru_cache(maxsize=1)
def definitions():
    db = apsw.Connection(":memory:")
    try:
        db.execute(sql("online"))
        return {name: list(db.execute('PRAGMA table_info("' + name + '")')) for name in FACTS}
    finally:
        db.close()


def columns(name):
    return [r[1] for r in definitions()[name] if r[1] not in DERIVED_COLUMNS]


def primary_key(name):
    keys = sorted((r[5], r[1]) for r in definitions()[name] if r[5] and r[1] not in DERIVED_COLUMNS)
    return [k for _, k in keys] or [{"objects": "sha256", "work_observations": "observation_id"}[name]]


@lru_cache(maxsize=len(FACTS))
def arrow_schema(name):
    fields = []
    for _, column, kind, required, _, primary in definitions()[name]:
        if column in DERIVED_COLUMNS:
            continue
        if name == "captures" and column == "raw_zlib":
            column, kind = "raw_body", "BLOB"
        elif name == "objects" and column == "pack_path":
            fields.extend([pa.field("pack_file", pa.string(), False), pa.field("member_name", pa.string(), False)])
            continue
        elif name == "work_tags" and column == "tag_id":
            column, kind = "tag", "TEXT"
        typ = pa.bool_() if column in BOOL_COLUMNS else {"TEXT": pa.string(), "INTEGER": pa.int64(), "BLOB": pa.binary()}[kind]
        fields.append(pa.field(column, typ, not bool(required or primary)))
    return pa.schema(fields, metadata={b"studio_schema": ("canonical-media-v2:" + name).encode()})


def check_rows(records):
    if set(records) - set(FACTS):
        raise IntegrityError("Unknown canonical record set")
    total = 0
    for name, rows in records.items():
        schema = arrow_schema(name)
        allowed = set(schema.names)
        for row in rows:
            if set(row) != allowed:
                raise IntegrityError("Record fields do not match " + name)
            for field in schema:
                value = row[field.name]
                if value is None:
                    if not field.nullable:
                        raise IntegrityError("Missing required field " + name + "." + field.name)
                elif pa.types.is_boolean(field.type):
                    if not isinstance(value, bool):
                        raise IntegrityError("Boolean required: " + field.name)
                elif pa.types.is_integer(field.type):
                    if isinstance(value, bool) or not isinstance(value, int) or not -(2**63) <= value < 2**63:
                        raise IntegrityError("Integer required: " + field.name)
                elif pa.types.is_binary(field.type):
                    if not isinstance(value, bytes):
                        raise IntegrityError("Bytes required: " + field.name)
                elif not isinstance(value, str):
                    raise IntegrityError("String required: " + field.name)
                if isinstance(value, str) and field.name.endswith("_json"):
                    canonical(json.loads(value))
                if isinstance(value, str) and field.name.endswith("_at") and utc(value) != value:
                    raise IntegrityError("Canonical timestamps require UTC with six fractional digits")
            if name == "captures" and (len(row["raw_body"]) != row["raw_bytes"] or digest(row["raw_body"]) != row["raw_sha256"]):
                raise IntegrityError("Raw capture length or hash mismatch")
            total += 1
    if total > MAX_BATCH_ROWS:
        raise IntegrityError("Canonical batch exceeds its record budget")


def empty_replay(job_id, definition_sha256, receipt_ids=()):
    return dict(version=1, job_id=job_id, definition_sha256=definition_sha256,
                receipt_ids=list(receipt_ids), task_outcomes=[], checkpoint_advances=[], discovery_snapshot_ids=[])
