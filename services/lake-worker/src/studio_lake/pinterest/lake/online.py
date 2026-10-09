"""Atomic bounded publication and independent reconstruction of Pinterest facts."""

import json
from pathlib import Path
import uuid
import zlib

from . import FEATURES, ONLINE_VERSION, SCHEMA_SET
from . import schema
from ...archive_io import read_facts
from ...canonical import canonical, utc
from ...online_schema import set_state, settings
from ...online_storage import connect
from ...util import FileLock, IntegrityError, atomic_json, contained, digest, failpoint, read_json, sync_file


def initialize(info, index, *, generation=None, activate=True):
    index = Path(index)
    marker = index / "ONLINE.json"
    pointer = dict(schema_version=ONLINE_VERSION, library_id=info["library_id"], site="pinterest",
                   schema_set=SCHEMA_SET, required_features=list(FEATURES), generation=generation or str(uuid.uuid4()), file="online.sqlite")
    db = connect(index / "online.sqlite")
    try:
        if db.execute("PRAGMA user_version").fetchone()[0] == 0:
            with db:
                db.execute(schema.sql("online"))
                set_state(db, library_id=info["library_id"], site="pinterest", generation=pointer["generation"],
                    schema_version=ONLINE_VERSION, schema_set=SCHEMA_SET, required_features=canonical(list(FEATURES)),
                    served_seq=0, archive_seq=0, min_seq=0)
        else:
            state = settings(db)
            if (state.get("library_id") != info["library_id"] or state.get("site") != "pinterest"
                    or state.get("schema_set") != SCHEMA_SET or state.get("required_features") != canonical(list(FEATURES))
                    or db.execute("PRAGMA user_version").fetchone()[0] != ONLINE_VERSION):
                raise IntegrityError("Pinterest serving identity or schema differs")
            pointer["generation"] = state["generation"]
            if generation is not None and generation != state["generation"]:
                raise IntegrityError("Pinterest reconstruction generation differs")
        if marker.exists() and read_json(marker) != pointer:
            raise IntegrityError("Pinterest online pointer was replaced")
    finally:
        db.close()
    sync_file(index / "online.sqlite")
    if activate:
        atomic_json(marker, pointer)
    return pointer


def load_records(directory, manifest):
    records = read_facts(directory, manifest, schema.FACTS, schema.arrow_schema, SCHEMA_SET + ":", schema.MAX_ROWS, schema.MAX_METADATA)
    schema.check_rows(records)
    return records


def insert_fact(db, name, source, seq, batch):
    row = dict(source)
    if name == "captures":
        row["raw_zlib"] = zlib.compress(row.pop("raw_body"))
    if name == "objects":
        row["pack_path"] = "segments/" + batch + "/" + row.pop("pack_file")
        row.pop("member_name")
    keys = schema.primary_key(name)
    columns = list(row)
    previous = db.execute("SELECT " + ",".join(columns) + " FROM " + name + " WHERE " + " AND ".join(k + "=?" for k in keys),
                          tuple(row[k] for k in keys)).fetchone()
    values = tuple(row[k] for k in columns)
    if previous:
        if previous != values:
            raise IntegrityError("Immutable Pinterest fact was changed: " + name)
        return False
    row["first_seq" if name in {"objects", "pins", "source_entities"} else "commit_seq"] = seq
    db.execute("INSERT INTO " + name + "(" + ",".join(row) + ") VALUES(" + ",".join("?" for _ in row) + ")", tuple(row.values()))
    return True


def apply(db, records, manifest, seq):
    schema.check_rows(records)
    previous = db.execute("SELECT counts_json FROM publications ORDER BY seq DESC LIMIT 1").fetchone()
    counts = json.loads(previous[0]) if previous else dict(objects=0, pins=0, assets=0)
    if set(counts) != {"objects", "pins", "assets"} or any(type(v) is not int or v < 0 for v in counts.values()):
        raise IntegrityError("Pinterest publication counts are invalid")
    db.execute("INSERT INTO publications VALUES(?,?,?,'published',?,?,?)",
               (seq, manifest["batch_id"], digest(canonical(manifest).encode()), manifest["created_at"], utc(), "{}"))
    for name in schema.FACTS:
        for row in records.get(name, []):
            inserted = insert_fact(db, name, row, seq, manifest["batch_id"])
            if inserted and name in counts:
                counts[name] += 1
    for row in records.get("media_manifests", []):
        count = db.execute("SELECT count(*) FROM media_entries WHERE manifest_id=?", (row["manifest_id"],)).fetchone()[0]
        if count != row["item_count"]:
            raise IntegrityError("Pinterest manifest item count mismatch")
    for row in records.get("assets", []):
        valid = db.execute("""SELECT 1 FROM assets a JOIN media_entries m USING(media_id)
            JOIN media_manifests mm USING(manifest_id) JOIN acquisitions q USING(acquisition_id) JOIN objects o ON o.sha256=a.sha256
            WHERE a.asset_id=? AND q.download_sha256=o.sha256 AND q.download_bytes=o.length
              AND m.width=o.stored_width AND m.height=o.stored_height AND q.normalized_url=m.normalized_url
              AND q.context_id=mm.context_id AND mm.complete=1""", (row["asset_id"],)).fetchone()
        if not valid:
            raise IntegrityError("Pinterest asset does not match its frozen original media")
        db.execute("INSERT INTO changes VALUES(?,?,?) ON CONFLICT(seq,sha256) DO NOTHING", (seq, row["sha256"], '["source","media"]'))
    db.execute("UPDATE publications SET counts_json=? WHERE seq=?", (canonical(counts), seq))


