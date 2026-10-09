"""Pinterest commands, bounded reads and optimistic actions; no network in RPC handlers."""

import base64
import json
from pathlib import Path
import uuid

from . import COLLECTOR, CONTRACT_VERSION, model
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
                if marker.exists() and read_json(marker) != owner:
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
                    known_pins=len(spec["seeds"]), network_requests=0,
                    warnings=["Only static originals and supported single-image stories are downloaded; other shapes retain a gap."])

    def create(self, args):
        model.fields(args, ("request_key", "definition"))
        result = self.preview(dict(definition=args["definition"]))
        spec = result["definition"]
        args = {**args, "definition": spec}
        with locations.access(self.state, spec["library_id"]), self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            request = self._request(db, "job_create", args, str(uuid.uuid4()))
            identity = request["subject_id"]
            if request["state"] != "succeeded":
                old = db.execute("SELECT id FROM pinterest_jobs WHERE lake_id=? AND definition_sha256=? AND desired_state<>'cancelled' "
                    "AND state NOT IN ('completed','completed_with_gaps','cancelled') ORDER BY job_row LIMIT 1",
                    (spec["library_id"], result["definition_sha256"])).fetchone()
                if old:
                    identity = old[0]
                else:
                    at = utc()
                    context = dict(mode="anonymous", language=spec["access"]["language"], session_id=str(uuid.uuid4()), created_at=at)
                    db.execute("""INSERT INTO pinterest_jobs(id,request_key,lake_id,definition_json,definition_sha256,context_json,
                        state,desired_state,created_at,updated_at) VALUES(?,?,?,?,?,?,'queued','running',?,?)""",
                        (identity, args["request_key"], spec["library_id"], canonical(spec), result["definition_sha256"], canonical(context), at, at))
                db.execute("UPDATE pinterest_requests SET state='succeeded',subject_id=? WHERE request_key=?", (identity, args["request_key"]))
        return self.job(identity)

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
        spec = json.loads(row.pop("definition_json"))
        row.pop("context_json")
        row.pop("request_key")
        row.pop("job_row")
        counts = [dict(r) for r in db.execute("SELECT kind,state,n FROM pinterest_counts WHERE job_id=? AND n>0 ORDER BY kind,state", (identity,))]
        for item in counts:
            item.pop("job_id", None)
        actions = {"completed": [], "cancelled": [], "completed_with_gaps": ["retry", "cancel"],
                   "paused": ["resume", "cancel"], "pausing": ["cancel"], "cancelling": [],
                   "waiting_budget": ["pause", "cancel"], "needs_review": ["resume", "cancel"]}.get(row["state"], ["pause", "cancel"])
        if row["state"] == "needs_review" and not db.execute("SELECT 1 FROM pinterest_tasks WHERE job_id=? AND state='running' LIMIT 1", (identity,)).fetchone():
            actions = ["retry", "resume", "cancel"]
        return dict(**row, definition=spec, counts=counts, phase=row["state"], actions=actions)

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
        model.fields(args, ("job_id",) if kind == "items" else (), ("cursor", "limit", "library_id", "state"))
        limit = model.integer(args.get("limit", 50), 1, 100)
        filters = {k: v for k, v in args.items() if k not in ("limit", "cursor")}
        filters["kind"] = kind
        after = self.position(args.get("cursor"), filters)
        table, position = ("pinterest_tasks", "task_row") if kind == "items" else ("pinterest_jobs", "job_row")
        clauses, values = [position + ">?"], [after]
        for key, column in (("job_id", "job_id"), ("library_id", "lake_id"), ("state", "state")):
            if args.get(key) is not None:
                if (kind == "items" and key == "library_id") or (kind != "items" and key == "job_id"):
                    model.invalid("Unsupported Pinterest page filter")
                if key != "state":
                    model.identity(args[key])
                elif not isinstance(args[key], str) or len(args[key]) > 40:
                    model.invalid("Invalid Pinterest task state")
                clauses.append(column + "=?")
                values.append(args[key])
        with self.state.db() as db:
            rows = [dict(r) for r in db.execute("SELECT * FROM " + table + " WHERE " + " AND ".join(clauses) + " ORDER BY " + position + " LIMIT ?", (*values, limit + 1))]
            items = [self.job(r["id"], db) if kind == "jobs" else {k: r[k] for k in ("task_id", "kind", "pin_id", "state", "attempts", "reason", "updated_at")} for r in rows[:limit]]
        return dict(items=items, next_cursor=self.cursor(filters, rows[limit-1][position]) if len(rows) > limit else None)

    def action(self, args):
        model.fields(args, ("job_id", "action", "expected_revision"))
        model.integer(args["expected_revision"])
        action = args["action"]
        if action not in ("pause", "resume", "cancel", "retry"):
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
            desired = dict(pause="paused", resume="running", cancel="cancelled", retry="running")[action]
            state = dict(pause="pausing", resume="queued", cancel="cancelling", retry="queued")[action]
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
        if command == "capabilities":
            model.fields(args)
            return dict(contract_version=CONTRACT_VERSION, site="pinterest", collector=COLLECTOR, seed_kinds=["pin"],
                media_types=["static_image", "single_image_story"], access_modes=["anonymous"], archive_format=3, online_format=4,
                discovery=False, schedules=False, image_profiles=["original"], max_seeds=500)
        if command == "lakes":
            model.fields(args)
            with self.state.db() as db:
                return dict(items=[self.lake(r[0], db) for r in db.execute("SELECT lake_id FROM pinterest_lakes ORDER BY lake_id LIMIT 200")])
        if command == "job":
            model.fields(args, ("job_id",))
            return self.job(args["job_id"])
        if command in ("jobs", "items"):
            return self.page(args, command)
        handler = {"lake_create": self.create_lake, "preview": self.preview, "create": self.create, "action": self.action}.get(command)
        if handler is None:
            model.invalid("Unknown Pinterest command")
        return handler(args)
