"""Immutable canonical-media batches and transactional collection checkpoints."""

from contextlib import closing, contextmanager
import json
from pathlib import Path
import shutil
import sqlite3
import uuid

import pyarrow.parquet as pq

from . import ARCHIVE_VERSION, FEATURES, ONLINE_VERSION, SCHEMA_SET
from .schema import FACTS, MAX_BATCH_METADATA_BYTES, arrow_schema, canonical, check_rows, sql, utc
from .. import __version__
from ..archive_io import accept_batch, write_facts
from ..library import Batch as LegacyBatch, Library, read_object
from ..sqlite_control import Connection
from ..util import FileLock, IntegrityError, atomic_json, contained, digest, failpoint, file_hash, read_json, stable_id, sync_directory, sync_file


def _uuid(value):
    try:
        return str(uuid.UUID(value)) == value
    except (TypeError, ValueError, AttributeError):
        return False


def verify_manifest(lib, directory, manifest, *, hash_media=True):
    if (manifest.get("format_version") != ARCHIVE_VERSION or manifest.get("schema_set") != SCHEMA_SET
            or set(manifest.get("required_features", [])) != set(FEATURES)):
        raise IntegrityError("Unsupported canonical archive features")
    if manifest.get("library_id") != lib.info["library_id"] or not _uuid(manifest.get("batch_id")) or directory.name != manifest["batch_id"]:
        raise IntegrityError("Archive batch identity mismatch")
    allowed = {name + ".parquet" for name in FACTS} | {"media.tar", "collection_intent.json", "collection_replay.json"}
    files = manifest.get("files", {})
    if not isinstance(files, dict) or set(files) - allowed or "collection_replay.json" not in files:
        raise IntegrityError("Invalid canonical archive file set")
    for name, info in files.items():
        path = contained(directory, name)
        if not path.is_file() or path.stat().st_size != info["bytes"]:
            raise IntegrityError("Archive file missing or truncated")
        if (hash_media or name != "media.tar") and file_hash(path) != info["sha256"]:
            raise IntegrityError("Archive file hash mismatch")
    if (directory / "manifest.json").exists() and canonical(read_json(directory / "manifest.json")) != canonical(manifest):
        raise IntegrityError("Archive journal and sealed manifest differ")
    source = manifest.get("source", {})
    if source.get("site") != "pixiv" or source.get("collector") != "pixiv_web_v1" or source.get("semantics_version") != "pixiv-plan-v1":
        raise IntegrityError("Unsupported archive collector semantics")


def replay_receipt(directory, manifest):
    replay = read_json(directory / "collection_replay.json")
    source = manifest["source"]
    expected = dict(version=1, library_id=manifest["library_id"], batch_id=manifest["batch_id"],
                    job_id=source["job_id"], definition_sha256=source["definition_sha256"])
    if any(replay.get(k) != v for k, v in expected.items()):
        raise IntegrityError("Collection replay identity mismatch")
    receipts = replay.get("receipt_ids", [])
    if not 1 <= len(receipts) <= 2000 or len(set(receipts)) != len(receipts) or not all(_uuid(r) for r in receipts):
        raise IntegrityError("Invalid collection receipt identities")
    streams = set()
    for point in replay.get("checkpoint_advances", []):
        if point["stream_key"] in streams or point["next_revision"] != point["expected_revision"] + 1:
            raise IntegrityError("Invalid collection checkpoint progression")
        streams.add(point["stream_key"])
    outcome_receipts, outcome_tasks = set(), set()
    for outcome in replay.get("task_outcomes", []):
        if outcome["state"] not in {"archived", "done", "unavailable", "excluded", "needs_review"} or not _uuid(outcome["claim_token"]):
            raise IntegrityError("Invalid collection task outcome")
        identity = outcome.get("receipt_id") or (receipts[0] if len(receipts) == 1 else None)
        if identity not in receipts or identity in outcome_receipts or outcome["task_id"] in outcome_tasks:
            raise IntegrityError("Task outcome does not identify one distinct batch receipt")
        outcome_receipts.add(identity)
        outcome_tasks.add(outcome["task_id"])
    intent = "collection_intent.json" in manifest["files"]
    key = (stable_id("collection-intent-v1", manifest["library_id"], source["job_id"], source["definition_sha256"])
           if intent else stable_id("collection-batch-v1", manifest["library_id"], source["job_id"], sorted(receipts)))
    if key != manifest["dedupe_key"]:
        raise IntegrityError("Batch deduplication identity mismatch")
    return replay