class Publisher:
    def __init__(self, lib, *, index=None, pointer=None):
        self.lib = lib
        self.index = Path(index or lib.cache)
        self.pointer = pointer or read_json(self.index / "ONLINE.json")
        if (self.pointer.get("schema_version") != ONLINE_VERSION or self.pointer.get("schema_set") != SCHEMA_SET
                or self.pointer.get("required_features") != list(FEATURES) or self.pointer.get("site") != "pinterest"
                or self.pointer.get("library_id") != lib.info["library_id"]):
            raise IntegrityError("Pinterest publisher pointer mismatch")
        self.path = contained(self.index, self.pointer["file"])
        if not self.path.is_file():
            raise IntegrityError("Pinterest online index is unavailable; rebuild it before accepting new results")

    def _check(self, db):
        state = settings(db)
        if (state.get("generation") != self.pointer["generation"] or state.get("library_id") != self.pointer["library_id"]
                or state.get("schema_set") != SCHEMA_SET or state.get("site") != "pinterest"
                or state.get("required_features") != canonical(list(FEATURES))):
            raise IntegrityError("Pinterest online generation or schema changed")
        if (self.index / "LAKE-RELOCATION.json").exists():
            raise IntegrityError("Lake relocation is pending")
        return int(state["served_seq"])

    def _pending(self, after, through=None):
        with self.lib.journal() as db:
            head = db.execute("SELECT coalesce(max(seq),0) FROM commits").fetchone()[0]
            end = head if through is None else through
            if not after <= end <= head:
                raise IntegrityError("Invalid Pinterest archive prefix")
            rows = [dict(r) for r in db.execute("SELECT * FROM commits WHERE seq>? AND seq<=? ORDER BY seq LIMIT 129", (after, end))]
        return rows, head, end

    def validate(self, records, manifest):
        with FileLock(self.index / ".online.lock"):
            db = connect(self.path)
            try:
                after = self._check(db)
                pending, head, _ = self._pending(after)
                if len(pending) > 128:
                    raise IntegrityError("Publish the pending Pinterest batches before accepting more")
                db.execute("BEGIN IMMEDIATE")
                try:
                    for commit in pending:
                        m = json.loads(commit["manifest_json"])
                        apply(db, load_records(self.lib.root / "segments" / commit["batch_id"], m), m, commit["seq"])
                    apply(db, records, manifest, head + 1)
                finally:
                    db.execute("ROLLBACK")
            finally:
                db.close()

    def sync(self, *, through=None, stop=None):
        from .library import verify_manifest

        with FileLock(self.index / ".online.lock"):
            db = connect(self.path)
            published = 0
            try:
                after = self._check(db)
                while True:
                    rows, head, end = self._pending(after, through)
                    if not rows:
                        return dict(archive_seq=head, served_seq=after, published=published)
                    for commit in rows[:128]:
                        if commit["seq"] != after + 1:
                            raise IntegrityError("Pinterest journal prefix is discontinuous")
                        manifest = json.loads(commit["manifest_json"])
                        directory = self.lib.root / "segments" / commit["batch_id"]
                        verify_manifest(self.lib, directory, manifest, hash_media=False)
                        records = load_records(directory, manifest)
                        if stop:
                            stop("before_publication", commit["seq"])
                        with db:
                            apply(db, records, manifest, commit["seq"])
                            set_state(db, served_seq=commit["seq"], archive_seq=head)
                        after, published = commit["seq"], published + 1
                        failpoint("pinterest_after_publish")
                        if stop:
                            stop("published", after)
                    if after == end:
                        return dict(archive_seq=head, served_seq=after, published=published)
            finally:
                db.close()


def rebuild(archive, output, *, through=None, stop=None):
    """Build a separate index from authoritative files; never replace the active pointer implicitly."""
    output = Path(output).resolve()
    if output.exists() and any(output.iterdir()):
        raise IntegrityError("Pinterest rebuild output must be empty")
    output.mkdir(parents=True, exist_ok=True)
    initialize(archive.info, output)
    atomic_json(output / "cache_owner.json", dict(library_id=archive.info["library_id"], root=str(archive.root)))
    result = Publisher(archive, index=output).sync(through=through, stop=stop)
    db = connect(output / "online.sqlite")
    try:
        if list(db.execute("PRAGMA foreign_key_check")) or db.execute("PRAGMA quick_check").fetchone()[0] != "ok":
            raise IntegrityError("Pinterest rebuilt index failed integrity validation")
    finally:
        db.close()
    return dict(state="verified", output=str(output), **result)
