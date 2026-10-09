"""Bounded Pinterest execution slices with receipt-first recovery and fenced acceptance."""

import json
import re
import threading
import time
import uuid

from . import budget, downloader, model, normalize, planner, receipts
from .http import Client
from .lake.library import Batch
from .lake.schema import arrow_schema
from .service import Service
from ..canonical import utc
from ..updates import locations
from ..updates.resources import Resources
from ..updates.sites import UpdateError
from ..util import FileLock, IntegrityError, contained, failpoint, read_json, stable_id


class Runner:
    def __init__(self, state, *, resources=None, stop=None, client_factory=None, image_http=None):
        self.state, self.service = state, Service(state)
        self.resources, self.stop = resources or Resources(), stop or threading.Event()
        self.resources.configure_source("pinterest", model.SITE_LIMITS)
        self.client_factory, self.image_http = client_factory or Client, image_http

    def update_state(self, identity, state, reason=None, *, retry=0):
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            desired = self.service.row(identity, db)["desired_state"]
            if desired == "paused":
                state = "paused"
            elif desired == "cancelled" and state != "cancelled":
                state = "cancelling"
            db.execute("UPDATE pinterest_jobs SET state=?,error_code=?,error_message=?,retry_at=?,updated_at=? WHERE id=?",
                       (state, reason, reason, time.time() + retry if retry else 0, utc(), identity))

    def run(self, identity, *, time_slice=15):
        job = self.service.row(identity)
        started = time.monotonic()
        try:
            with locations.access(self.state, job["lake_id"]), self.service.execution_lock(identity):
                lib = self.service.library(job["lake_id"])
                with FileLock(lib.cache / ".daily-run.lock", timeout=0):
                    self._run(lib, identity, started + time_slice, started)
        except UpdateError as error:
            current = self.service.row(identity)
            if error.code == "CANCELLED":
                target = {"running": "queued", "paused": "paused", "cancelled": "cancelling"}[current["desired_state"]]
                self.update_state(identity, target)
            elif error.code in ("UPDATE_SPACE", "UPDATE_RESOURCE_LIMIT"):
                self.update_state(identity, "waiting_resources", error.code, retry=30)
            elif error.code == "PINTEREST_NETWORK":
                with self.state.db() as db:
                    db.execute("UPDATE pinterest_tasks SET state=CASE WHEN attempts>=3 THEN 'unavailable' ELSE 'waiting_retry' END,"
                        "reason=?,retry_at=? WHERE job_id=? AND state='running'", (error.code, time.time() + error.retry_after, identity))
                self.update_state(identity, "waiting_retry", error.code, retry=error.retry_after)
            else:
                self.update_state(identity, "needs_review", error.code)
        except RuntimeError as error:
            if str(error).startswith("另一个进程正在使用此工作区"):
                return self.service.job(identity)
            raise
        except (IntegrityError, OSError):
            self.update_state(identity, "needs_review", "PINTEREST_STORAGE_CHECK_REQUIRED")
            raise
        finally:
            with self.state.db() as db:
                db.execute("UPDATE pinterest_jobs SET elapsed_seconds=elapsed_seconds+? WHERE id=?", (time.monotonic() - started, identity))
        return self.service.job(identity)

    def _cancelled(self, identity, deadline):
        return self.stop.is_set() or time.monotonic() >= deadline or self.service.row(identity)["desired_state"] != "running"

    def _fence(self, job, replay):
        current = self.service.row(job["id"])
        if current["definition_sha256"] != job["definition_sha256"]:
            raise IntegrityError("Pinterest frozen definition changed")
        if current["desired_state"] != "running":
            raise UpdateError("CANCELLED", "Pinterest admission is closed")
        if replay["kind"] == "task":
            with self.state.db() as db:
                task = db.execute("SELECT state,claim_token,receipt_id FROM pinterest_tasks WHERE task_id=? AND job_id=?",
                                  (replay["task_id"], job["id"])).fetchone()
            if not task or tuple(task) != ("running", replay["claim_token"], replay["receipt_id"]):
                raise IntegrityError("Stale Pinterest execution claim")

    def _publish(self, lib, job, replay, records, media=None, definition=None):
        identity = replay["receipt_id"]
        sealed = next((p for p in (lib.root / "segments" / identity, lib.root / "staging" / identity)
                       if (p / "manifest.json").exists()), None)
        with lib.writer_lock():
            if sealed:
                lib.accept_manifest(sealed, read_json(sealed / "manifest.json"), fence=lambda r: self._fence(job, r))
            else:
                batch = Batch(lib, job, replay, records, definition=definition)
                if media:
                    key = media["download_key"]
                    if not re.fullmatch("[0-9a-f]{64}", key):
                        raise IntegrityError("Invalid Pinterest download key")
                    path = contained(lib.cache, "pinterest_downloads/" + job["id"] + "/" + key + ".downloaded")
                    if path.stat().st_size > self.resources.max_download_bytes:
                        raise IntegrityError("Pinterest staged image exceeds current resource limits")
                    with self.resources.encoding(max(1, path.stat().st_size * 2), lambda: self.stop.is_set()):
                        sha = batch.add_blob(path.read_bytes(), media["ext"], width=media["width"], height=media["height"], content_type=media["content_type"])
                    if sha != media["download_sha256"]:
                        raise IntegrityError("Pinterest staged bytes changed after preparation")
                lib.accept_manifest(batch.path, batch.seal(), fence=lambda r: self._fence(job, r))
            while receipts.replay(self.state, lib, job["id"]) == 128:
                pass
            published = lib.sync_online()
        with self.state.db() as db:
            db.execute("UPDATE pinterest_jobs SET served_seq=?,updated_at=? WHERE id=?", (published["served_seq"], utc(), job["id"]))
        failpoint("pinterest_after_control_replay")
        # The journal and published index now own this evidence. Keep SSD staging bounded during long jobs.
        root = receipts.directory(lib, job["id"])
        for suffix in ("response.json", "prepared.json"):
            (root / (identity + "." + suffix)).unlink(missing_ok=True)
        contained(lib.cache, "pack_staging/" + identity + ".tar.tmp").unlink(missing_ok=True)
        if replay.get("download_key"):
            key = replay["download_key"]
            if not re.fullmatch("[0-9a-f]{64}", key):
                raise IntegrityError("Invalid accepted Pinterest download key")
            for suffix in ("downloaded", "download.json", "partial", "partial.json"):
                contained(lib.cache, "pinterest_downloads/" + job["id"] + "/" + key + "." + suffix).unlink(missing_ok=True)

    def _intent(self, lib, job):
        identity = str(uuid.uuid5(uuid.UUID(job["id"]), "pinterest-intent-v1"))
        spec = json.loads(job["definition_json"])
        replay = dict(kind="intent", receipt_id=identity, next_tasks=[])
        replay["next_tasks"].extend(planner.intent(spec, job["id"], replay))
        with lib.journal() as db:
            exists = db.execute("SELECT 1 FROM pinterest_runs WHERE job_id=?", (job["id"],)).fetchone()
        if not exists:
            self._publish(lib, job, replay, {}, definition=spec)

    def _claim(self, identity, spec):
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            row = db.execute("SELECT * FROM pinterest_tasks WHERE job_id=? AND (state IN ('running','queued') OR "
                "(state='waiting_retry' AND retry_at<=?)) ORDER BY CASE state WHEN 'running' THEN 0 ELSE 1 END,"
                "CASE kind WHEN 'media_download' THEN 1 WHEN 'pin_detail' THEN 2 "
                "WHEN 'pin_enrichment' THEN CASE json_extract(input_json,'$.purpose') WHEN 'sample' THEN 0 ELSE 7 END "
                "WHEN 'pin_admit' THEN 3 WHEN 'board_admit' THEN 4 WHEN 'board_resolve' THEN 4 WHEN 'section_resolve' THEN 4 ELSE 5 END,"
                "coalesce((SELECT last_turn FROM pinterest_streams s WHERE s.scan_id=json_extract(input_json,'$.scan_id')),0),task_row LIMIT 1", (identity, time.time())).fetchone()
            if not row:
                return None
            task = dict(row)
            if task["state"] != "running":
                blocked = budget.reason(db, self.service.row(identity, db), task, spec)
                if blocked:
                    db.execute("UPDATE pinterest_tasks SET state='waiting_budget',reason=?,updated_at=? WHERE task_id=?", (blocked, utc(), task["task_id"]))
                    return {"blocked": blocked}
                task.update(state="running", claim_token=str(uuid.uuid4()), receipt_id=str(uuid.uuid4()), attempts=task["attempts"] + 1)
                db.execute("UPDATE pinterest_tasks SET state='running',claim_token=?,receipt_id=?,attempts=?,updated_at=? WHERE task_id=?",
                           (task["claim_token"], task["receipt_id"], task["attempts"], utc(), task["task_id"]))
            return task

    def _prepare(self, lib, job, task, client, cancelled):
        root = receipts.directory(lib, job["id"])
        old = receipts.load_prepared(root, task["receipt_id"])
        if old:
            return old
        replay = dict(kind="task", receipt_id=task["receipt_id"], task_id=task["task_id"], claim_token=task["claim_token"],
                      state="done", reason=None, retry_at=0, next_tasks=[])
        entry = json.loads(task["input_json"])
        media = None
        if task["kind"] == "board_admit":
            records = {}
            planner.admission(self.state, job, entry, replay)
        elif task["kind"] == "pin_admit":
            records = {}
            planner.admit_pin(self.state, job, entry, replay)
        elif task["kind"] in model.NETWORK_KINDS:
            response = receipts.load_response(root, task["receipt_id"])
            if response is None:
                with self.state.db() as db:
                    budget.request(db, job["id"], task["kind"])
                if task["kind"] == "pin_detail":
                    response = client.pin(task["pin_id"])
                elif task["kind"] == "pin_enrichment":
                    response = client.pin(task["pin_id"], expanded=True)
                else:
                    response = client.request(task["kind"], entry)
                receipts.save_response(root, task["receipt_id"], response)
                failpoint("pinterest_after_response")
            if task["kind"] in ("pin_detail", "pin_enrichment"):
                records, parsed = normalize.response_facts(response, task["receipt_id"], task["pin_id"])
                replay.update(state="done" if parsed["state"] == "ready" else "needs_review", reason=parsed["reason"])
                if task["kind"] == "pin_detail":
                    replay["admissions"] = [dict(kind="pin", source_id=task["pin_id"])]
                    planner.downloads(replay, parsed, entry.get("scan_id"), confirmed_detail=True)
                    planner.detail_expansion(self.state, job, entry, records, replay)
                    if json.loads(job["definition_json"])["metadata"]["detail_enrichment"] == "all" and parsed["state"] == "ready":
                        replay["next_tasks"].append(planner.task("pin_enrichment", task["pin_id"], pin_id=task["pin_id"], purpose="all"))
                elif records.get("pin_observations"):
                    planner.comparison(replay, entry, parsed)
            elif task["kind"] in model.PAGE_KINDS:
                records = planner.page(self.state, response, job, entry, replay)
            else:
                records = planner.resolved(response, job, entry, replay)
            if response.status in (429, 503) or response.status >= 500:
                replay.update(state="waiting_retry" if task["attempts"] < 3 else "unavailable",
                              retry_at=time.time() + max(response.retry_after, 60))
            elif response.status in (404, 410):
                replay["state"] = "unavailable"
        else:
            with self.state.db() as db:
                scan = db.execute("SELECT force_detail FROM pinterest_streams WHERE scan_id=? AND job_id=?", (entry.get("scan_id"), job["id"])).fetchone()
            if scan and scan[0] and not entry.get("confirmed_detail"):
                replay.update(state="superseded", reason="list_manifest_requires_detail")
                replay["next_tasks"].append(planner.task("pin_detail", entry["pin_id"], pin_id=entry["pin_id"], scan_id=entry["scan_id"]))
                receipts.save_prepared(root, replay, {})
                return dict(replay=replay, records={}, media=None)
            result, path = downloader.acquire(self.state, lib, job, task, entry, self.resources, cancelled, http=self.image_http)
            records = {}
            if result["state"] == "downloaded":
                if not result.get("archive_reuse"):
                    media = {k: result[k] for k in ("download_key", "download_sha256", "width", "height", "ext", "content_type")}
                records["acquisitions"] = [{k: result[k] for k in arrow_schema("acquisitions").names}]
                records["assets"] = [dict(asset_id=stable_id("pinterest-asset-v1", entry["media_id"], "original"),
                    media_id=entry["media_id"], acquisition_id=result["acquisition_id"], sha256=result["download_sha256"],
                    representation="original", recipe_id="original", acquired_at=result["acquired_at"])]
                planner.metric(replay, "acquisition_reuses" if result.get("reused") else "original_downloads")
                if result.get("reuse_key"):
                    replay["reuse_acquisition"] = result
                replay["download_key"] = result["download_key"]
                if result["evidence"] != "downloaded":
                    planner.metric(replay, result["evidence"])
                planner.metric(replay, "new_byte_objects" if lib.lookup_object(result["download_sha256"]) is None else "existing_byte_bindings")
                if entry.get("scan_id"):
                    with self.state.db() as db:
                        origin = db.execute("SELECT entrypoint FROM pinterest_streams WHERE scan_id=?", (entry["scan_id"],)).fetchone()
                    if origin:
                        replay.setdefault("seen", []).append(dict(entrypoint=origin[0], kind="objects", value=result["download_sha256"]))
            else:
                state = "waiting_retry" if result["state"] == "failed" and task["attempts"] < 3 else "needs_review"
                replay.update(state=state, reason=result.get("reason"), retry_at=result.get("retry_at", 0))
        receipts.save_prepared(root, replay, records, media)
        failpoint("pinterest_after_prepared")
        return dict(replay=replay, records=records, media=media)

    def _run(self, lib, identity, deadline, started):
        job = self.service.row(identity)
        # Accepted work always replays and publishes, even if the next user command is cancel.
        while receipts.replay(self.state, lib, identity) == 128:
            pass
        published = lib.sync_online()
        with self.state.db() as db:
            db.execute("UPDATE pinterest_jobs SET served_seq=? WHERE id=?", (published["served_seq"], identity))
        if job["desired_state"] != "running":
            self.update_state(identity, "cancelled" if job["desired_state"] == "cancelled" else "paused")
            if job["desired_state"] == "cancelled":
                self.cleanup(lib, identity)
            return
        if job["state"] in model.TERMINAL:
            self.cleanup(lib, identity)
            return
        self._intent(lib, job)
        self.update_state(identity, "running")
        spec = json.loads(job["definition_json"])
        def cancelled():
            return self._cancelled(identity, deadline)
        client = self.client_factory(self.state.root, json.loads(job["context_json"]), cancelled=cancelled)
        try:
            while not cancelled():
                current = self.service.row(identity)
                with self.state.db() as db:
                    pending = db.execute("SELECT 1 FROM pinterest_tasks WHERE job_id=? AND state='running' LIMIT 1", (identity,)).fetchone()
                baseline = json.loads(current["budget_baseline_json"])
                if not pending and current["elapsed_seconds"] + time.monotonic() - started - baseline.get("elapsed_seconds", 0) >= spec["run_budget"]["wall_seconds"]:
                    self.update_state(identity, "waiting_budget", "PINTEREST_RUN_BUDGET")
                    return
                task = self._claim(identity, spec)
                if task and "blocked" in task:
                    continue
                if task is None:
                    with self.state.db() as db:
                        # A drained media queue can release discovery backpressure without a new budget round.
                        backlog = db.execute("SELECT coalesce(sum(n),0) FROM pinterest_counts WHERE job_id=? AND kind IN ('media_download','pin_admit') AND state IN ('queued','running','waiting_retry','waiting_budget')", (identity,)).fetchone()[0]
                        if backlog < spec.get("discovery", {}).get("max_pending_downloads", 128):
                            released = db.execute("UPDATE pinterest_tasks SET state='queued',reason=NULL WHERE job_id=? AND state='waiting_budget' AND reason='discovery_backlog'", (identity,)).rowcount
                            if released:
                                continue
                        retry = db.execute("SELECT min(retry_at) FROM pinterest_tasks WHERE job_id=? AND state='waiting_retry'", (identity,)).fetchone()[0]
                        gaps = db.execute("SELECT 1 FROM pinterest_tasks WHERE job_id=? AND kind<>'pin_enrichment' AND state IN ('needs_review','unavailable') LIMIT 1", (identity,)).fetchone()
                        limited = db.execute("SELECT 1 FROM pinterest_tasks WHERE job_id=? AND state='waiting_budget' LIMIT 1", (identity,)).fetchone()
                    self.update_state(identity, "waiting_retry" if retry else "waiting_budget" if limited else "completed_with_gaps" if gaps else "completed",
                                      retry=max(1, retry - time.time()) if retry else 0)
                    if not retry and not limited:
                        self.cleanup(lib, identity)
                    return
                # Recover a sealed batch without reconstructing mutable execution inputs.
                sealed = next((p for p in (lib.root / "segments" / task["receipt_id"], lib.root / "staging" / task["receipt_id"])
                               if (p / "manifest.json").exists()), None)
                prepared = dict(replay=read_json(sealed / "replay.json"), records={}, media=None) if sealed else self._prepare(lib, job, task, client, cancelled)
                self._publish(lib, job, **prepared)
                reason = prepared["replay"].get("reason")
                if reason in ("pin_http_401", "pin_http_403", "pin_http_301", "pin_http_302", "pin_response_invalid", "image_http_401", "image_http_403"):
                    self.update_state(identity, "needs_review", reason)
                    return
            current = self.service.row(identity)
            self.update_state(identity, {"running": "queued", "paused": "paused", "cancelled": "cancelling"}[current["desired_state"]])
        finally:
            client.close()

    def cleanup(self, lib, identity):
        # Successful acquisition files must remain available for unfinished/failed bindings sharing their URL.
        row = self.service.row(identity)
        root = receipts.directory(lib, identity)
        # Inspect only still-staged receipts, not the entire accepted history of a growing job.
        for receipt_path in root.glob("*.prepared.json"):
            receipt_id = model.identity(receipt_path.name.removesuffix(".prepared.json"))
            with lib.journal() as journal:
                archived = journal.execute("SELECT 1 FROM pinterest_receipts WHERE job_id=? AND receipt_id=?", (identity, receipt_id)).fetchone()
            contained(lib.cache, "pack_staging/" + receipt_id + ".tar.tmp").unlink(missing_ok=True)
            staging = contained(lib.root, "staging/" + receipt_id)
            if not archived and staging.exists() and row["state"] == "cancelled":
                for path in staging.iterdir():
                    if path.is_file() and (path.suffix == ".parquet" or path.name in {"media.tar", "run.json", "replay.json", "manifest.json"}):
                        path.unlink()
                if not any(staging.iterdir()):
                    staging.rmdir()
            if archived:
                receipt_path.unlink(missing_ok=True)
                (root / (receipt_id + ".response.json")).unlink(missing_ok=True)
        if row["state"] == "completed_with_gaps":
            return
        for folder in ("pinterest_downloads", "pinterest_receipts"):
            root = contained(lib.cache, folder + "/" + model.identity(identity))
            if root.exists():
                for path in root.iterdir():
                    if path.is_file() and re.fullmatch(r"[0-9a-f-]{32,64}\.(?:downloaded|partial|download\.json|partial\.json|response\.json|prepared\.json)", path.name):
                        path.unlink()
                if not any(root.iterdir()):
                    root.rmdir()
        with self.state.db() as db:
            if row["state"] == "cancelled":
                db.execute("UPDATE pinterest_tasks SET state='cancelled',reason='cancelled' WHERE job_id=? AND state IN ('queued','running','waiting_retry','waiting_budget')", (identity,))
            db.execute("UPDATE pinterest_jobs SET cleanup_state='complete' WHERE id=?", (identity,))
