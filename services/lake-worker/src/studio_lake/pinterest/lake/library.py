"""Pinterest-owned immutable batches and receipt journal, using common file/pack machinery."""

from contextlib import closing, contextmanager
import json
from pathlib import Path
import shutil
import sqlite3
import uuid

from . import ARCHIVE_VERSION, FEATURES, ONLINE_VERSION, SCHEMA_SET, schema
from ... import __version__
from ...archive_io import accept_batch, descriptor, write_facts
from ...canonical import canonical, utc
from ...library import Library, Batch as BlobBatch, read_object
from ...sqlite_control import Connection
from ...util import FileLock, IntegrityError, atomic_json, contained, digest, failpoint, file_hash, read_json, stable_id, sync_directory, sync_file


def check_info(info):
    if (info.get("format_version") != ARCHIVE_VERSION or info.get("site") != "pinterest"
            or info.get("schema_set") != SCHEMA_SET or info.get("required_features") != list(FEATURES)
            or str(uuid.UUID(info["library_id"])) != info["library_id"]):
        raise IntegrityError("Unsupported Pinterest archive identity or features")


def verify_manifest(lib, directory, manifest, *, hash_media=True):
    check_info(manifest)
    if manifest["library_id"] != lib.info["library_id"] or directory.name != manifest.get("batch_id"):
        raise IntegrityError("Pinterest batch identity mismatch")
    allowed = {name + ".parquet" for name in schema.FACTS} | {"media.tar", "run.json", "replay.json"}
    files = manifest.get("files")
    if not isinstance(files, dict) or set(files) - allowed or "replay.json" not in files:
        raise IntegrityError("Unknown Pinterest archive files")
    for name, info in files.items():
        path = contained(directory, name)
        if not path.is_file() or path.stat().st_size != info["bytes"]:
            raise IntegrityError("Pinterest archive file missing or truncated")
        if (hash_media or name != "media.tar") and file_hash(path) != info["sha256"]:
            raise IntegrityError("Pinterest archive file hash mismatch")
    if canonical(read_json(directory / "manifest.json")) != canonical(manifest):
        raise IntegrityError("Pinterest journal and sealed manifest differ")
    replay = read_json(directory / "replay.json")
    for field in ("library_id", "batch_id", "job_id", "definition_sha256", "receipt_id"):
        if replay.get(field) != manifest.get(field):
            raise IntegrityError("Pinterest replay identity mismatch")
    if (replay.get("version") != 1 or replay.get("kind") not in ("intent", "task")
            or manifest.get("collector") != "pinterest_web_v1"
            or manifest["dedupe_key"] != stable_id("pinterest-batch-v1", manifest["library_id"], manifest["job_id"], manifest["receipt_id"])):
        raise IntegrityError("Unsupported Pinterest replay semantics")
    if replay["kind"] == "intent":
        intent = read_json(directory / "run.json")
        if digest(canonical(intent).encode()) != manifest["definition_sha256"]:
            raise IntegrityError("Pinterest frozen definition hash mismatch")
    elif (replay.get("state") not in ("done", "superseded", "needs_review", "unavailable", "waiting_retry")
          or not isinstance(replay.get("next_tasks"), list) or len(replay["next_tasks"]) > 1000):
        raise IntegrityError("Invalid Pinterest task replay")
    return replay


