"""Ordered, interruptible publication; pre-commit validation shares the same writer."""

import json
from pathlib import Path
import uuid

import apsw
import pyarrow.parquet as pq

from . import ONLINE_VERSION
from .schema import FACTS, MAX_BATCH_METADATA_BYTES, MAX_BATCH_ROWS, InvalidCanonicalResult, arrow_schema, canonical, check_rows, sql, utc
from .records import apply_facts, counts, insert_fact, publication_row, project, validate_relations
from ..online_schema import set_state, settings
from ..online_storage import connect
from ..util import FileLock, IntegrityError, atomic_json, contained, digest, read_json, same_directory, sync_file

MAX_PENDING_BATCHES = 512
MAX_PENDING_METADATA = 256 * 1024**2


def initialize(library, index, *, generation=None, activate=True):
    index = Path(index).resolve()
    index.mkdir(parents=True, exist_ok=True)
    marker = index / "ONLINE.json"
    generation = generation or str(uuid.uuid4())
    pointer = dict(schema_version=ONLINE_VERSION, library_id=library["library_id"], site=library["site"],
                   generation=generation, file="online.sqlite")
    if marker.exists():
        old = read_json(marker)
        if old["library_id"] != library["library_id"] or old["schema_version"] != ONLINE_VERSION or old["site"] != library["site"]:
            raise IntegrityError("Online initialization identity mismatch")
        return old
    path = index / "online.sqlite"
    db = connect(path)
    try:
        if db.execute("PRAGMA user_version").fetchone()[0] == 0:
            with db:
                db.execute(sql("online"))
                set_state(db, library_id=library["library_id"], generation=generation, site=library["site"],
                          served_seq=0, archive_seq=0, min_seq=0, schema_version=ONLINE_VERSION)
        else:
            state = settings(db)
            if state["library_id"] != library["library_id"] or db.execute("PRAGMA user_version").fetchone()[0] != ONLINE_VERSION:
                raise IntegrityError("Cannot initialize an existing unrelated online database")
            pointer["generation"] = state["generation"]
    finally:
        db.close()
    sync_file(path)
    if activate:
        atomic_json(marker, pointer)
    return pointer


def load_records(directory, manifest):
    result, total, bytes_count = {}, 0, 0
    for name in FACTS:
        info = manifest["files"].get(name + ".parquet")
        if not info:
            continue
        if info.get("schema") != "canonical-media-v2:" + name or info.get("role") != "facts":
            raise IntegrityError("Unknown canonical Parquet schema")
        path = contained(directory, name + ".parquet")
        parquet = pq.ParquetFile(path)
        if not parquet.schema_arrow.equals(arrow_schema(name), check_metadata=True):
            raise IntegrityError("Archive schema differs from its declared record set")
        total += parquet.metadata.num_rows
        if total > MAX_BATCH_ROWS or parquet.metadata.num_rows != info.get("rows"):
            raise IntegrityError("Archive row count exceeds its declared budget")
        rows = []
        for chunk in parquet.iter_batches(batch_size=256):
            bytes_count += chunk.nbytes
            if bytes_count > MAX_BATCH_METADATA_BYTES:
                raise IntegrityError("Archive metadata exceeds batch budget")
            rows.extend(chunk.to_pylist())
        result[name] = rows
    check_rows(result)
    return result


def pending_commits(lib, after, *, through=None):
    with lib.journal() as db:
        head = db.execute("SELECT coalesce(max(seq),0) FROM commits").fetchone()[0]
        end = head if through is None else through
        if end < after or end > head:
            raise IntegrityError("Invalid archive prefix")
        rows = [dict(r) for r in db.execute("SELECT * FROM commits WHERE seq>? AND seq<=? ORDER BY seq LIMIT ?", (after, end, MAX_PENDING_BATCHES + 1))]
    if len(rows) > MAX_PENDING_BATCHES:
        raise IntegrityError("Publication backlog exceeds its batch budget")
    size = sum(sum(v["bytes"] for k, v in json.loads(r["manifest_json"])["files"].items() if k.endswith(".parquet")) for r in rows)
    if size > MAX_PENDING_METADATA:
        raise IntegrityError("Publication backlog exceeds its metadata budget")
    return rows, head


