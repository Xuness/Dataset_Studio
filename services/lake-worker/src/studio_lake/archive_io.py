"""Bounded immutable file envelopes and journal acceptance, without source semantics."""

import json

import pyarrow as pa
import pyarrow.parquet as pq

from .canonical import canonical, utc
from .util import IntegrityError, contained, failpoint, file_hash, sync_directory, sync_file


def descriptor(path, role, schema, rows):
    return dict(role=role, schema=schema, rows=rows, bytes=path.stat().st_size, sha256=file_hash(path))


def write_facts(directory, records, names, schema, prefix, max_bytes):
    files, total = {}, 0
    for name in names:
        rows = records.get(name, [])
        if not rows:
            continue
        table = pa.Table.from_pylist(rows, schema=schema(name))
        total += table.nbytes
        if total > max_bytes:
            raise IntegrityError("Canonical batch exceeds metadata budget")
        path = directory / (name + ".parquet")
        pq.write_table(table, path, compression="zstd", version="2.6", store_schema=True, row_group_size=4096)
        sync_file(path)
        files[path.name] = descriptor(path, "facts", prefix + name, len(rows))
    return files


def read_facts(directory, manifest, names, schema, prefix, max_rows, max_bytes):
    result, total, size = {}, 0, 0
    for name in names:
        info = manifest["files"].get(name + ".parquet")
        if not info:
            continue
        if info.get("schema") != prefix + name or info.get("role") != "facts":
            raise IntegrityError("Unknown canonical Parquet schema")
        parquet = pq.ParquetFile(contained(directory, name + ".parquet"))
        if not parquet.schema_arrow.equals(schema(name), check_metadata=True):
            raise IntegrityError("Archive schema differs from its declared record set")
        total += parquet.metadata.num_rows
        if total > max_rows or parquet.metadata.num_rows != info.get("rows"):
            raise IntegrityError("Archive row count exceeds its declared budget")
        rows = []
        for chunk in parquet.iter_batches(batch_size=256):
            size += chunk.nbytes
            if size > max_bytes:
                raise IntegrityError("Archive metadata exceeds batch budget")
            rows.extend(chunk.to_pylist())
        result[name] = rows
    return result


def accept_batch(lib, directory, manifest, journal_writer, *, fence=None, fault_prefix, fault=None):
    """The owner validates source relations first and holds its lake writer lock."""
    text = canonical(manifest)
    final = lib.root / "segments" / manifest["batch_id"]
    if directory != final:
        if final.exists():
            raise IntegrityError("Sealed batch destination is already occupied")
        directory.rename(final)
        sync_directory(final.parent)
    emit = fault or failpoint
    emit(fault_prefix + "_after_rename")
    with lib.journal() as db, db:
        if fence:
            fence()
        previous = db.execute("SELECT seq,manifest_json FROM commits WHERE dedupe_key=?", (manifest["dedupe_key"],)).fetchone()
        if previous:
            if canonical(json.loads(previous["manifest_json"])) != text:
                raise IntegrityError("Idempotent batch has different immutable content")
            return previous["seq"]
        db.execute("INSERT INTO commits(batch_id,dedupe_key,manifest_json,committed_at) VALUES(?,?,?,?)",
                   (manifest["batch_id"], manifest["dedupe_key"], text, utc()))
        seq = db.execute("SELECT last_insert_rowid()").fetchone()[0]
        journal_writer(db, seq)
    emit(fault_prefix + "_after_commit")
    return seq
