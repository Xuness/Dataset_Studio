"""Pinterest commands, bounded reads and optimistic actions; no network in RPC handlers."""

import base64
import json
from pathlib import Path
import uuid

from . import COLLECTOR, CONTRACT_VERSION, budget, model
from .lake.library import PinterestLibrary
from ..canonical import canonical, utc
from ..config import Config
from ..updates import locations
from ..updates.sites import UpdateError
from ..util import FileLock, atomic_json, digest, read_json


class Service:
    def __init__(self, state):
        self.state = state

    def execution_lock(self, identity):
        return FileLock(self.state.root / "executions" / ("pinterest-" + model.identity(identity) + ".lock"), timeout=0)

    def owns(self, owner, identity):
        if owner.get("library_id") != identity or not isinstance(owner.get("root"), str):
            return False
        try:
            # Rust canonical paths use the Windows extended prefix; compare the actual directory identity.
            return Path(owner["root"]).samefile(self.state.root)
        except OSError:
            return False

    def _request(self, db, operation, args, subject):
        model.identity(args["request_key"])
        fingerprint = digest(canonical(args).encode())
        old = db.execute("SELECT * FROM pinterest_requests WHERE request_key=?", (args["request_key"],)).fetchone()
        if old:
            if old["operation"] != operation or old["request_hash"] != fingerprint:
                raise UpdateError("PINTEREST_IDEMPOTENCY_CONFLICT", "Request key belongs to another Pinterest command")
            return dict(old)
        db.execute("INSERT INTO pinterest_requests VALUES(?,?,?,?,'pending',NULL,?)",
                   (args["request_key"], operation, fingerprint, subject, utc()))
        return dict(subject_id=subject, state="pending")

    def lake(self, identity, db=None):
        model.identity(identity)
        if db is None:
            with self.state.db() as db:
                return self.lake(identity, db)
        row = db.execute("SELECT l.* FROM lakes l JOIN pinterest_lakes p ON p.lake_id=l.id WHERE l.id=?", (identity,)).fetchone()
        if not row:
            raise UpdateError("NOT_FOUND", "Pinterest lake does not exist")
        return dict(library_id=identity, site="pinterest", media_root=row["media"], index_root=row["index_root"],
                    archive_format=3, online_format=4, collector=COLLECTOR, state="ready")

    def library(self, identity):
        lake = self.lake(identity)
        return PinterestLibrary(Config(Path(lake["media_root"]), Path(lake["index_root"])))

    def create_lake(self, args):
        model.fields(args, ("request_key", "site", "media_root", "index_root"))
        if args["site"] != "pinterest":
            model.invalid("Pinterest lake requires site=pinterest")
        for key in ("media_root", "index_root"):
            if not isinstance(args[key], str) or len(args[key]) > 32768 or not Path(args[key]).is_absolute():
                model.invalid("Lake directories must be absolute paths")
        try:
            config = Config(Path(args["media_root"]), Path(args["index_root"]))
        except ValueError as error:
            model.invalid(str(error))
        args = {**args, "media_root": str(config.root), "index_root": str(config.cache)}
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            request = self._request(db, "lake_create", args, str(uuid.uuid4()))
            identity = request["subject_id"]
            if request["state"] == "succeeded":
                return self.lake(identity, db)
        with FileLock(self.state.root / "pinterest-requests" / (args["request_key"] + ".lock")):
            PinterestLibrary.initialize(config, library_id=identity)
            with FileLock(config.cache / ".update-registration.lock"):
                marker = config.cache / "UPDATE-CONTROLLER.json"
                owner = dict(library_id=identity, root=str(self.state.root))
                if marker.exists() and not self.owns(read_json(marker), identity):
                    raise UpdateError("UPDATE_CONFLICT", "Pinterest lake belongs to another controller")
                atomic_json(marker, owner)
            with self.state.db() as db:
                db.execute("INSERT INTO lakes VALUES(?,'pinterest',?,?,?) ON CONFLICT(id) DO NOTHING", (identity, str(config.root), str(config.cache), utc()))
                db.execute("INSERT INTO pinterest_lakes VALUES(?,3,4) ON CONFLICT(lake_id) DO NOTHING", (identity,))
                db.execute("UPDATE pinterest_requests SET state='succeeded' WHERE request_key=?", (args["request_key"],))
        return self.lake(identity)

    def preview(self, args):
        model.fields(args, ("definition",))
        spec = model.definition(args["definition"])
        self.lake(spec["library_id"])
        return dict(definition=spec, definition_sha256=digest(canonical(spec).encode()),
                    known_pins=sum(s["kind"] == "pin" for s in spec["seeds"]), network_requests=0,
                    warnings=["Only static originals and supported single-image stories are downloaded; other shapes retain a gap.",
                        "Board links are resolved during the job. A scan describes this access context, not an atomic board snapshot."]
                        + (["List manifests will not be checked against detail responses."] if spec["metadata"]["detail_enrichment"] == "none" else []))

    def create(self, args):
        model.fields(args, ("request_key", "definition"))
        result = self.preview(dict(definition=args["definition"]))
        spec = result["definition"]
        args = {**args, "definition": spec}
        with locations.access(self.state, spec["library_id"]), self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            identity = self.create_job_db(db, spec, args["request_key"])
        return self.job(identity)

    def register_lake(self, args):
        model.fields(args, ("request_key", "site", "media_root", "index_root"))
        if args["site"] != "pinterest" or any(not isinstance(args[k], str) or not Path(args[k]).is_absolute() for k in ("media_root", "index_root")):
            model.invalid("Register an existing Pinterest lake using absolute directories")
        try:
            config = Config(Path(args["media_root"]), Path(args["index_root"]))
        except ValueError as error:
            model.invalid(str(error))
        lib = PinterestLibrary(config)
        identity = lib.info["library_id"]
        args = {**args, "media_root": str(config.root), "index_root": str(config.cache)}
        with locations.access(self.state, identity), FileLock(config.cache / ".update-registration.lock"), FileLock(config.cache / ".daily-run.lock", timeout=0):
            marker = config.cache / "UPDATE-CONTROLLER.json"
            owner = dict(library_id=identity, root=str(self.state.root))
            if marker.exists() and not self.owns(read_json(marker), identity):
                raise UpdateError("UPDATE_CONFLICT", "Pinterest lake belongs to another controller")
            with lib.writer_lock():
                lib.sync_online()
            with self.state.db() as db:
                db.execute("BEGIN IMMEDIATE")
                previous = db.execute("SELECT media,index_root FROM lakes WHERE id=?", (identity,)).fetchone()
                if previous and tuple(previous) != (str(config.root), str(config.cache)):
                    raise UpdateError("UPDATE_CONFLICT", "Registered lake paths differ; reconnect its managed location first")
                self._request(db, "lake_register", args, identity)
                atomic_json(marker, owner)
                db.execute("INSERT INTO lakes VALUES(?,'pinterest',?,?,?) ON CONFLICT(id) DO NOTHING", (identity, str(config.root), str(config.cache), utc()))
                db.execute("INSERT INTO pinterest_lakes VALUES(?,3,4) ON CONFLICT(lake_id) DO NOTHING", (identity,))
                db.execute("UPDATE pinterest_requests SET state='succeeded' WHERE request_key=?", (args["request_key"],))
        return self.lake(identity)

    def create_job_db(self, db, spec, request_key):
        fingerprint = digest(canonical(spec).encode())
        args = dict(request_key=request_key, definition=spec)
        request = self._request(db, "job_create", args, str(uuid.uuid4()))
        identity = request["subject_id"]
        if request["state"] != "succeeded":
            old = db.execute("SELECT id FROM pinterest_jobs WHERE lake_id=? AND definition_sha256=? AND desired_state<>'cancelled' "
                "AND state NOT IN ('completed','completed_with_gaps','cancelled') ORDER BY job_row LIMIT 1", (spec["library_id"], fingerprint)).fetchone()
            if old:
                identity = old[0]
            else:
                at = utc()
                context = dict(mode="anonymous", language=spec["access"]["language"], session_id=str(uuid.uuid4()), created_at=at)
                db.execute("""INSERT INTO pinterest_jobs(id,request_key,lake_id,definition_json,definition_sha256,context_json,
                    state,desired_state,created_at,updated_at) VALUES(?,?,?,?,?,?,'queued','running',?,?)""",
                    (identity, request_key, spec["library_id"], canonical(spec), fingerprint, canonical(context), at, at))
            db.execute("UPDATE pinterest_requests SET state='succeeded',subject_id=? WHERE request_key=?", (identity, request_key))
        return identity

    def row(self, identity, db=None):
        model.identity(identity)
        if db is None:
            with self.state.db() as db:
                return self.row(identity, db)
        row = db.execute("SELECT * FROM pinterest_jobs WHERE id=?", (identity,)).fetchone()
        if not row:
            raise UpdateError("NOT_FOUND", "Pinterest job does not exist")
        return dict(row)

    def job(self, identity, db=None):
        if db is None:
            with self.state.db() as db:
                return self.job(identity, db)
        row = self.row(identity, db)
        usage = budget.usage(db, row)
        baseline = json.loads(row.pop("budget_baseline_json"))
        spec = json.loads(row.pop("definition_json"))
        row.pop("context_json")
        row.pop("request_key")
        row.pop("job_row")
        row["library_id"] = row.pop("lake_id")
        counts = [dict(r) for r in db.execute("SELECT kind,state,n FROM pinterest_counts WHERE job_id=? AND n>0 ORDER BY kind,state", (identity,))]
        for item in counts:
            item.pop("job_id", None)
        actions = {"completed": [], "cancelled": [], "completed_with_gaps": ["retry", "cancel"],
                   "paused": ["resume", "continue", "cancel"], "pausing": ["cancel"], "cancelling": [],
                   "waiting_budget": ["continue", "pause", "cancel"], "needs_review": ["resume", "cancel"]}.get(row["state"], ["pause", "cancel"])
        if row["state"] == "needs_review" and not db.execute("SELECT 1 FROM pinterest_tasks WHERE job_id=? AND state='running' LIMIT 1", (identity,)).fetchone():
            actions = ["retry", "resume", "cancel"]
        metrics = {r[0]: r[1] for r in db.execute("SELECT name,value FROM pinterest_metrics WHERE job_id=?", (identity,))}
        media_pending = sum(r["n"] for r in counts if r["kind"] != "pin_enrichment" and r["state"] in ("queued", "running", "waiting_retry", "waiting_budget"))
        media_gaps = sum(r["n"] for r in counts if r["kind"] != "pin_enrichment" and r["state"] in ("needs_review", "unavailable"))
        return dict(**row, definition=spec, counts=counts, phase=row["state"], actions=actions,
            metrics=metrics, totals=usage, budget_usage={k: v - baseline.get(k, 0) for k, v in usage.items()},
            media_complete=media_pending == 0 and media_gaps == 0 and row["state"] in ("completed", "waiting_budget"),
            enrichment_pending=sum(r["n"] for r in counts if r["kind"] == "pin_enrichment" and r["state"] != "done"))

    @staticmethod
    def cursor(filters, position):
        return base64.urlsafe_b64encode(canonical(dict(filters=filters, position=position)).encode()).decode()

    @staticmethod
    def position(cursor, filters):
        if cursor is None:
            return 0
        try:
            if not isinstance(cursor, str) or len(cursor) > 4096:
                raise ValueError()
            value = json.loads(base64.urlsafe_b64decode(cursor))
            if value["filters"] != filters or type(value["position"]) is not int or value["position"] < 0:
                raise ValueError()
            return value["position"]
        except (ValueError, TypeError, KeyError):
            model.invalid("Pinterest page cursor does not match this query")

    def page(self, args, kind):
        model.fields(args, ("job_id",) if kind in ("items", "streams") else (), ("cursor", "limit", "library_id", "state"))
        limit = model.integer(args.get("limit", 50), 1, 100)
        filters = {k: v for k, v in args.items() if k not in ("limit", "cursor")}
        filters["kind"] = kind
        after = self.position(args.get("cursor"), filters)
        table, position = {"items": ("pinterest_tasks", "task_row"), "streams": ("pinterest_streams", "rowid"),
                           "jobs": ("pinterest_jobs", "job_row")}[kind]
        clauses, values = [position + ">?"], [after]
        for key, column in (("job_id", "job_id"), ("library_id", "lake_id"), ("state", "state")):
            if args.get(key) is not None:
                if (kind != "jobs" and key == "library_id") or (kind == "jobs" and key == "job_id"):
                    model.invalid("Unsupported Pinterest page filter")
                if key != "state":
                    model.identity(args[key])
                elif not isinstance(args[key], str) or len(args[key]) > 40:
                    model.invalid("Invalid Pinterest task state")
                clauses.append(column + "=?")
                values.append(args[key])
        with self.state.db() as db:
            rows = [dict(r) for r in db.execute("SELECT rowid,* FROM " + table + " WHERE " + " AND ".join(clauses) + " ORDER BY " + position + " LIMIT ?", (*values, limit + 1))]
            items, size = [], 0
            for row in rows[:limit]:
                if kind == "jobs":
                    value = self.job(row["id"], db)
                elif kind == "streams":
                    value = {k: row[k] for k in ("scan_id", "entrypoint", "subject_id", "depth", "state", "reason", "pages", "members", "force_detail", "samples_checked", "mismatches", "updated_at")}
                    value.update(root=json.loads(row["root_json"]), total=None, has_cursor=row["cursor_json"] not in (None, "null"))
                else:
                    value = {k: row[k] for k in ("task_id", "kind", "pin_id", "state", "attempts", "reason", "updated_at")}
                encoded = len(canonical(value).encode())
                if items and size + encoded > 1800 * 1024:
                    break
                items.append(value)
                size += encoded
        return dict(items=items, next_cursor=self.cursor(filters, rows[len(items)-1][position]) if items and len(rows)>len(items) else None)

    def action(self, args):
        model.fields(args, ("job_id", "action", "expected_revision"))
        model.integer(args["expected_revision"])
        action = args["action"]
        if action not in ("pause", "resume", "continue", "cancel", "retry"):
            model.invalid("Unknown Pinterest action")
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            row = self.row(args["job_id"], db)
            if row["revision"] != args["expected_revision"]:
                raise UpdateError("PINTEREST_CONFLICT", "Pinterest job changed; refresh before retrying this action")
            if action not in self.job(row["id"], db)["actions"]:
                raise UpdateError("PINTEREST_CONFLICT", "This action is not available for the current Pinterest state")
            if row["state"] == "cancelled" or (row["state"] in model.TERMINAL and action in ("pause", "resume")):
                raise UpdateError("PINTEREST_CONFLICT", "This action is not available for a finished job")
            desired = dict(pause="paused", resume="running", **{"continue": "running"}, cancel="cancelled", retry="running")[action]
            state = dict(pause="pausing", resume="queued", **{"continue": "queued"}, cancel="cancelling", retry="queued")[action]
            if action == "continue":
                db.execute("UPDATE pinterest_jobs SET budget_round=budget_round+1,budget_baseline_json=? WHERE id=?",
                           (canonical(budget.usage(db, row)), row["id"]))
                db.execute("UPDATE pinterest_tasks SET state='queued',reason=NULL WHERE job_id=? AND state='waiting_budget'", (row["id"],))
            if action == "retry":
                # Active claims and accepted receipts must be replayed before a new generation is possible.
                if db.execute("SELECT 1 FROM pinterest_tasks WHERE job_id=? AND state='running' LIMIT 1", (row["id"],)).fetchone():
                    raise UpdateError("PINTEREST_CONFLICT", "Pause and let the current claim settle before retrying")
                db.execute("UPDATE pinterest_tasks SET state='queued',reason=NULL,retry_at=0,attempts=0,claim_token=NULL,receipt_id=NULL,"
                    "download_generation=download_generation+1 WHERE job_id=? AND state IN ('needs_review','unavailable','waiting_retry')", (row["id"],))
            db.execute("UPDATE pinterest_jobs SET desired_state=?,state=?,revision=revision+1,retry_at=0,error_code=NULL,error_message=NULL,updated_at=? WHERE id=?",
                       (desired, state, utc(), row["id"]))
        return self.job(row["id"])

    def dispatch(self, command, args):
        if command in ("schedules", "schedule_save", "schedule_remove"):
            from .schedules import Schedules
            return getattr(Schedules(self), {"schedules": "list", "schedule_save": "save", "schedule_remove": "remove"}[command])(args)
        if command == "status":
            model.fields(args)
            with self.state.db() as db:
                counts = {r[0]: r[1] for r in db.execute("SELECT state,count(*) FROM pinterest_jobs GROUP BY state")}
                rows = list(db.execute("SELECT id FROM pinterest_jobs WHERE state NOT IN ('completed','cancelled') ORDER BY created_at DESC,id DESC LIMIT 12"))
                return dict(counts=counts, active=[self.job(r["id"], db) for r in rows])
        if command == "capabilities":
            model.fields(args)
            return dict(contract_version=CONTRACT_VERSION, site="pinterest", collector=COLLECTOR, seed_kinds=list(model.SEED_KINDS),
                media_types=["static_image", "single_image_story"], access_modes=["anonymous"], archive_format=3, online_format=4,
                discovery=True, schedules=True, image_profiles=["original"], max_seeds=500,
                entrypoints=sorted(model.ENTRYPOINTS), detail_enrichment_modes=["none", "sample", "all"], reuse_modes=["revalidate", "historical"])
        if command == "lakes":
            model.fields(args, (), ("cursor", "limit"))
            limit = model.integer(args.get("limit", 100), 1, 200)
            filters = dict(kind="lakes")
            after = self.position(args.get("cursor"), filters)
            with self.state.db() as db:
                rows = list(db.execute("SELECT rowid,lake_id FROM pinterest_lakes WHERE rowid>? ORDER BY rowid LIMIT ?", (after, limit+1)))
                return dict(items=[self.lake(r["lake_id"], db) for r in rows[:limit]],
                    next_cursor=self.cursor(filters, rows[limit-1]["rowid"]) if len(rows)>limit else None)
        if command == "job":
            model.fields(args, ("job_id",))
            return self.job(args["job_id"])
        if command in ("jobs", "items", "streams"):
            return self.page(args, command)
        handler = {"lake_create": self.create_lake, "lake_register": self.register_lake, "preview": self.preview, "create": self.create, "action": self.action}.get(command)
        if handler is None:
            model.invalid("Unknown Pinterest command")
        return handler(args)
