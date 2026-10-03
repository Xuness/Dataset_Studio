"""Collection application service. Network execution stays outside control transactions."""

import base64
import copy
import json
from pathlib import Path
import uuid

from . import CONTRACT_VERSION, TERMINAL, model, requests as ledger
from .accounts import Accounts
from ..config import Config
from ..media_lake.library import MediaLibrary
from ..media_lake.schema import canonical, utc
from ..updates import locations, settings
from ..updates.sites import UpdateError
from ..util import FileLock, atomic_json, digest, read_json, safe_managed_path


def initial_counters():
    return dict(download_bytes=0, objects=0, browsable_images=0, media_downloaded=0, media_reused=0,
                archived_media=0, published_media=0, round=dict(api_requests=0, admitted_authors=0, download_bytes=0, wall_seconds=0))


class Service:
    def __init__(self, state):
        self.state, self.accounts = state, Accounts(state)

    def execution_lock(self, identity):
        model.identity(identity)
        return FileLock(safe_managed_path(self.state.root, self.state.root / "executions" / ("collection-" + identity + ".lock")), timeout=0)

    def active(self, identity):
        lock = self.execution_lock(identity)
        if not lock.path.exists():
            return False
        try:
            with lock:
                return False
        except RuntimeError:
            return True

    def lake(self, identity, db=None):
        model.identity(identity)
        if db is None:
            with self.state.db() as db:
                return self.lake(identity, db)
        row = db.execute("SELECT l.*,c.archive_format,c.online_format,c.collector FROM collection_lakes c JOIN lakes l ON l.id=c.lake_id WHERE c.lake_id=?", (identity,)).fetchone()
        if row is None:
            raise UpdateError("NOT_FOUND", "Collection lake does not exist")
        return dict(library_id=identity, site=row["site"], media_root=row["media"], index_root=row["index_root"],
                    archive_format=row["archive_format"], online_format=row["online_format"], collector=row["collector"], state="ready")

    def create_lake(self, args):
        model.fields(args, ("request_key", "site", "media_root", "index_root"))
        model.choice(args["site"], ("pixiv",))
        for key in ("media_root", "index_root"):
            if not isinstance(args[key], str) or not args[key] or len(args[key]) > 32768 or not Path(args[key]).is_absolute():
                model.invalid("Lake directories must be absolute paths")
        config = Config(Path(args["media_root"]), Path(args["index_root"]))
        args = {**args, "media_root": str(config.root), "index_root": str(config.cache)}
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            request, _ = ledger.begin(db, "lake_create", args["request_key"], args, str(uuid.uuid4()))
            identity = request["subject_id"]
            if request["state"] == "succeeded":
                return self.lake(identity, db)
        # This per-request gate prevents simultaneous idempotent initializations.
        with FileLock(self.state.root / "collection-requests" / (args["request_key"] + ".lock")):
            lib = MediaLibrary.initialize(config, library_id=identity)
            with FileLock(config.cache / ".update-registration.lock"):
                marker = config.cache / "UPDATE-CONTROLLER.json"
                if marker.exists() and read_json(marker) != dict(library_id=identity, root=str(self.state.root)):
                    raise UpdateError("COLLECTION_PROTOCOL", "Lake already belongs to another controller")
                atomic_json(marker, dict(library_id=identity, root=str(self.state.root)))
            with self.state.db() as db:
                db.execute("INSERT INTO lakes VALUES(?,'pixiv',?,?,?) ON CONFLICT(id) DO NOTHING", (identity, str(lib.root), str(lib.cache), utc()))
                db.execute("INSERT INTO collection_lakes VALUES(?,2,3,'pixiv_web_v1') ON CONFLICT(lake_id) DO NOTHING", (identity,))
                ledger.succeed(db, args["request_key"], identity)
        return self.lake(identity)

    def pipeline(self):
        with self.state.db() as db:
            row = db.execute("SELECT value FROM settings WHERE key='collection_pipeline_v1'").fetchone()
        saved = json.loads(row[0]) if row else dict(revision=0, value=copy.deepcopy(model.PIPELINE_DEFAULTS))
        return {**saved, "shared_limits": settings.read(self.state)["value"]}

    def register_lake(self, args):
        model.fields(args, ("request_key", "site", "media_root", "index_root"))
        model.choice(args["site"], ("pixiv",))
        for name in ("media_root", "index_root"):
            if not isinstance(args[name], str) or not Path(args[name]).is_absolute():
                model.invalid("Lake directories must be absolute paths")
        lib = MediaLibrary(Config(Path(args["media_root"]), Path(args["index_root"])))
        identity = lib.info["library_id"]
        with locations.access(self.state, identity), FileLock(lib.cache / ".update-registration.lock"), self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            request, _ = ledger.begin(db, "lake_register", args["request_key"], args, identity)
            if request["state"] != "succeeded":
                old = db.execute("SELECT media,index_root FROM lakes WHERE id=?", (identity,)).fetchone()
                if old and (Path(old[0]).resolve(), Path(old[1]).resolve()) != (lib.root.resolve(), lib.cache.resolve()):
                    raise UpdateError("SOURCE_LOCATION_CONFLICT", "Use coordinated lake relocation to change registered paths")
                marker = lib.cache / "UPDATE-CONTROLLER.json"
                if marker.exists():
                    owner = read_json(marker)
                    if owner.get("library_id") != identity or Path(owner.get("root", "")).resolve() != self.state.root:
                        raise UpdateError("UPDATE_CONFLICT", "Lake already belongs to another controller")
                # Existing archive task receipts need their original control records.
                # Do not claim a foreign archive and later fail midway through replay.
                with lib.journal() as journal:
                    for (job_id,) in journal.execute("SELECT job_id FROM collection_runs"):
                        if not db.execute("SELECT 1 FROM collection_jobs WHERE id=?", (job_id,)).fetchone():
                            raise UpdateError("UPDATE_CONFLICT", "Restore the original collection controller with this archive")
                atomic_json(marker, dict(library_id=identity, root=str(self.state.root)))
                db.execute("INSERT INTO lakes VALUES(?,'pixiv',?,?,?) ON CONFLICT(id) DO NOTHING", (identity, str(lib.root), str(lib.cache), utc()))
                db.execute("INSERT INTO collection_lakes VALUES(?,2,3,'pixiv_web_v1') ON CONFLICT(lake_id) DO NOTHING", (identity,))
                ledger.succeed(db, args["request_key"], identity)
        return self.lake(identity)

    def save_pipeline(self, args):
        model.fields(args, ("expected_revision", "value"))
        revision = model.integer(args["expected_revision"])
        value = model.pipeline(args["value"])
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            row = db.execute("SELECT value FROM settings WHERE key='collection_pipeline_v1'").fetchone()
            if (json.loads(row[0])["revision"] if row else 0) != revision:
                raise UpdateError("REVISION_CONFLICT", "Collection pipeline changed")
            db.execute("INSERT INTO settings VALUES('collection_pipeline_v1',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", (canonical(dict(revision=revision + 1, value=value)),))
        return self.pipeline()

    def preview(self, value):
        spec = model.definition(value)
        self.lake(spec["library_id"])
        account = self.accounts.public(spec["account_id"])
        issues = []
        if account["mode"] == "anonymous":
            issues.append(dict(code="PUBLIC_VISIBILITY_UNVERIFIED", message="公开访问按本次可见目录采集，登录显示条件单独记录为未知", severity="info"))
        elif account["state"] != "valid":
            issues.append(dict(code="CREDENTIALS_REQUIRED", message="采集前需要导入并验证 Pixiv 会话", severity="warning"))
        return dict(definition=spec, known_seed_count=len(spec["seeds"]["ids"]),
                    known_work_count=len(spec["seeds"]["ids"]) if spec["seeds"]["kind"] == "works" else None,
                    known_media_count=None, issues=issues)

    def create_job(self, args):
        model.fields(args, ("request_key", "definition"))
        spec = self.preview(args["definition"])["definition"]
        with locations.access(self.state, spec["library_id"]), self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            identity, replayed, coalesced = self.create_job_db(db, spec, args["request_key"])
        return dict(job=self.job(identity), replayed=replayed, coalesced=coalesced)

    def create_job_db(self, db, spec, request_key):
        """One transaction for manual admission, schedule occurrence and request replay."""
        request, replayed = ledger.begin(db, "job_create", request_key, dict(definition=spec), str(uuid.uuid4()))
        identity = request["subject_id"]
        coalesced = bool(json.loads(request["result_json"] or "{}").get("coalesced"))
        if request["state"] != "succeeded":
            fingerprint = digest(canonical(spec).encode())
            existing = db.execute("""SELECT id FROM collection_jobs WHERE lake_id=? AND definition_sha256=?
                AND desired_state='run' AND state NOT IN ('completed','completed_with_gaps','cancelled','needs_review') ORDER BY job_row LIMIT 1""",
                                  (spec["library_id"], fingerprint)).fetchone()
            if existing:
                identity, coalesced = existing[0], True
            else:
                account = self.accounts.row(spec["account_id"], db)
                at = utc()
                state = "queued" if account["state"] == "valid" else "waiting_credentials"
                db.execute("""INSERT INTO collection_jobs(id,request_key,lake_id,account_id,definition_json,definition_sha256,desired_state,state,
                    revision,counters_json,created_at,updated_at) VALUES(?,?,?,?,?,?,'run',?,1,?,?,?)""",
                           (identity, request_key, spec["library_id"], spec["account_id"], canonical(spec), fingerprint, state, canonical(initial_counters()), at, at))
                from .planner import entity

                for seed in spec["seeds"]["ids"]:
                    entity(db, identity, "author" if spec["seeds"]["kind"] == "authors" else "work", seed, 0)
            ledger.succeed(db, request_key, identity, dict(coalesced=coalesced))
        return identity, replayed, coalesced

    def row(self, identity, db=None):
        model.identity(identity)
        if db is None:
            with self.state.db() as db:
                return self.row(identity, db)
        row = db.execute("SELECT * FROM collection_jobs WHERE id=?", (identity,)).fetchone()
        if row is None:
            raise UpdateError("NOT_FOUND", "Collection job does not exist")
        return dict(row)

    def progress(self, row, db):
        identity = row["id"]
        counts = {(r[0], r[1]): r[2] for r in db.execute("SELECT kind,state,n FROM collection_counts WHERE job_id=?", (identity,))}
        def n(kind, states=None):
            return sum(v for (k, s), v in counts.items() if k == kind and (states is None or s in states))
        closed = {"done", "unavailable", "excluded", "needs_review", "cancelled"}
        gaps = {"unavailable", "needs_review"}
        counters = json.loads(row["counters_json"])
        staged = list(counters.get("staged_media", {}).values())
        authors = db.execute("SELECT count(*),coalesce(sum(state<>'candidate' AND state<>'excluded'),0) FROM collection_entities WHERE job_id=? AND kind='author'", (identity,)).fetchone()
        discovery_done = n("relationship_page") == n("relationship_page", closed) and not db.execute("SELECT 1 FROM collection_entities WHERE job_id=? AND state='candidate' LIMIT 1", (identity,)).fetchone()
        directories_done = n("author_directory") == n("author_directory", {"done"})
        details_done = n("work_detail") == n("work_detail", closed)
        manifests_done = n("media_manifest") == n("media_manifest", {"done"})
        visibility = json.loads(row["visibility_json"]) if row["visibility_json"] else {}
        publication = counters.get("publication", dict(archive_seq=0, served_seq=0, pending_batches=0))
        retained_media = counters.get("retained_media", 0)
        mode = json.loads(visibility.get("policy_json", "{}" )).get("login", "anonymous")
        return dict(authors=dict(discovered=authors[0], admitted=authors[1], scanned=n("author_directory", {"done"})),
                    works=dict(planned=n("work_detail") if discovery_done and directories_done else None,
                               details=n("work_detail", {"done", "excluded"}), gaps=n("work_detail", gaps), retained=counters.get("retained_works", 0), excluded=n("work_detail", {"excluded"})),
                    media=dict(planned=n("media_download") + retained_media if discovery_done and directories_done and details_done and manifests_done else None,
                               downloaded=counters["media_downloaded"] + staged.count("downloaded"), historical_reused=counters["media_reused"] + staged.count("reused"), http_validated=0,
                               archived=counters["archived_media"], published=counters["published_media"], gaps=n("media_download", gaps), retained=retained_media),
                    objects=dict(stored=counters["objects"], browsable_images=counters["browsable_images"]),
                    download_bytes=counters["download_bytes"], publication=publication, access_mode=mode,
                    directory_delta=counters.get("directory_delta", dict(added=0, no_longer_listed=0, unchanged=0)),
                    task_gaps=sum(v for (k, s), v in counts.items() if s in gaps),
                    budget=dict(limits=json.loads(row["definition_json"])["run_budget"], used=counters["round"]),
                    closure=dict(discovery_exhausted=discovery_done, directories_complete=directories_done,
                                 manifests_complete=manifests_done and details_done, visibility_verified=visibility.get("verified") is True))

    def job(self, identity):
        with self.state.db() as db:
            row = self.row(identity, db)
            progress = self.progress(row, db)
        return dict(id=identity, library_id=row["lake_id"], account_id=row["account_id"], definition=json.loads(row["definition_json"]),
                    state=row["state"], desired_state=row["desired_state"], revision=row["revision"], execution_epoch=row["execution_epoch"],
                    execution_active=self.active(identity), wait_reason=row["error_code"], progress=progress,
                    created_at=row["created_at"], updated_at=row["updated_at"])

    @staticmethod
    def cursor(filters, after):
        return base64.urlsafe_b64encode(canonical(dict(filters=filters, after=after)).encode()).decode()

    @staticmethod
    def position(value, filters, *, composite=False):
        if value is None:
            return 0
        try:
            if len(value) > 2048:
                raise ValueError()
            cursor = json.loads(base64.urlsafe_b64decode(value))
            if cursor["filters"] != filters:
                raise ValueError()
            return cursor["after"] if composite else model.integer(cursor["after"])
        except (ValueError, TypeError, KeyError, UnicodeError):
            model.invalid("Collection page cursor does not match its filters")

    def jobs(self, args):
        model.fields(args, (), ("cursor", "limit", "library_id", "state"))
        limit = model.integer(args.get("limit", 50), 1, 200)
        filters = {k: args.get(k) for k in ("library_id", "state")}
        after = self.position(args.get("cursor"), filters)
        clauses, values = ["job_row>?"], [after]
        for key, column in (("library_id", "lake_id"), ("state", "state")):
            if args.get(key):
                clauses.append(column + "=?")
                values.append(args[key])
        with self.state.db() as db:
            rows = list(db.execute("SELECT job_row,id FROM collection_jobs WHERE " + " AND ".join(clauses) + " ORDER BY job_row LIMIT ?", (*values, limit + 1)))
        return dict(items=[self.job(r[1]) for r in rows[:limit]], next_cursor=self.cursor(filters, rows[limit - 1][0]) if len(rows) > limit else None)

    def registry(self, kind, args):
        model.fields(args, (), ("cursor", "limit"))
        limit = model.integer(args.get("limit", 50), 1, 200)
        filters = dict(registry=kind)
        table, column, render = (("collection_lakes", "lake_id", self.lake) if kind == "lakes"
                                 else ("collection_accounts", "id", self.accounts.public))
        after = self.position(args.get("cursor"), filters)
        with self.state.db() as db:
            rows = list(db.execute(f"SELECT rowid,{column} FROM {table} WHERE rowid>? ORDER BY rowid LIMIT ?", (after, limit + 1)))
        return dict(items=[render(row[1]) for row in rows[:limit]],
                    next_cursor=self.cursor(filters, rows[limit - 1][0]) if len(rows) > limit else None)

    def tasks(self, identity, args):
        self.row(identity)
        model.fields(args, (), ("cursor", "limit", "kind", "state", "reason"))
        limit = model.integer(args.get("limit", 50), 1, 200)
        filters = {"job_id": identity, **{k: args.get(k) for k in ("kind", "state", "reason")}}
        clauses, values = ["job_id=?", "task_row>?"], [identity, self.position(args.get("cursor"), filters)]
        for key in ("kind", "state", "reason"):
            if args.get(key):
                if key == "state" and args[key] == "gaps":
                    clauses.append("state IN ('unavailable','needs_review')")
                    continue
                clauses.append(key + "=?")
                values.append(args[key])
        with self.state.db() as db:
            rows = [dict(r) for r in db.execute("SELECT * FROM collection_tasks WHERE " + " AND ".join(clauses) + " ORDER BY task_row LIMIT ?", (*values, limit + 1))]
        keys = ("id", "job_id", "kind", "subject_key", "state", "attempts", "reason", "retry_at_ms")
        return dict(items=[{**{k: r[k] for k in keys}, "summary": json.loads(r["summary_json"] or "{}")} for r in rows[:limit]], next_cursor=self.cursor(filters, rows[limit - 1]["task_row"]) if len(rows) > limit else None)

    def action(self, identity, args):
        model.fields(args, ("request_key", "expected_revision", "action"), ("task_ids",))
        action = model.choice(args["action"], ("pause", "resume", "retry_failed", "cancel", "replay_publication"))
        model.integer(args["expected_revision"], 1)
        selected = args.get("task_ids")
        if selected is not None and (action != "retry_failed" or not isinstance(selected, list) or not 1 <= len(selected) <= 512 or any(not isinstance(v, str) or len(v) != 64 for v in selected)):
            model.invalid("A failed-task selection requires 1–512 task identities")
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            request, replayed = ledger.begin(db, "job_action", args["request_key"], dict(id=identity, **args), identity)
            row = self.row(identity, db)
            if request["state"] != "succeeded":
                if row["revision"] != args["expected_revision"]:
                    raise UpdateError("REVISION_CONFLICT", "Collection job changed; reload before acting")
                active = self.active(identity)
                if action in {"resume", "retry_failed", "replay_publication"} and active:
                    raise UpdateError("COLLECTION_EXECUTION_ACTIVE", "The previous execution has not exited")
                state, desired = row["state"], row["desired_state"]
                counters = json.loads(row["counters_json"])
                if action == "pause" and state not in TERMINAL:
                    state, desired = ("pausing" if active else "paused"), "pause"
                elif action == "cancel" and (state not in TERMINAL or state == "completed_with_gaps"):
                    state, desired = ("cancelling" if active else "cancelled"), "cancel"
                    counters["cleanup"] = "pending"
                elif action in {"resume", "retry_failed"}:
                    if state == "cancelled":
                        raise UpdateError("COLLECTION_SCOPE_CHANGED", "Cancelled jobs cannot be resumed")
                    if state in {"completed", "completed_with_gaps"} and action == "resume":
                        raise UpdateError("COLLECTION_SCOPE_CHANGED", "Use a new snapshot to collect updated source contents")
                    if action == "retry_failed":
                        suffix, values = "", []
                        if selected:
                            suffix = " AND id IN (" + ",".join("?" for _ in selected) + ")"
                            values = selected
                        db.execute("UPDATE collection_tasks SET state='queued',attempts=0,retry_at_ms=0,reason=NULL,updated_at=? WHERE job_id=? AND state IN ('unavailable','needs_review','waiting_credentials','waiting_resources','retry_wait')" + suffix, (utc(), identity, *values))
                    else:
                        db.execute("UPDATE collection_tasks SET state='queued',retry_at_ms=0,reason=NULL WHERE job_id=? AND state IN ('waiting_credentials','waiting_resources','retry_wait')", (identity,))
                    state, desired = "queued", "run"
                    counters["round"] = initial_counters()["round"]
                elif action == "replay_publication":
                    # A publication-only command is a durable mode, not a second collector process.
                    counters["publication_only"] = True
                    counters["return_state"] = row["state"]
                    state = "publishing"
                db.execute("UPDATE collection_jobs SET state=?,desired_state=?,revision=revision+1,counters_json=?,error_code=NULL,updated_at=? WHERE id=?",
                           (state, desired, canonical(counters), utc(), identity))
                if state == "cancelled":
                    db.execute("UPDATE collection_tasks SET state='cancelled',updated_at=? WHERE job_id=? AND state NOT IN ('done','unavailable','excluded','needs_review')", (utc(), identity))
                ledger.succeed(db, args["request_key"], identity)
        return dict(job=self.job(identity), replayed=replayed)

    def coverage(self, identity):
        job = self.job(identity)
        return dict(job_id=identity, state=job["state"], scope=job["definition"]["scope"], closure=job["progress"]["closure"],
                    progress=job["progress"], statement="本次会话的已观察范围；目录结束不等于全站或历史完整覆盖")

    def dispatch(self, command, args):
        if command in {"schedules", "schedule_save", "schedule_remove"}:
            from .schedules import Schedules

            return getattr(Schedules(self), command.removeprefix("schedule_") if command != "schedules" else "list")(args)
        if command in {"workspace_lakes", "workspace_jobs", "workspace_schedules"}:
            from .workspace import Workspace

            return getattr(Workspace(self), command.removeprefix("workspace_"))(args)
        if command == "capabilities":
            return dict(collector="pixiv_web_v1", contract_version=CONTRACT_VERSION, work_types=["illustration", "manga", "ugoira"],
                        discovery_entrypoints=["bookmarks", "following", "recommendations"], archive_formats=[2], online_formats=[3],
                        limits=dict(max_seeds=1000, max_depth=4, page_size=200, max_control_bytes=2 * 1024**2), authentication_modes=["anonymous", "session"],
                        refresh_modes=["all", "missing_or_stale"], periodic_snapshots=True)
        if command == "status":
            with self.state.db() as db:
                counts = dict(db.execute("SELECT state,count(*) FROM collection_jobs GROUP BY state"))
                active = [r[0] for r in db.execute("SELECT id FROM collection_jobs WHERE state IN ('running','pausing','publishing','cancelling') ORDER BY job_row LIMIT 20")]
            return dict(protocol_version=1, collection_contract_version=CONTRACT_VERSION, counts=counts, active=[self.job(v) for v in active])
        if command == "lakes":
            return self.registry("lakes", args)
        if command == "lake_create":
            return self.create_lake(args)
        if command == "lake_register":
            return self.register_lake(args)
        if command == "accounts":
            return self.registry("accounts", args)
        if command == "account_save":
            return self.accounts.save(args)
        if command == "account_authenticate":
            return self.accounts.authenticate(args)
        if command in {"account_clear", "account_probe"}:
            args = dict(args)
            identity = args.pop("id")
            return getattr(self.accounts, command.removeprefix("account_"))(identity, args)
        if command == "pipeline_get":
            return self.pipeline()
        if command == "pipeline_set":
            return self.save_pipeline(args)
        if command == "preview":
            return self.preview(args["definition"])
        if command == "create":
            return self.create_job(args)
        if command == "jobs":
            return self.jobs(args)
        if command in {"job", "coverage"}:
            return getattr(self, command)(args["id"])
        if command in {"tasks", "action"}:
            args = dict(args)
            identity = args.pop("id")
            return getattr(self, command)(identity, args)
        raise UpdateError("INVALID_INPUT", "Unknown collection operation")
