"""Short WAL control transactions. Immutable archive checkpoints repair execution state."""

from contextlib import contextmanager, ExitStack
from datetime import datetime, timezone
from pathlib import Path
import hashlib
import json
import re
import time
import uuid

from ..config import Config
from ..library import Library
from ..util import read_json, contained, now, atomic_json, FileLock, safe_managed_path
from . import credentials
from .protocol import definition, validate_site, timestamp, number
from .sites import UpdateError
from ..sqlite_control import Connection
from . import read_model, cleanup, locations

DDL = """
CREATE TABLE IF NOT EXISTS lakes(id TEXT PRIMARY KEY,site TEXT NOT NULL,media TEXT NOT NULL,
 index_root TEXT NOT NULL,registered_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS credentials(site TEXT PRIMARY KEY,blob BLOB NOT NULL,revision INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY,lake_id TEXT NOT NULL REFERENCES lakes(id),
 request_key TEXT UNIQUE NOT NULL,definition TEXT NOT NULL,state TEXT NOT NULL,cursor TEXT NOT NULL,
 created_at TEXT NOT NULL,updated_at TEXT NOT NULL,error_code TEXT,error_message TEXT,
 retry_at REAL NOT NULL DEFAULT 0,execution INTEGER NOT NULL DEFAULT 0);
CREATE INDEX IF NOT EXISTS jobs_queue ON jobs(state,retry_at,created_at,id);
CREATE TABLE IF NOT EXISTS items(job_id TEXT NOT NULL REFERENCES jobs(id),post_id INTEGER NOT NULL,
 observation_id TEXT,record_json TEXT NOT NULL,state TEXT NOT NULL,reason TEXT,asset_id TEXT,
 attempts INTEGER NOT NULL DEFAULT 0,retry_at REAL NOT NULL DEFAULT 0,
 PRIMARY KEY(job_id,post_id));
CREATE INDEX IF NOT EXISTS items_queue ON items(job_id,state,retry_at,post_id);
CREATE TABLE IF NOT EXISTS counts(job_id TEXT NOT NULL,state TEXT NOT NULL,n INTEGER NOT NULL,
 PRIMARY KEY(job_id,state));
CREATE TRIGGER IF NOT EXISTS count_insert AFTER INSERT ON items BEGIN
 INSERT INTO counts VALUES(new.job_id,new.state,1) ON CONFLICT(job_id,state) DO UPDATE SET n=n+1; END;
CREATE TRIGGER IF NOT EXISTS count_change AFTER UPDATE OF state ON items WHEN old.state<>new.state BEGIN
 UPDATE counts SET n=n-1 WHERE job_id=old.job_id AND state=old.state;
 INSERT INTO counts VALUES(new.job_id,new.state,1) ON CONFLICT(job_id,state) DO UPDATE SET n=n+1; END;
CREATE TABLE IF NOT EXISTS applied_batches(batch_id TEXT PRIMARY KEY,job_id TEXT NOT NULL,seq INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS coverage(job_id TEXT PRIMARY KEY,lake_id TEXT NOT NULL,range_json TEXT NOT NULL,
 metadata_complete INTEGER NOT NULL,media_complete INTEGER NOT NULL,exceptions INTEGER NOT NULL,
 scope TEXT NOT NULL,checked_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS schedules(id TEXT PRIMARY KEY,definition TEXT NOT NULL,every_seconds INTEGER NOT NULL,
 next_at REAL NOT NULL,enabled INTEGER NOT NULL,revision INTEGER NOT NULL,last_job TEXT);
CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS telemetry(job_id TEXT PRIMARY KEY,json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS inputs(id TEXT PRIMARY KEY,lake_id TEXT NOT NULL REFERENCES lakes(id),
 state TEXT NOT NULL,source_version TEXT NOT NULL,provenance TEXT NOT NULL,created_at TEXT NOT NULL,
 count INTEGER NOT NULL DEFAULT 0,sha256 TEXT);
CREATE TABLE IF NOT EXISTS input_ids(input_id TEXT NOT NULL REFERENCES inputs(id),post_id INTEGER NOT NULL,
 PRIMARY KEY(input_id,post_id));
CREATE TRIGGER IF NOT EXISTS input_count AFTER INSERT ON input_ids BEGIN
 UPDATE inputs SET count=count+1 WHERE id=new.input_id; END;
"""
TERMINAL = {"completed", "completed_with_exclusions", "cancelled"}
SCHEMA_VERSION = 12