class PinterestLibrary(Library):
    def __init__(self, config):
        self.config, self.root, self.cache = config, config.root, config.cache
        self.info = read_json(self.root / "library.json")
        check_info(self.info)
        if not (self.root / "journal.sqlite").is_file():
            raise IntegrityError("Pinterest archive journal is missing")
        owner = read_json(self.cache / "cache_owner.json")
        if owner.get("library_id") != self.info["library_id"]:
            raise IntegrityError("Pinterest online directory belongs to another lake")
        with self.journal() as db:
            if db.execute("PRAGMA user_version").fetchone()[0] != ARCHIVE_VERSION or db.execute("PRAGMA application_id").fetchone()[0] != 0x44534A4C:
                raise IntegrityError("Unsupported Pinterest journal")

    @classmethod
    def initialize(cls, config, *, library_id):
        from .online import initialize

        if str(uuid.UUID(library_id)) != library_id:
            raise IntegrityError("Invalid Pinterest library identity")
        config.root.mkdir(parents=True, exist_ok=True)
        config.cache.mkdir(parents=True, exist_ok=True)
        with FileLock(config.root / ".writer.lock"), FileLock(config.cache / ".initialize.lock"):
            marker = config.root / "library.json"
            intent_path = config.root / "initialization.json"
            intent = dict(library_id=library_id, media_root=str(config.root), index_root=str(config.cache), format_version=ARCHIVE_VERSION)
            if marker.exists():
                info = read_json(marker)
                check_info(info)
                if info["library_id"] != library_id:
                    raise IntegrityError("Existing Pinterest lake belongs to another initialization")
            else:
                if intent_path.exists():
                    if read_json(intent_path) != intent:
                        raise IntegrityError("Another initialization owns this directory")
                elif any(p.name != ".writer.lock" for p in config.root.iterdir()) or any(p.name != ".initialize.lock" for p in config.cache.iterdir()):
                    raise IntegrityError("New lake directories must be empty")
                atomic_json(intent_path, intent)
                with closing(Connection(config.root / "journal.sqlite")) as db, db:
                    version = db.execute("PRAGMA user_version").fetchone()[0]
                    if version == 0:
                        db.executescript(schema.sql("journal"))
                    elif version != ARCHIVE_VERSION:
                        raise IntegrityError("Pinterest journal initialization was replaced")
                sync_file(config.root / "journal.sqlite")
                info = dict(format_version=ARCHIVE_VERSION, library_id=library_id, site="pinterest", schema_set=SCHEMA_SET,
                    required_features=list(FEATURES), created_at=utc(), created_by=__version__, image_format="uncompressed-pax-tar",
                    source_policy="preserve-all-fields-and-original-api-response-bodies")
                atomic_json(marker, info)
            owner = config.cache / "cache_owner.json"
            if owner.exists() and read_json(owner).get("library_id") != library_id:
                raise IntegrityError("Pinterest index directory belongs to another lake")
            atomic_json(owner, dict(library_id=library_id, root=str(config.root)))
            initialize(info, config.cache)
            atomic_json(config.root / "online-index.json", dict(schema_version=ONLINE_VERSION, library_id=library_id, index_root=str(config.cache)))
            for name in ("segments", "staging", "source_manifests"):
                (config.root / name).mkdir(exist_ok=True)
        return cls(config)

    def sync_online(self, **kwargs):
        from .online import Publisher
        return Publisher(self).sync(**kwargs)

    def lookup_object(self, sha):
        from .online import Publisher
        from ...online_storage import connect

        publisher = Publisher(self)
        db = connect(publisher.path)
        try:
            publisher._check(db)
            row = db.execute("SELECT sha256,pack_path,offset,length FROM objects WHERE sha256=?", (sha,)).fetchone()
            return dict(zip(("sha256", "pack_path", "offset", "length"), row)) if row else None
        finally:
            db.close()

    def accept_manifest(self, directory, manifest, *, fence):
        from .online import Publisher, load_records

        directory = Path(directory)
        replay = verify_manifest(self, directory, manifest)
        old = self.committed_key(manifest["dedupe_key"])
        if old:
            with self.journal() as db:
                saved = db.execute("SELECT manifest_json FROM commits WHERE seq=?", (old["seq"],)).fetchone()[0]
            if canonical(json.loads(saved)) != canonical(manifest):
                raise IntegrityError("Pinterest batch changed after acceptance")
            return old["seq"]
        fence(replay)
        Publisher(self).validate(load_records(directory, manifest), manifest)

        def write_journal(db, seq):
            if replay["kind"] == "intent":
                definition = read_json(self.root / "segments" / manifest["batch_id"] / "run.json")
                db.execute("INSERT INTO pinterest_runs VALUES(?,?,?,?,?)", (manifest["job_id"], manifest["definition_sha256"],
                    canonical(definition), manifest["batch_id"], manifest["created_at"]))
            row = db.execute("SELECT definition_sha256 FROM pinterest_runs WHERE job_id=?", (manifest["job_id"],)).fetchone()
            if not row or row[0] != manifest["definition_sha256"]:
                raise IntegrityError("Pinterest run intent differs")
            db.execute("INSERT INTO pinterest_receipts VALUES(?,?,?,?,?,?)", (manifest["receipt_id"], manifest["job_id"], seq,
                replay.get("task_id"), replay.get("claim_token"), canonical(replay)))

        return accept_batch(self, directory, manifest, write_journal, fence=lambda: fence(replay), fault_prefix="pinterest")

    def verify(self, deep=False):
        from .online import load_records

        result = dict(batches=0, objects=0, captures=0)
        with self.journal() as db:
            for row in db.execute("SELECT * FROM commits ORDER BY seq"):
                directory = self.root / "segments" / row["batch_id"]
                manifest = json.loads(row["manifest_json"])
                verify_manifest(self, directory, manifest, hash_media=deep)
                records = load_records(directory, manifest)
                result["batches"] += 1
                result["captures"] += len(records.get("captures", []))
                result["objects"] += len(records.get("objects", []))
                for obj in records.get("objects", []) if deep else []:
                    if digest(read_object(directory, obj["pack_file"], obj["offset"], obj["length"])) != obj["sha256"]:
                        raise IntegrityError("Pinterest object byte identity differs")
        return result

    def recover(self, deep=False, *, fence=None):
        results = []
        with self.writer_lock():
            for parent in (self.root / "segments", self.root / "staging"):
                for path in sorted(parent.iterdir()):
                    if not path.is_dir() or not (path / "manifest.json").exists():
                        continue
                    with self.journal() as db:
                        accepted = db.execute("SELECT 1 FROM commits WHERE batch_id=?", (path.name,)).fetchone()
                    if accepted:
                        continue
                    if fence is None:
                        results.append(dict(batch=path.name, status="requires_pinterest_owner"))
                    else:
                        seq = self.accept_manifest(path, read_json(path / "manifest.json"), fence=fence)
                        results.append(dict(batch=path.name, status="committed", seq=seq))
        if deep:
            self.verify(deep=True)
        return results

    @classmethod
    def archive(cls, root):
        return Archive(root)


