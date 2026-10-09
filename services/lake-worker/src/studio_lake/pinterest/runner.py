"""Bounded Pinterest execution slices with receipt-first recovery and fenced acceptance."""

import json
import re
import threading
import time
import uuid

from . import downloader, model, normalize, receipts
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
                    self._run(lib, identity, started + time_slice)
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

    def _intent(self, lib, job):
        identity = str(uuid.uuid5(uuid.UUID(job["id"]), "pinterest-intent-v1"))
        spec = json.loads(job["definition_json"])
        replay = dict(kind="intent", receipt_id=identity,
            next_tasks=[dict(kind="pin_detail", pin_id=s["id"], input=dict(pin_id=s["id"])) for s in spec["seeds"]])
        with lib.journal() as db:
            exists = db.execute("SELECT 1 FROM pinterest_runs WHERE job_id=?", (job["id"],)).fetchone()
        if not exists:
            self._publish(lib, job, replay, {}, definition=spec)

    def _claim(self, identity):
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            row = db.execute("SELECT * FROM pinterest_tasks WHERE job_id=? AND (state IN ('running','queued') OR "
                "(state='waiting_retry' AND retry_at<=?)) ORDER BY CASE state WHEN 'running' THEN 0 ELSE 1 END,"
                "CASE kind WHEN 'media_download' THEN 0 ELSE 1 END,task_row LIMIT 1", (identity, time.time())).fetchone()
            if not row:
                return None
            task = dict(row)
            if task["state"] != "running":
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
        if task["kind"] == "pin_detail":
            response = receipts.load_response(root, task["receipt_id"])
            if response is None:
                with self.state.db() as db:
                    db.execute("UPDATE pinterest_jobs SET api_requests=api_requests+1 WHERE id=?", (job["id"],))
                response = client.pin(task["pin_id"])
                receipts.save_response(root, task["receipt_id"], response)
                failpoint("pinterest_after_response")
            records, parsed = normalize.response_facts(response, task["receipt_id"], task["pin_id"])
            replay.update(state="done" if parsed["state"] == "ready" else "needs_review", reason=parsed["reason"])
            if response.status in (429, 503) or response.status >= 500:
                replay.update(state="waiting_retry" if task["attempts"] < 3 else "unavailable",
                              retry_at=time.time() + max(response.retry_after, 60))
            elif response.status in (404, 410):
                replay["state"] = "unavailable"
            replay["next_tasks"] = [dict(kind="media_download", pin_id=task["pin_id"], input={**v, "context_id": parsed["context_id"]}) for v in parsed["entries"]]
        else:
            result, path = downloader.acquire(self.state, lib, job, task, entry, self.resources, cancelled, http=self.image_http)
            records = {}
            if result["state"] == "downloaded":
                media = {k: result[k] for k in ("download_key", "download_sha256", "width", "height", "ext", "content_type")}
                records["acquisitions"] = [{k: result[k] for k in arrow_schema("acquisitions").names}]
                records["assets"] = [dict(asset_id=stable_id("pinterest-asset-v1", entry["media_id"], "original"),
                    media_id=entry["media_id"], acquisition_id=result["acquisition_id"], sha256=result["download_sha256"],
                    representation="original", recipe_id="original", acquired_at=result["acquired_at"])]
            else:
                state = "waiting_retry" if result["state"] == "failed" and task["attempts"] < 3 else "needs_review"
                replay.update(state=state, reason=result.get("reason"), retry_at=result.get("retry_at", 0))
        receipts.save_prepared(root, replay, records, media)
        failpoint("pinterest_after_prepared")
        return dict(replay=replay, records=records, media=media)

    def _run(self, lib, identity, deadline):
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
                    admitted = db.execute("SELECT count(*) FROM pinterest_tasks WHERE job_id=? AND kind='pin_detail' AND (attempts>0 OR download_generation>0)", (identity,)).fetchone()[0]
                    next_kind = db.execute("SELECT kind,attempts,download_generation FROM pinterest_tasks WHERE job_id=? AND (state='queued' OR (state='waiting_retry' AND retry_at<=?)) "
                        "ORDER BY CASE kind WHEN 'media_download' THEN 0 ELSE 1 END,task_row LIMIT 1", (identity, time.time())).fetchone()
                budget = spec["run_budget"]
                if not pending and next_kind and (current["download_bytes"] >= budget["download_bytes"]
                        or current["elapsed_seconds"] >= budget["wall_seconds"]
                        or (next_kind[0] == "pin_detail" and (current["api_requests"] >= min(budget["api_requests"], budget["detail_requests"])
                            or (next_kind[1] == 0 and next_kind[2] == 0 and admitted >= budget["admitted_pins"])))):
                    self.update_state(identity, "waiting_budget", "PINTEREST_RUN_BUDGET")
                    return
                task = self._claim(identity)
                if task is None:
                    with self.state.db() as db:
                        retry = db.execute("SELECT min(retry_at) FROM pinterest_tasks WHERE job_id=? AND state='waiting_retry'", (identity,)).fetchone()[0]
                        gaps = db.execute("SELECT 1 FROM pinterest_tasks WHERE job_id=? AND state IN ('needs_review','unavailable') LIMIT 1", (identity,)).fetchone()
                    self.update_state(identity, "waiting_retry" if retry else "completed_with_gaps" if gaps else "completed",
                                      retry=max(1, retry - time.time()) if retry else 0)
                    if not retry:
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
        with lib.journal() as journal:
            archived = {r[0] for r in journal.execute("SELECT receipt_id FROM pinterest_receipts WHERE job_id=?", (identity,))}
        with self.state.db() as db:
            claims = {r[0] for r in db.execute("SELECT receipt_id FROM pinterest_tasks WHERE job_id=? AND receipt_id IS NOT NULL", (identity,))}
        for receipt_id in archived | claims:
            model.identity(receipt_id)
            contained(lib.cache, "pack_staging/" + receipt_id + ".tar.tmp").unlink(missing_ok=True)
            staging = contained(lib.root, "staging/" + receipt_id)
            if receipt_id not in archived and staging.exists() and row["state"] == "cancelled":
                for path in staging.iterdir():
                    if path.is_file() and (path.suffix == ".parquet" or path.name in {"media.tar", "run.json", "replay.json", "manifest.json"}):
                        path.unlink()
                if not any(staging.iterdir()):
                    staging.rmdir()
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
                db.execute("UPDATE pinterest_tasks SET state='cancelled',reason='cancelled' WHERE job_id=? AND state IN ('queued','running','waiting_retry')", (identity,))
            db.execute("UPDATE pinterest_jobs SET cleanup_state='complete' WHERE id=?", (identity,))