class State:
    def __init__(self, root):
        self.root = Path(root).resolve()
        self.root.mkdir(parents=True, exist_ok=True)
        # The daemon and the first RPC can both open a new database. Serialize before enabling WAL;
        # SQLite can reject competing journal-mode changes without invoking the busy timeout.
        with FileLock(self.root / "initialize.lock", timeout=10), ExitStack() as upgrade, self.db() as db:
            version = db.execute("PRAGMA user_version").fetchone()[0]
            application = db.execute("PRAGMA application_id").fetchone()[0]
            if (
                application not in {0, 0x44535550}
                or version not in range(SCHEMA_VERSION + 1)
                or (version != 0 and application != 0x44535550)
            ):
                raise UpdateError("UPDATE_PROTOCOL", "Unsupported update control database")
            if version < SCHEMA_VERSION:
                try:
                    upgrade.enter_context(FileLock(self.root / "runner.lock", timeout=0.1))
                    version = db.execute("PRAGMA user_version").fetchone()[0]
                    if version == SCHEMA_VERSION:
                        return
                    for path in (self.root / "executions").glob("*.lock"):
                        upgrade.enter_context(FileLock(path, timeout=0))
                except RuntimeError:
                    # A new worker may already own runner.lock after completing
                    # the upgrade while this RPC waited for admission.
                    if db.execute("PRAGMA user_version").fetchone()[0] == SCHEMA_VERSION:
                        return
                    raise UpdateError("UPDATE_CONFLICT", "请先停止旧版更新运行器，再由 Studio 升级控制状态") from None
            if version == 0:
                tables = {
                    r[0]
                    for r in db.execute(
                        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"
                    )
                }
                if tables and not {"lakes", "jobs", "items", "applied_batches", "schedules"}.issubset(tables):
                    raise UpdateError(
                        "UPDATE_PROTOCOL", "Unrecognized database in the update state directory"
                    )
                db.execute("BEGIN IMMEDIATE")
                db.executescript(DDL)
                db.execute("PRAGMA application_id=1146312016")
                db.execute("PRAGMA user_version=2")
            elif version == 1:
                db.execute("BEGIN IMMEDIATE")
                db.execute("CREATE TABLE IF NOT EXISTS telemetry(job_id TEXT PRIMARY KEY,json TEXT NOT NULL)")
                db.execute("PRAGMA user_version=2")
            if version < 3:
                db.executescript(read_model.DDL)
                db.execute("PRAGMA user_version=3")
            if version < 4:
                db.execute("PRAGMA user_version=4")
            if version < 5:
                db.executescript(cleanup.DDL)
                db.execute("PRAGMA user_version=5")
            if version < 6:
                db.executescript(locations.DDL)
                db.execute("PRAGMA user_version=6")
            if version < 7:
                # Development hot reload could persist v5 before lake_dispatch was
                # added to that migration. A later v6 upgrade did not revisit it.
                # Repair in a new transactionally versioned step, preserving any
                # existing service order and all task/credential/cursor records.
                db.execute("CREATE TABLE IF NOT EXISTS lake_dispatch("
                           "lake_id TEXT PRIMARY KEY REFERENCES lakes(id),sequence INTEGER NOT NULL)")
                db.execute("PRAGMA user_version=7")
            if version < 8:
                db.executescript((Path(__file__).parents[1] / "collections" / "schema.sql").read_text(encoding="utf-8"))
                db.execute("PRAGMA user_version=8")
            if version < 9:
                db.executescript((Path(__file__).parents[1] / "collections" / "continuous.sql").read_text(encoding="utf-8"))
                db.execute("PRAGMA user_version=9")
            if version < 10:
                from ..collections.migration import repair_indexes

                repair_indexes(db)
                db.execute("PRAGMA user_version=10")
            if version < 11:
                db.executescript((Path(__file__).parents[1] / "collections" / "recovery.sql").read_text(encoding="utf-8"))
                db.execute("PRAGMA user_version=11")
            if version < 12:
                # Older runners interpret unknown ranges as unfiltered ID scans.
                # Fence them out before any literal-tag task can be admitted.
                db.execute("PRAGMA user_version=12")

    @contextmanager
    def db(self):
        db = Connection(self.root / "updates.sqlite", timeout=10)
        try:
            db.execute("PRAGMA journal_mode=WAL")
            db.execute("PRAGMA synchronous=FULL")
            db.execute("PRAGMA foreign_keys=ON")
            with db:
                yield db
        finally:
            db.close()

    def register(self, target):
        if set(target) != {"library_id", "site", "media_root", "index_root"}:
            raise UpdateError("INVALID_INPUT", "Target requires library_id, site and two roots")
        media, index = Path(target["media_root"]).resolve(), Path(target["index_root"]).resolve()
        pointer, info = read_json(index / "ONLINE.json"), read_json(media / "library.json")
        if target["library_id"] == "":
            target = {**target, "library_id": pointer.get("library_id")}
        if (
            pointer.get("schema_version") != 2
            or pointer.get("site") != target["site"]
            or (
                pointer.get("library_id") != target["library_id"]
                or info.get("library_id") != target["library_id"]
            )
        ):
            raise UpdateError("SOURCE_ID_MISMATCH", "Online lake identity/site mismatch")
        if not contained(index, pointer["file"]).is_file():
            raise UpdateError("SOURCE_UNAVAILABLE", "Online database is missing")
        with self.db() as db:
            old = db.execute("SELECT * FROM lakes WHERE id=?", (target["library_id"],)).fetchone()
            if old and (old["site"], old["media"], old["index_root"]) != (target["site"], str(media), str(index)):
                raise UpdateError("SOURCE_LOCATION_CONFLICT", "Use the coordinated lake relocation command")
            if locations.pending(self, target["library_id"], db):
                raise UpdateError("UPDATE_CONFLICT", "Lake relocation is pending")
        linked = read_json(media / "online-index.json")
        if Path(linked["index_root"]).resolve() != index:
            raise UpdateError("SOURCE_ID_MISMATCH", "Publisher uses a different online root")
        with FileLock(index / ".update-registration.lock"):
            marker = index / "UPDATE-CONTROLLER.json"
            if marker.exists():
                owner = read_json(marker)
                if owner.get("library_id") != target["library_id"] or not Path(owner["root"]).samefile(
                    self.root
                ):
                    raise UpdateError("UPDATE_CONFLICT", "Lake already belongs to another update controller")
            atomic_json(marker, {"library_id": target["library_id"], "root": str(self.root)})
        with self.db() as db:
            old = db.execute("SELECT * FROM lakes WHERE id=?", (target["library_id"],)).fetchone()
            if old and (old["site"], old["media"], old["index_root"]) != (
                target["site"],
                str(media),
                str(index),
            ):
                raise UpdateError(
                    "SOURCE_LOCATION_CONFLICT", "Stop/reconcile existing tasks before changing roots"
                )
            db.execute(
                "INSERT OR IGNORE INTO lakes VALUES(?,?,?,?,?)",
                (target["library_id"], target["site"], str(media), str(index), now()),
            )
        return self.lake(target["library_id"])

    def lake(self, identity):
        with self.db() as db:
            row = db.execute("SELECT * FROM lakes WHERE id=?", (identity,)).fetchone()
        if row is None:
            raise UpdateError("NOT_FOUND", "Update lake is not registered")
        return dict(row)

    def library(self, identity):
        lake = self.lake(identity)
        lib = Library(Config(root=Path(lake["media"]), cache=Path(lake["index_root"])))
        if lib.info["library_id"] != identity:
            raise UpdateError("SOURCE_ID_MISMATCH", "Archive identity changed")
        return lib

    def set_credentials(self, site, value):
        blob = credentials.encode(site, value)
        with self.db() as db:
            db.execute(
                "INSERT INTO credentials VALUES(?,?,1) ON CONFLICT(site) DO UPDATE SET "
                "blob=excluded.blob,revision=revision+1",
                (site, blob),
            )
        return {"site": site, "credential_set": True}

    def credentials(self, site):
        with self.db() as db:
            row = db.execute("SELECT blob FROM credentials WHERE site=?", (site,)).fetchone()
        return credentials.decode(row[0]) if row else {}

    def credential_status(self):
        with self.db() as db:
            return [
                {"site": r[0], "credential_set": True, "revision": r[1]}
                for r in db.execute("SELECT site,revision FROM credentials ORDER BY site")
            ]

    def create(self, spec, request_key, *, db=None):
        spec = definition(spec)
        lake = self.lake(spec["library_id"])
        validate_site(lake["site"], spec)
        if spec["range"]["kind"] == "input":
            frozen = self.input(spec["range"]["input_id"])
            if frozen["lake_id"] != spec["library_id"] or frozen["state"] != "sealed":
                raise UpdateError("INVALID_INPUT", "Input must be sealed and belong to this lake")
        if not isinstance(request_key, str) or not 1 <= len(request_key) <= 128:
            raise UpdateError("INVALID_INPUT", "An idempotency key of 1–128 characters is required")
        text = json.dumps(spec, sort_keys=True, separators=(",", ":"))

        def insert(connection):
            old = connection.execute(
                "SELECT id,definition FROM jobs WHERE request_key=?", (request_key,)
            ).fetchone()
            if old:
                if old[1] != text:
                    raise UpdateError(
                        "IDEMPOTENCY_CONFLICT", "Idempotency key already has another definition"
                    )
                return old[0]
            identity, date = uuid.uuid4().hex, now()
            connection.execute(
                "INSERT INTO jobs(id,lake_id,request_key,definition,state,cursor,created_at,updated_at) "
                "VALUES(?,?,?,?,'queued','{}',?,?)",
                (identity, spec["library_id"], request_key, text, date, date),
            )
            return identity

        if db is not None:
            return insert(db)
        with self.db() as con:
            con.execute("BEGIN IMMEDIATE")
            identity = insert(con)
        return self.job(identity)

    def job(self, identity):
        with self.db() as db:
            row = db.execute("SELECT * FROM jobs WHERE id=?", (identity,)).fetchone()
            counts = dict(db.execute("SELECT state,n FROM counts WHERE job_id=? AND n>0", (identity,)))
            telemetry = db.execute("SELECT json FROM telemetry WHERE job_id=?", (identity,)).fetchone()
            cleanup_row = db.execute("SELECT phase,retry_at,error_code FROM job_cleanup WHERE job_id=?", (identity,)).fetchone()
        if row is None:
            raise UpdateError("NOT_FOUND", "Update job not found")
        out = dict(row)
        out["cleanup"] = dict(cleanup_row) if cleanup_row else None
        out["telemetry"] = json.loads(telemetry[0]) if telemetry else {}
        out["execution_active"] = self.execution_active(identity)
        if row["state"] == "running" and not out["execution_active"]:
            # A crashed worker's last frame is not live progress. The supervisor will reclaim the job.
            out["telemetry"].update(phase="waiting_worker", download_rate_bps=0,
                                    publish_rate_images_per_second=0, current_post_id=None,
                                    current_bytes=None, current_total_bytes=None, files=[],
                                    active_downloads=0, active_encodes=0, waiting_encode=0, metadata_active=False)
            out["telemetry"].update(publishing_images=0, waiting_staging=0,
                                    staging_reserved_bytes=None, decode_reserved_bytes=None)
        out["definition"], out["cursor"], out["counts"] = (
            json.loads(out["definition"]),
            json.loads(out["cursor"]),
            counts,
        )
        return out

    def progress(self, identity, **values):
        with self.db() as db:
            db.execute("BEGIN IMMEDIATE")
            row = db.execute("SELECT json FROM telemetry WHERE job_id=?", (identity,)).fetchone()
            current = json.loads(row[0]) if row else {}
            for key, value in values.items():
                if key.endswith("_delta"):
                    key = key[:-6]
                    current[key] = current.get(key, 0) + value
                else:
                    current[key] = value
            current["sampled_at"] = now()
            db.execute("INSERT OR REPLACE INTO telemetry VALUES(?,?)", (identity, json.dumps(current)))

    def execution_active(self, identity):
        path = self.execution_lock(identity).path
        if not path.exists():
            return False
        try:
            with FileLock(path, timeout=0):
                return False
        except RuntimeError:
            return True

    def execution_lock(self, identity):
        if not isinstance(identity, str) or not re.fullmatch(r"[a-f0-9]{32}", identity):
            raise UpdateError("INVALID_INPUT", "Invalid update job identity")
        path = safe_managed_path(self.root, self.root / "executions" / (identity + ".lock"))
        return FileLock(path, timeout=0)

    def claim(self, identity):
        """Caller owns execution and lake locks. Cancel/pause may still win this transaction."""
        from .commands import read_job

        with self.db() as db:
            db.execute("BEGIN IMMEDIATE")
            row = read_job(db, identity)
            if locations.pending(self, row["lake_id"], db):
                return None
            if row["state"] not in {"queued", "running", "waiting_retry", "waiting_space"} or row["retry_at"] > time.time():
                return None
            db.execute(
                "UPDATE jobs SET state='running',retry_at=0,error_code=NULL,error_message=NULL,updated_at=? "
                "WHERE id=? AND state=? AND execution=?", (now(), identity, row["state"], row["execution"]),
            )
            return dict(row)

    def jobs(self, after="", limit=50, lake_id=None, status=None):
        return read_model.jobs(self, after, limit, lake_id, status)

    def items(self, identity, after=0, limit=100, status=None, reason=None):
        self.job(identity)
        number(limit, 1, 500)
        clauses, values = ["job_id=?", "post_id>?"], [identity, number(after, 0)]
        if status:
            states = read_model.PROBLEMS if status == "problems" else (status,)
            if len(status) > 64:
                raise UpdateError("INVALID_INPUT", "Invalid item state")
            clauses.append("state IN (" + ",".join("?" for _ in states) + ")")
            values.extend(states)
        if reason:
            if len(reason) > 128:
                raise UpdateError("INVALID_INPUT", "Invalid item reason")
            clauses.append("reason=?")
            values.append(reason)
        with self.db() as db:
            rows = [
                dict(r)
                for r in db.execute(
                    "SELECT post_id,observation_id,state,reason,asset_id,attempts,retry_at "
                    "FROM items WHERE " + " AND ".join(clauses) + " ORDER BY post_id LIMIT ?",
                    (*values, limit + 1),
                )
            ]
        return {
            "items": rows[:limit],
            "next_cursor": rows[limit - 1]["post_id"] if len(rows) > limit else None,
        }

    def update(self, identity, **values):
        if set(values) - {"state", "cursor", "error_code", "error_message", "retry_at"}:
            raise ValueError("Invalid state column")
        if "cursor" in values:
            values["cursor"] = json.dumps(values["cursor"], separators=(",", ":"))
        values["updated_at"] = now()
        with self.db() as db:
            guard = " AND state NOT IN ('paused','cancelled')" if "state" in values and values["state"] != "cancelled" else ""
            db.execute(
                "UPDATE jobs SET " + ",".join(k + "=?" for k in values) + " WHERE id=?" + guard,
                (*values.values(), identity),
            )

    def action(self, identity, action):
        from .commands import action as apply

        return apply(self, identity, action)

    def set_schedule(
        self, spec, every_seconds=None, first_run_at=None, enabled=False, identity=None, revision=None
    ):
        spec = definition(spec)
        validate_site(self.lake(spec["library_id"])["site"], spec)
        every_seconds = number(every_seconds, 60, 366 * 86400) if every_seconds is not None else 0
        next_at = timestamp(first_run_at).timestamp()
        if not isinstance(enabled, bool):
            raise UpdateError("INVALID_INPUT", "enabled must be boolean")
        identity = identity or uuid.uuid4().hex
        with self.db() as db:
            old = db.execute("SELECT revision FROM schedules WHERE id=?", (identity,)).fetchone()
            if old and old[0] != revision:
                raise UpdateError("REVISION_CONFLICT", "Schedule changed; refresh before editing")
            new_revision = old[0] + 1 if old else 1
            db.execute(
                "INSERT INTO schedules VALUES(?,?,?,?,?,?,NULL) ON CONFLICT(id) DO UPDATE SET "
                "definition=excluded.definition,every_seconds=excluded.every_seconds,next_at=excluded.next_at,"
                "enabled=excluded.enabled,revision=excluded.revision",
                (identity, json.dumps(spec), every_seconds, next_at, int(enabled), new_revision),
            )
        return {"id": identity, "revision": new_revision}

    def tick_schedules(self, at=None):
        at = time.time() if at is None else at
        with self.db() as db:
            db.execute("BEGIN IMMEDIATE")
            for row in db.execute(
                "SELECT * FROM schedules WHERE enabled=1 AND next_at<=? ORDER BY next_at LIMIT 100", (at,)
            ).fetchall():
                occurrence = (
                    (
                        row["next_at"]
                        + int((at - row["next_at"]) // row["every_seconds"]) * row["every_seconds"]
                    )
                    if row["every_seconds"]
                    else row["next_at"]
                )
                key = hashlib.sha256(f"{row['id']}:{row['revision']}:{occurrence}".encode()).hexdigest()
                job = self.create(json.loads(row["definition"]), "schedule:" + key, db=db)
                db.execute(
                    "UPDATE schedules SET next_at=?,last_job=?,enabled=? WHERE id=?",
                    (occurrence + row["every_seconds"], job, int(bool(row["every_seconds"])), row["id"]),
                )

    def schedules(self):
        with self.db() as db:
            rows = [dict(r) for r in db.execute("SELECT * FROM schedules ORDER BY id LIMIT 1001")]
        if len(rows) > 1000:
            raise UpdateError("UPDATE_RESOURCE_LIMIT", "Schedule limit exceeded")
        for row in rows:
            row["definition"] = json.loads(row["definition"])
            row["enabled"] = bool(row["enabled"])
            row["every_seconds"] = row["every_seconds"] or None
            row["next_run_at"] = datetime.fromtimestamp(row.pop("next_at"), timezone.utc).isoformat()
        return {"items": rows}

    def input(self, identity):
        with self.db() as db:
            row = db.execute("SELECT * FROM inputs WHERE id=?", (identity,)).fetchone()
        if row is None:
            raise UpdateError("NOT_FOUND", "Update input not found")
        value = dict(row)
        value["provenance"] = json.loads(value["provenance"])
        return value

    @locations.input_access()
    def create_input(self, library_id, source_version=None, provenance=None, identity=None):
        from .archive import online
        from .runner import lease

        lib = self.library(library_id)
        if identity is not None:
            import re
            if not isinstance(identity, str) or not re.fullmatch("[a-f0-9]{32}", identity):
                raise UpdateError("INVALID_INPUT", "Invalid fixed input identity")
            with self.db() as db:
                old = db.execute("SELECT lake_id,source_version,provenance FROM inputs WHERE id=?", (identity,)).fetchone()
            if old:
                if old["lake_id"] != library_id or old["source_version"] != source_version or json.loads(old["provenance"]) != (provenance or {}):
                    raise UpdateError("IDEMPOTENCY_CONFLICT", "Fixed input identity has different provenance")
                return self.input(identity)
        identity = identity or uuid.uuid4().hex
        with online(lib) as (_, status):
            version = source_version or f"online-v2:{status['generation']}:{status['served_seq']}"
            parts = version.split(":")
            if len(parts) != 3 or parts[:2] != ["online-v2", status["generation"]] or not parts[2].isdigit():
                raise UpdateError("SOURCE_CHANGED", "Input version does not belong to this lake generation")
            seq = int(parts[2])
            if not int(status["min_seq"]) <= seq <= int(status["served_seq"]):
                raise UpdateError("SOURCE_CHANGED", "Input version is not retained")
        if len(json.dumps(provenance or {})) > 65536:
            raise UpdateError("INVALID_INPUT", "Input provenance exceeds 64 KiB")
        lease(lib, "input:" + identity, seq, int((time.time() + 1800) * 1000))
        with self.db() as db:
            db.execute(
                "INSERT INTO inputs(id,lake_id,state,source_version,provenance,created_at) VALUES(?,?,'draft',?,?,?)",
                (identity, library_id, version, json.dumps(provenance or {}), now()),
            )
        return self.input(identity)

    @locations.input_access(by_input=True)
    def append_input(self, identity, post_ids=None, object_sha256s=None):
        from .archive import online
        from .runner import lease
        import re

        frozen = self.input(identity)
        ids = post_ids or []
        objects = object_sha256s or []
        if not isinstance(ids, list) or not isinstance(objects, list) or len(ids) + len(objects) > 10000:
            raise UpdateError("INVALID_INPUT", "Input append accepts at most 10000 references")
        if len(objects) > 128 or any(
            not isinstance(x, str) or not re.fullmatch("[a-f0-9]{64}", x) for x in objects
        ):
            raise UpdateError("INVALID_INPUT", "Append at most 128 valid object hashes")
        ids = [number(v) for v in ids]
        lib = self.library(frozen["lake_id"])
        with self.db() as db, online(lib) as (source, _):
            db.execute("BEGIN IMMEDIATE")
            if db.execute("SELECT state FROM inputs WHERE id=?", (identity,)).fetchone()[0] != "draft":
                raise UpdateError("UPDATE_CONFLICT", "Sealed input is immutable")
            lease(
                lib,
                "input:" + identity,
                int(frozen["source_version"].split(":")[-1]),
                int((time.time() + 1800) * 1000),
            )
            db.executemany("INSERT OR IGNORE INTO input_ids VALUES(?,?)", ((identity, v) for v in ids))
            seq = int(frozen["source_version"].split(":")[-1])
            for sha in objects:
                cur = source.execute(
                    "SELECT DISTINCT post_id FROM assets WHERE sha256=? AND commit_seq<=? AND post_id IS NOT NULL ORDER BY post_id",
                    (sha, seq),
                )
                found = False
                for (pid,) in cur:
                    found = True
                    db.execute("INSERT OR IGNORE INTO input_ids VALUES(?,?)", (identity, pid))
                if not found:
                    raise UpdateError(
                        "UPDATE_INPUT_UNMAPPED", "An object has no source post in the frozen version"
                    )
        return self.input(identity)

    @locations.input_access(by_input=True)
    def seal_input(self, identity):
        from .runner import lease
        from .archive import io_lock
        import os

        frozen = self.input(identity)
        if frozen["state"] == "sealed":
            return frozen
        with self.db() as db:
            db.execute("UPDATE inputs SET state='sealing' WHERE id=?", (identity,))
        lib = self.library(frozen["lake_id"])
        with io_lock(self.root, lib), lib.writer_lock():
            directory = lib.root / "plans" / "updates"
            directory.mkdir(parents=True, exist_ok=True)
            path = directory / (identity + ".ids")
            temporary = directory / (identity + ".partial")
            checksum = hashlib.sha256()
            after = 0
            with temporary.open("wb") as out:
                while True:
                    with self.db() as db:
                        rows = db.execute(
                            "SELECT post_id FROM input_ids WHERE input_id=? AND post_id>? ORDER BY post_id LIMIT 4096",
                            (identity, after),
                        ).fetchall()
                    if not rows:
                        break
                    block = b"".join(str(row[0]).encode() + b"\n" for row in rows)
                    checksum.update(block)
                    out.write(block)
                    after = rows[-1][0]
                out.flush()
                os.fsync(out.fileno())
            os.replace(temporary, path)
            atomic_json(
                directory / (identity + ".json"),
                {**frozen, "state": "sealed", "sha256": checksum.hexdigest()},
            )
        with self.db() as db:
            db.execute(
                "UPDATE inputs SET state='sealed',sha256=? WHERE id=?", (checksum.hexdigest(), identity)
            )
        lease(lib, "input:" + identity)
        return self.input(identity)