class Publisher:
    def __init__(self, lib, *, index=None, pointer=None):
        self.lib = lib
        self.index = Path(index or lib.cache)
        self.pointer = pointer or read_json(self.index / "ONLINE.json")
        if self.pointer["schema_version"] != ONLINE_VERSION or self.pointer["library_id"] != lib.info["library_id"]:
            raise IntegrityError("Online publisher identity mismatch")
        self.path = contained(self.index, self.pointer["file"])

    def check_location(self):
        if (self.index / "LAKE-RELOCATION.json").exists():
            raise IntegrityError("Lake relocation is pending")
        owner = self.index / "cache_owner.json"
        if owner.exists() and not same_directory(read_json(owner).get("root"), self.lib.root):
            raise IntegrityError("Online publisher points at a previous media location")

    def validate(self, records, manifest):
        """Use rollback-only writes under the publisher lock, including a bounded pending suffix.

        This enforces the actual SQLite foreign keys and immutable-content rules before
        the archive becomes authoritative. No validation row can become visible/durable.
        """
        with FileLock(self.index / ".online.lock"):
            self.check_location()
            db = connect(self.path)
            try:
                start = int(settings(db)["served_seq"])
                pending, head = pending_commits(self.lib, start)
                db.execute("BEGIN IMMEDIATE")
                try:
                    for commit in pending:
                        m = json.loads(commit["manifest_json"])
                        publication_row(db, commit["seq"], commit["batch_id"], digest(canonical(m).encode()), commit["committed_at"])
                        apply_facts(db, load_records(self.lib.root / "segments" / commit["batch_id"], m), commit["seq"], commit["batch_id"])
                    seq = head + 1
                    publication_row(db, seq, manifest["batch_id"], digest(canonical(manifest).encode()), manifest["created_at"])
                    try:
                        apply_facts(db, records, seq, manifest["batch_id"])
                        validate_relations(db, records, self.lib.info["library_id"])
                    except (IntegrityError, apsw.ConstraintError) as error:
                        raise InvalidCanonicalResult(str(error)) from error
                finally:
                    db.execute("ROLLBACK")
            finally:
                db.close()

    def sync(self, *, chunk_rows=256, stop=None, through=None):
        if not 1 <= chunk_rows <= 4096:
            raise ValueError("Publication chunk size must be 1–4096")
        with FileLock(self.index / ".online.lock"):
            self.check_location()
            db = connect(self.path)
            published = 0
            try:
                if settings(db)["generation"] != self.pointer["generation"]:
                    raise IntegrityError("Online generation changed")
                # Read journal pages, not an unbounded list of historical commits.
                with self.lib.journal() as journal:
                    head = journal.execute("SELECT coalesce(max(seq),0) FROM commits").fetchone()[0]
                end = head if through is None else through
                if not 0 <= end <= head:
                    raise IntegrityError("Rebuild prefix is outside the archive")
                with db:
                    set_state(db, archive_seq=head)
                while int(settings(db)["served_seq"]) < end:
                    seq = int(settings(db)["served_seq"]) + 1
                    with self.lib.journal() as journal:
                        row = journal.execute("SELECT * FROM commits WHERE seq=?", (seq,)).fetchone()
                    if row is None:
                        raise IntegrityError("Archive journal prefix is discontinuous")
                    self._apply(db, dict(row), chunk_rows, stop)
                    published += 1
                return dict(archive_seq=head, served_seq=int(settings(db)["served_seq"]), published=published)
            finally:
                db.close()

    def _apply(self, db, commit, chunk_rows, stop):
        from .library import verify_manifest

        seq, batch = commit["seq"], commit["batch_id"]
        m = json.loads(commit["manifest_json"])
        directory = self.lib.root / "segments" / batch
        verify_manifest(self.lib, directory, m, hash_media=False)
        records = load_records(directory, m)
        fingerprint = digest(canonical(m).encode())
        with db:
            publication_row(db, seq, batch, fingerprint, commit["committed_at"])
            db.execute("INSERT INTO pending_publication VALUES(?,?,?) ON CONFLICT(seq) DO NOTHING", (seq, batch, fingerprint))
        if stop:
            stop("preparing", seq)
        for name in FACTS:
            rows = records.get(name, [])
            if name == "assets":
                rows = sorted(rows, key=lambda r: r["derived_from_asset_id"] is not None)
            progress_name = f"publication:{seq}:{name}"
            previous = db.execute("SELECT position,digest FROM build_progress WHERE name=?", (progress_name,)).fetchone()
            if previous and previous[1] != fingerprint:
                raise IntegrityError("Publication checkpoint fingerprint changed")
            offset = int(previous[0]) if previous else 0
            for first in range(offset, len(rows), chunk_rows):
                chunk = rows[first:first + chunk_rows]
                with db:
                    for row in chunk:
                        insert_fact(db, name, row, seq, batch)
                    last = first + len(chunk)
                    db.execute("INSERT INTO build_progress VALUES(?,?,?,?,?) ON CONFLICT(name) DO UPDATE SET position=excluded.position,rows=excluded.rows,complete=excluded.complete",
                               (progress_name, str(last), last, fingerprint, int(last == len(rows))))
                if stop:
                    stop("facts:" + name, seq)
        validate_relations(db, records, self.lib.info["library_id"])
        project(db, records, seq, stop)
        if stop:
            stop("before_watermark", seq)
        with db:
            db.execute("UPDATE publications SET state='published',published_at=?,counts_json=? WHERE seq=?",
                       (utc(), canonical(counts(db, seq)), seq))
            set_state(db, served_seq=seq)
            db.execute("DELETE FROM pending_publication WHERE seq=?", (seq,))
            db.execute("DELETE FROM build_progress WHERE name LIKE ?", (f"publication:{seq}:%",))
        if stop:
            stop("published", seq)


def rebuild(lib, output, *, through=None, stop=None):
    """Compatibility convenience around the shared build/verify maintenance entry points."""
    from .maintenance import build, verify

    plan = build(lib.root, output, through=through, stop=stop)
    verify(output)
    pointer = {key: plan[key] for key in ("schema_version", "library_id", "site", "generation", "file")}
    return dict(state="verified", output=str(Path(output).resolve()), archive_seq=plan["sequence"],
                served_seq=plan["sequence"], pointer=pointer)