class MediaLibrary(Library):
    @classmethod
    def archive(cls, root):
        """Open immutable facts without an online cache, including after cache loss."""
        return Archive(root)

    def __init__(self, config):
        self.config, self.root, self.cache = config, config.root, config.cache
        self.info = read_json(self.root / "library.json")
        if self.info.get("format_version") != ARCHIVE_VERSION or self.info.get("site") != "pixiv":
            raise IntegrityError("Unsupported canonical media library")
        if not (self.root / "journal.sqlite").is_file():
            raise IntegrityError("Archive journal is missing")
        owner = read_json(self.cache / "cache_owner.json")
        if owner["library_id"] != self.info["library_id"]:
            raise IntegrityError("Online directory belongs to another lake")
        with self.journal() as db:
            if db.execute("PRAGMA user_version").fetchone()[0] != 2 or db.execute("PRAGMA application_id").fetchone()[0] != 0x44534A4C:
                raise IntegrityError("Invalid archive journal format")

    @classmethod
    def initialize(cls, config, *, library_id=None):
        library_id = library_id or str(uuid.uuid4())
        if not _uuid(library_id):
            raise IntegrityError("Library identity must be a canonical UUID")
        config.root.mkdir(parents=True, exist_ok=True)
        config.cache.mkdir(parents=True, exist_ok=True)
        with FileLock(config.root / ".writer.lock"), FileLock(config.cache / ".initialize.lock"):
            marker = config.root / "library.json"
            intent_path = config.root / "initialization.json"
            intent = dict(library_id=library_id, media_root=str(config.root), index_root=str(config.cache), format_version=2)
            if marker.exists():
                info = read_json(marker)
                if info["library_id"] != library_id or info["format_version"] != ARCHIVE_VERSION:
                    raise IntegrityError("Existing lake does not match this initialization")
            else:
                if intent_path.exists():
                    if read_json(intent_path) != intent:
                        raise IntegrityError("Another initialization owns this directory")
                elif any(p.name != ".writer.lock" for p in config.root.iterdir()) or any(p.name != ".initialize.lock" for p in config.cache.iterdir()):
                    raise IntegrityError("New lake directories must be empty")
                atomic_json(intent_path, intent)
                path = config.root / "journal.sqlite"
                with closing(Connection(path)) as db, db:
                    version = db.execute("PRAGMA user_version").fetchone()[0]
                    if version == 0:
                        db.executescript(sql("journal"))
                    elif version != 2:
                        raise IntegrityError("Journal initialization was replaced")
                sync_file(path)
                info = dict(format_version=ARCHIVE_VERSION, library_id=library_id, site="pixiv", created_at=utc(),
                            created_by=__version__, image_format="uncompressed-pax-tar", schema_set=SCHEMA_SET,
                            required_features=list(FEATURES), source_policy="preserve-all-fields-and-original-api-response-bodies")
                atomic_json(marker, info)
            owner = config.cache / "cache_owner.json"
            if owner.exists() and read_json(owner)["library_id"] != library_id:
                raise IntegrityError("Online directory belongs to another lake")
            atomic_json(owner, dict(library_id=library_id, root=str(config.root)))
            from .online import initialize

            initialize(info, config.cache)
            atomic_json(config.root / "online-index.json", dict(schema_version=ONLINE_VERSION, library_id=library_id, index_root=str(config.cache)))
            for name in ("segments", "staging", "source_manifests"):
                (config.root / name).mkdir(exist_ok=True)
        return cls(config)

    def sync_online(self, **kwargs):
        from .online import Publisher

        return Publisher(self).sync(**kwargs)

    def lookup_object(self, sha):
        from ..online_storage import connect
        from ..online_schema import settings
        from .records import one
        from .online import pending_commits

        pointer = read_json(self.cache / "ONLINE.json")
        db = connect(contained(self.cache, pointer["file"]))
        try:
            row = one(db, "SELECT * FROM objects WHERE sha256=?", (sha,))
            after = int(settings(db)["served_seq"])
        finally:
            db.close()
        if row:
            return row
        for commit in pending_commits(self, after)[0]:
            path = self.root / "segments" / commit["batch_id"] / "objects.parquet"
            if path.exists():
                for block in pq.ParquetFile(path).iter_batches(batch_size=256):
                    for row in block.to_pylist():
                        if row["sha256"] == sha:
                            return {**row, "pack_path": "segments/" + commit["batch_id"] + "/" + row["pack_file"]}
        return None

    def accept_manifest(self, directory, manifest, verify=True, *, fence=None):
        """Caller owns writer_lock; acceptance validates against the available serving store.

        Publication acknowledgement is separate. An unavailable serving store
        still prevents new acceptance because it supplies cross-batch constraints.
        """
        directory = Path(directory)
        verify_manifest(self, directory, manifest)
        replay = replay_receipt(directory, manifest)
        manifest_text = canonical(manifest)
        with self.journal() as db:
            old = db.execute("SELECT * FROM commits WHERE dedupe_key=?", (manifest["dedupe_key"],)).fetchone()
        if old:
            if canonical(json.loads(old["manifest_json"])) != manifest_text:
                raise IntegrityError("Idempotent batch has different immutable content")
            return old["seq"]
        if fence:
            fence(replay)
        from .online import Publisher, load_records

        records = load_records(directory, manifest)
        Publisher(self).validate(records, manifest)
        source = manifest["source"]
        is_intent = "collection_intent.json" in manifest["files"]
        if is_intent:
            intent = read_json(directory / "collection_intent.json")
            if (source["intent_batch_id"] != manifest["batch_id"] or replay["task_outcomes"] or replay["checkpoint_advances"]
                    or digest(canonical(intent["definition"]).encode()) != source["definition_sha256"]
                    or intent.get("planner_version") != "pixiv-plan-v1" or intent.get("job_id") != source["job_id"]):
                raise IntegrityError("Invalid archived collection intent")
        def journal_writer(db, seq):
            if is_intent:
                db.execute("INSERT INTO collection_runs VALUES(?,?,?,?,?)",
                           (source["job_id"], source["definition_sha256"], manifest["batch_id"], "pixiv-plan-v1", manifest["created_at"]))
            run = db.execute("SELECT * FROM collection_runs WHERE job_id=?", (source["job_id"],)).fetchone()
            if not run or run["definition_sha256"] != source["definition_sha256"] or run["intent_batch_id"] != source["intent_batch_id"]:
                raise IntegrityError("Collection run intent mismatch")
            db.execute("INSERT INTO collection_run_batches VALUES(?,?,?,?)", (source["job_id"], seq, manifest["batch_id"], digest(canonical(replay).encode())))
            for point in replay["checkpoint_advances"]:
                old = db.execute("SELECT revision FROM collection_checkpoints WHERE job_id=? AND stream_key=?", (source["job_id"], point["stream_key"])).fetchone()
                if (old[0] if old else 0) != point["expected_revision"]:
                    raise IntegrityError("Collection checkpoint compare-and-swap failed")
                db.execute("INSERT INTO collection_checkpoints VALUES(?,?,?,?,?,?) ON CONFLICT(job_id,stream_key) DO UPDATE SET revision=excluded.revision,cursor_json=excluded.cursor_json,seq=excluded.seq,batch_id=excluded.batch_id",
                           (source["job_id"], point["stream_key"], point["next_revision"], canonical(dict(cursor=point["next_cursor"], exhausted=point["exhausted"], capture_id=point["capture_id"])), seq, manifest["batch_id"]))
        return accept_batch(self, directory, manifest, journal_writer,
                            fence=(lambda: fence(replay)) if fence else None, fault_prefix="collection", fault=failpoint)

    def recover(self, deep=False, *, fence=None):
        output = []
        with self.writer_lock():
            # Only unaccepted candidates require validation. Journal lookups are indexed.
            for parent in (self.root / "segments", self.root / "staging"):
                for path in sorted(parent.iterdir()):
                    if not path.is_dir() or not (path / "manifest.json").exists():
                        continue
                    with self.journal() as db:
                        old = db.execute("SELECT seq FROM commits WHERE batch_id=?", (path.name,)).fetchone()
                    if old:
                        continue
                    if fence is None:
                        # A control owner must authorize first acceptance of collection results.
                        output.append(dict(batch=path.name, status="requires_collection_owner"))
                    else:
                        output.append(dict(batch=path.name, status="committed", seq=self.accept_manifest(path, read_json(path / "manifest.json"), fence=fence)))
        if deep:
            self.verify(deep=True)
        return output

    def verify(self, deep=False):
        from .online import load_records

        result = dict(batches=0, objects=0, bytes=0, captures=0)
        with self.journal() as db:
            for commit in db.execute("SELECT * FROM commits ORDER BY seq"):
                manifest = json.loads(commit["manifest_json"])
                directory = self.root / "segments" / commit["batch_id"]
                verify_manifest(self, directory, manifest, hash_media=deep)
                records = load_records(directory, manifest)
                result["batches"] += 1
                result["captures"] += len(records.get("captures", []))
                for row in records.get("objects", []):
                    result["objects"] += 1
                    result["bytes"] += row["length"]
                    if deep and digest(read_object(directory, row["pack_file"], row["offset"], row["length"])) != row["sha256"]:
                        raise IntegrityError("Archive object offset/hash mismatch")
        return result