class Archive:
    def __init__(self, root):
        self.root = Path(root).resolve()
        self.info = read_json(self.root / "library.json")
        check_info(self.info)

    @contextmanager
    def journal(self):
        with closing(sqlite3.connect((self.root / "journal.sqlite").as_uri() + "?mode=ro", uri=True, timeout=30)) as db:
            db.row_factory = sqlite3.Row
            db.execute("PRAGMA query_only=ON")
            if db.execute("PRAGMA user_version").fetchone()[0] != ARCHIVE_VERSION or db.execute("PRAGMA application_id").fetchone()[0] != 0x44534A4C:
                raise IntegrityError("Unsupported Pinterest archive journal")
            yield db

    verify = PinterestLibrary.verify


class Batch(BlobBatch):
    def __init__(self, lib, job, replay, records, *, definition=None):
        self.lib, self.id = lib, replay["receipt_id"]
        if str(uuid.UUID(self.id)) != self.id:
            raise IntegrityError("Pinterest receipt must be a UUID")
        self.key = stable_id("pinterest-batch-v1", lib.info["library_id"], job["id"], self.id)
        self.path = contained(lib.root, "staging/" + self.id)
        self.path.mkdir(parents=True, exist_ok=True)
        self.replay = {**replay, "version": 1, "library_id": lib.info["library_id"], "batch_id": self.id,
                       "job_id": job["id"], "definition_sha256": job["definition_sha256"]}
        self.definition, self.records = definition, records
        self.objects, self.new_hashes, self.payload_bytes = [], set(), 0
        self.tar = self.pack_writer = self.pack_staging_path = None
        self.metadata_staging_path = lib.cache / "metadata_staging" / self.id
        # Only this immutable receipt owns this temporary pack; sealed batches are recovered first.
        if (self.path / "manifest.json").exists():
            raise IntegrityError("Recover the sealed Pinterest batch before rebuilding it")
        contained(lib.cache, "pack_staging/" + self.id + ".tar.tmp").unlink(missing_ok=True)

    def add_blob(self, data, ext, *, width, height, content_type):
        before = len(self.objects)
        sha, _ = super().add_blob(data, ext, self.lib.lookup_object)
        self.new_hashes.add(sha)
        if len(self.objects) > before:
            self.objects[-1].update(pack_file="media.tar", content_type=content_type, media_category="image",
                                    stored_width=width, stored_height=height)
        return sha

    def seal(self):
        if self.objects:
            self.records["objects"] = self.objects
        schema.check_rows(self.records)
        files = {}
        if self.tar:
            self.tar.close()
            self.tar = None
            self.pack_writer.flush()
            self.pack_writer.close()
            sync_file(self.pack_staging_path)
            shutil.copyfile(self.pack_staging_path, self.path / "media.tar")
            sync_file(self.path / "media.tar")
            files["media.tar"] = descriptor(self.path / "media.tar", "objects", "pax-tar-v1", len(self.objects))
        files.update(write_facts(self.path, self.records, schema.FACTS, schema.arrow_schema, SCHEMA_SET + ":", schema.MAX_METADATA))
        if self.definition is not None:
            atomic_json(self.path / "run.json", self.definition)
            files["run.json"] = descriptor(self.path / "run.json", "intent", "pinterest-run-v1", 1)
        atomic_json(self.path / "replay.json", self.replay)
        files["replay.json"] = descriptor(self.path / "replay.json", "replay", "pinterest-replay-v1", 1)
        manifest = dict(format_version=ARCHIVE_VERSION, site="pinterest", schema_set=SCHEMA_SET, required_features=list(FEATURES),
            library_id=self.lib.info["library_id"], batch_id=self.id, receipt_id=self.id, job_id=self.replay["job_id"],
            definition_sha256=self.replay["definition_sha256"], collector="pinterest_web_v1", dedupe_key=self.key,
            created_at=utc(), writer_version=__version__, files=files)
        atomic_json(self.path / "manifest.json", manifest)
        sync_directory(self.path)
        failpoint("pinterest_after_seal")
        return manifest
