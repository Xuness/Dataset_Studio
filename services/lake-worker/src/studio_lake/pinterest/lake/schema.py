"""Independent Pinterest facts; explicit Arrow schemas are derived from its own DDL."""

from functools import lru_cache
import json
from pathlib import Path

import apsw
import pyarrow as pa

from ...canonical import canonical, utc
from ...util import IntegrityError, digest

FACTS = ("visibility_contexts", "captures", "source_entities", "entity_observations", "pins", "pin_observations",
         "media_manifests", "media_entries", "objects", "acquisitions", "assets", "source_relations",
         "discovery_snapshots", "discovery_members")
DERIVED = {"object_row", "first_seq", "commit_seq"}
BOOL = {"complete", "exhausted"}
MAX_ROWS = 10_000
MAX_METADATA = 32 * 1024**2


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


def primary_key(name):
    keys = sorted((r[5], r[1]) for r in definitions()[name] if r[5] and r[1] not in DERIVED)
    return [v for _, v in keys] or ["sha256"]


@lru_cache(maxsize=len(FACTS))
def arrow_schema(name):
    fields = []
    for _, column, kind, required, _, primary in definitions()[name]:
        if column in DERIVED:
            continue
        if column == "raw_zlib":
            column = "raw_body"
        if name == "objects" and column == "pack_path":
            fields.extend([pa.field("pack_file", pa.string(), False), pa.field("member_name", pa.string(), False)])
            continue
        fields.append(pa.field(column, pa.bool_() if column in BOOL else {"TEXT": pa.string(), "INTEGER": pa.int64(), "BLOB": pa.binary()}[kind],
                               not bool(required or primary)))
    return pa.schema(fields, metadata={b"studio_schema": ("pinterest-media-v1:" + name).encode()})


def check_rows(records):
    if set(records) - set(FACTS) or sum(map(len, records.values())) > MAX_ROWS:
        raise IntegrityError("Invalid Pinterest record set or row budget")
    size = 0
    for name, rows in records.items():
        schema = arrow_schema(name)
        for row in rows:
            if set(row) != set(schema.names):
                raise IntegrityError("Pinterest record fields differ: " + name)
            for field in schema:
                value = row[field.name]
                if value is None:
                    if not field.nullable:
                        raise IntegrityError("Missing Pinterest field: " + field.name)
                elif ((pa.types.is_boolean(field.type) and type(value) is not bool)
                      or (pa.types.is_integer(field.type) and (type(value) is not int or not -(2**63) <= value < 2**63))
                      or (pa.types.is_string(field.type) and not isinstance(value, str))
                      or (pa.types.is_binary(field.type) and not isinstance(value, bytes))):
                    raise IntegrityError("Invalid Pinterest field type: " + field.name)
                if isinstance(value, str) and field.name.endswith("_json"):
                    canonical(json.loads(value))
                if isinstance(value, str) and field.name.endswith("_at") and utc(value) != value:
                    raise IntegrityError("Pinterest timestamps must be canonical UTC")
            if name == "captures" and (len(row["raw_body"]) != row["raw_bytes"] or digest(row["raw_body"]) != row["raw_sha256"]):
                raise IntegrityError("Pinterest capture hash/size mismatch")
        size += pa.Table.from_pylist(rows, schema=schema).nbytes
        if size > MAX_METADATA:
            raise IntegrityError("Pinterest metadata exceeds batch budget")