class Archive:
    """Read-only reconstruction input; it cannot accept batches or mutate pointers."""

    def __init__(self, root):
        self.root = Path(root).resolve()
        self.info = read_json(self.root / "library.json")
        if (self.info.get("format_version") != ARCHIVE_VERSION or self.info.get("site") != "pixiv"
                or self.info.get("schema_set") != SCHEMA_SET or not _uuid(self.info.get("library_id"))):
            raise IntegrityError("Unsupported canonical media archive")
        with self.journal() as db:
            if db.execute("PRAGMA user_version").fetchone()[0] != 2 or db.execute("PRAGMA application_id").fetchone()[0] != 0x44534A4C:
                raise IntegrityError("Invalid archive journal format")

    @contextmanager
    def journal(self):
        path = (self.root / "journal.sqlite").as_uri() + "?mode=ro"
        with closing(sqlite3.connect(path, uri=True, timeout=30)) as db:
            db.row_factory = sqlite3.Row
            db.execute("PRAGMA query_only=ON")
            yield db

    verify = MediaLibrary.verify


class MediaBatch(LegacyBatch):
    def __init__(self, lib, job_id, definition_sha256, receipt_ids, *, intent_batch_id=None, definition=None, batch_id=None, resume=False):
        self.lib = lib
        self.id = batch_id or str(uuid.uuid4())
        if not _uuid(self.id) or not _uuid(job_id):
            raise IntegrityError("Batch and job identities must be UUIDs")
        self.key = (stable_id("collection-intent-v1", lib.info["library_id"], job_id, definition_sha256)
                    if definition is not None else stable_id("collection-batch-v1", lib.info["library_id"], job_id, sorted(receipt_ids)))
        if lib.committed_key(self.key):
            raise IntegrityError("This batch has already been committed")
        self.path = contained(lib.root, "staging/" + self.id)
        if self.path.exists():
            previous = read_json(self.path / "intent.json") if (self.path / "intent.json").exists() else None
            if not resume or previous is None or previous.get("library_id") != lib.info["library_id"] or previous.get("dedupe_key") != self.key or (self.path / "manifest.json").exists():
                raise IntegrityError("Cannot restart an unrelated or already sealed batch")
            # Only this batch's unsealed pack can be reconstructed from its receipt.
            contained(lib.cache, "pack_staging/" + self.id + ".tar.tmp").unlink(missing_ok=True)
        else:
            self.path.mkdir(parents=True, exist_ok=False)
        self.source = dict(site="pixiv", collector="pixiv_web_v1", job_id=job_id, semantics_version="pixiv-plan-v1",
                           definition_sha256=definition_sha256, intent_batch_id=self.id if definition is not None else intent_batch_id)
        self.replay = dict(version=1, library_id=lib.info["library_id"], job_id=job_id, batch_id=self.id,
                           definition_sha256=definition_sha256, receipt_ids=list(receipt_ids), task_outcomes=[],
                           checkpoint_advances=[], discovery_snapshot_ids=[])
        self.definition, self.records = definition, {}
        self.objects, self.new_hashes = [], set()
        self.payload_bytes = 0
        self.tar = self.pack_writer = self.pack_staging_path = None
        self.metadata_staging_path = contained(lib.cache / "metadata_staging", self.id)
        atomic_json(self.path / "intent.json", dict(library_id=lib.info["library_id"], batch_id=self.id, dedupe_key=self.key, source=self.source))

    def add(self, name, rows):
        if name not in FACTS or name == "objects":
            raise IntegrityError("Use add_blob for physical objects")
        self.records.setdefault(name, []).extend(rows)

    def add_blob(self, data, ext, *, content_type, media_category="image", width=None, height=None):
        before = len(self.objects)
        sha, ext = super().add_blob(data, ext, self.lib.lookup_object)
        # An existing blob has now been verified too. Repeated occurrences in
        # this physical batch need no additional lookup or random archive read.
        self.new_hashes.add(sha)
        if len(self.objects) > before:
            self.objects[-1].update(pack_file="media.tar", content_type=content_type, media_category=media_category,
                                    stored_width=width, stored_height=height)
        return sha

    def seal(self):
        if self.objects:
            self.records["objects"] = self.objects
        check_rows(self.records)
        files = {}

        def descriptor(path, role, schema, rows):
            return dict(role=role, schema=schema, rows=rows, bytes=path.stat().st_size, sha256=file_hash(path))

        if self.tar:
            self.tar.close()
            self.tar = None
            self.pack_writer.flush()
            self.pack_writer.close()
            sync_file(self.pack_staging_path)
            shutil.copyfile(self.pack_staging_path, self.path / "media.tar")
            sync_file(self.path / "media.tar")
            files["media.tar"] = descriptor(self.path / "media.tar", "objects", "pax-tar-v1", len(self.objects))
        files.update(write_facts(self.path, self.records, FACTS, arrow_schema, "canonical-media-v2:", MAX_BATCH_METADATA_BYTES))
        if self.definition is not None:
            path = self.path / "collection_intent.json"
            atomic_json(path, dict(version=1, job_id=self.source["job_id"], definition=self.definition, planner_version="pixiv-plan-v1"))
            files[path.name] = descriptor(path, "intent", "collection-intent-v1", 1)
        path = self.path / "collection_replay.json"
        atomic_json(path, self.replay)
        files[path.name] = descriptor(path, "replay", "collection-replay-v1", 1)
        manifest = dict(format_version=ARCHIVE_VERSION, schema_set=SCHEMA_SET, library_id=self.lib.info["library_id"],
                        batch_id=self.id, dedupe_key=self.key, created_at=utc(), writer_version=__version__,
                        required_features=list(FEATURES), source=self.source, files=files)
        atomic_json(self.path / "manifest.json", manifest)
        sync_directory(self.path)
        failpoint("collection_after_seal")
        return manifest

    def commit(self, *, fence=None):
        return self.lib.accept_manifest(self.path, self.seal(), fence=fence)
