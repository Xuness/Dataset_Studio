"""Finite, resumable collection slices inside the existing owned worker."""

from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import shutil
import threading
import time
import uuid

from . import TERMINAL, TASK_TERMINAL, media, planner, receipts
from .service import Service
from ..collectors.pixiv import normalize
from ..collectors.pixiv.http import Client, MediaSessions, MAX_RESPONSE
from ..media_lake.schema import canonical, utc
from ..updates import locations
from ..updates.resources import Resources
from ..updates.sites import UpdateError
from ..updates.staging import estimate
from ..util import FileLock, IntegrityError, read_json, safe_managed_path


def exception_code(error):
    if isinstance(error, UpdateError):
        return "COLLECTION_LIMIT" if error.code == "UPDATE_RESOURCE_LIMIT" else error.code
    return "COLLECTION_IO" if isinstance(error, OSError) else "COLLECTION_INTEGRITY"


class Runner:
    def __init__(self, state, *, resources=None, stop=None, client_factory=Client, image_http=None):
        self.service = Service(state)
        self.state = state
        self.resources = resources or Resources()
        self.stop = stop or threading.Event()
        self.client_factory, self.image_http = client_factory, image_http

    def update_state(self, identity, state, reason=None, retry=0):
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            current = self.service.row(identity, db)
            if current["desired_state"] == "pause":
                state = "paused"
            elif current["desired_state"] == "cancel":
                state = "cancelled"
            db.execute("UPDATE collection_jobs SET state=?,error_code=?,retry_at_ms=?,revision=revision+1,updated_at=? WHERE id=?",
                       (state, reason, int((time.time() + retry) * 1000) if retry else 0, utc(), identity))

    def run(self, identity, *, time_slice=None):
        initial = self.service.row(identity)
        if initial["state"] == "cancelled":
            self.cleanup(identity)
            return self.service.job(identity)
        if initial["state"] in TERMINAL | {"paused", "waiting_credentials", "waiting_budget", "needs_review"}:
            return self.service.job(identity)
        try:
            with locations.access(self.state, initial["lake_id"]), self.service.execution_lock(identity):
                lib = self.state.library(initial["lake_id"])
                with FileLock(lib.cache / ".daily-run.lock", timeout=0):
                    self._run(lib, identity, time_slice=time_slice)
        except Exception as error:
            if isinstance(error, RuntimeError) and str(error).startswith("另一个进程正在使用此工作区"):
                return self.service.job(identity)
            code = exception_code(error)
            target = {"COLLECTION_CREDENTIAL_REQUIRED": "waiting_credentials", "COLLECTION_SCOPE_CHANGED": "needs_review",
                      "COLLECTION_REMOTE_UNAVAILABLE": "waiting_retry", "UPDATE_SPACE": "waiting_resources",
                      "COLLECTION_IO": "waiting_resources", "CANCELLED": "queued"}.get(code, "needs_review")
            self.update_state(identity, target, code, getattr(error, "retry_after", 0))
            # Exceptions can contain response URLs. Retain structure, never exception text.
            import traceback

            with (self.state.root / "collection-errors.jsonl").open("a", encoding="utf-8") as log:
                log.write(canonical(dict(at=utc(), job_id=identity, code=code, exception=type(error).__name__,
                                         frames=[dict(file=Path(f.filename).name, line=f.lineno, function=f.name) for f in traceback.extract_tb(error.__traceback__)])) + "\n")
        result = self.service.job(identity)
        if result["state"] == "cancelled":
            self.cleanup(identity)
        return self.service.job(identity)

    def cleanup(self, identity):
        """Cancel removes only this job's scratch after all durable archive receipts are applied."""
        job = self.service.row(identity)
        if job["state"] != "cancelled":
            return
        with locations.access(self.state, job["lake_id"]), self.service.execution_lock(identity):
            lib = self.state.library(job["lake_id"])
            with FileLock(lib.cache / ".daily-run.lock", timeout=0):
                self.recover(lib, identity)
                receipts.publish_ack(self.service, lib, identity)
                receipts.release(self.service, lib, identity, resources=self.resources)
                with self.state.db() as db:
                    if db.execute("SELECT 1 FROM collection_outbox WHERE job_id=? AND (control_applied=0 OR published=0) LIMIT 1", (identity,)).fetchone():
                        raise IntegrityError("Cancelled job still has unacknowledged acquisition receipts")
                root = safe_managed_path(self.state.root, self.state.root / "collection-spool" / identity)
                if root.exists():
                    for directory in root.iterdir():
                        directory = safe_managed_path(self.state.root, directory)
                        owner = read_json(directory / "owner.json")
                        if owner.get("job_id") != identity or owner.get("definition_sha256") != job["definition_sha256"]:
                            raise IntegrityError("Cancellation scratch owner mismatch")
                        for child in directory.iterdir():
                            child = safe_managed_path(self.state.root, child)
                            if not child.is_file():
                                raise IntegrityError("Unexpected nested cancellation scratch")
                            child.unlink()
                        directory.rmdir()
                        self.resources.retire(directory)
                    root.rmdir()
                with self.state.db() as db:
                    db.execute("BEGIN IMMEDIATE")
                    current = self.service.row(identity, db)
                    counters = json.loads(current["counters_json"])
                    counters["cleanup"] = "complete"
                    db.execute("UPDATE collection_tasks SET state='cancelled' WHERE job_id=? AND state NOT IN ('done','unavailable','excluded','needs_review')", (identity,))
                    db.execute("UPDATE collection_jobs SET counters_json=?,updated_at=? WHERE id=?", (canonical(counters), utc(), identity))

    def cancelled(self, identity):
        if self.stop.is_set():
            return True
        with self.state.db() as db:
            row = db.execute("SELECT desired_state FROM collection_jobs WHERE id=?", (identity,)).fetchone()
        return not row or row[0] != "run"

    def recover(self, lib, identity):
        # Accept old staged claims before issuing a new execution epoch.
        while True:
            with self.state.db() as db:
                row = db.execute("SELECT * FROM collection_outbox WHERE job_id=? AND state='prepared' ORDER BY created_at,id LIMIT 1", (identity,)).fetchone()
            if row is None:
                break
            receipts.accept(self.service, lib, dict(row))
        receipts.reconcile(self.service, lib)

    def _run(self, lib, identity, *, time_slice):
        self.recover(lib, identity)
        job = self.service.row(identity)
        counters = json.loads(job["counters_json"])
        if counters.get("publication_only"):
            receipts.publish_ack(self.service, lib, identity)
            receipts.release(self.service, lib, identity, resources=self.resources)
            with self.state.db() as db:
                current = self.service.row(identity, db)
                counts = json.loads(current["counters_json"])
                state = counts.pop("return_state", "paused")
                counts.pop("publication_only", None)
                db.execute("UPDATE collection_jobs SET state=?,counters_json=?,revision=revision+1,updated_at=? WHERE id=?", (state, canonical(counts), utc(), identity))
            return
        if job["desired_state"] != "run":
            self.update_state(identity, "queued")
            return
        account, cookie_values, context = self.service.accounts.session(job["account_id"])
        frozen = json.loads(job["visibility_json"]) if job["visibility_json"] else None
        if frozen and frozen["comparison_key"] != context["comparison_key"]:
            raise UpdateError("COLLECTION_SCOPE_CHANGED", "Known session identity or display conditions changed")
        context = frozen or context
        with self.state.db() as db:
            db.execute("UPDATE collection_jobs SET state='running',execution_epoch=execution_epoch+1,visibility_json=?,revision=revision+1,error_code=NULL,updated_at=? WHERE id=? AND desired_state='run'",
                       (canonical(context), utc(), identity))
            db.execute("UPDATE collection_tasks SET state='queued' WHERE job_id=? AND state IN ('running','waiting_resources')", (identity,))
        job = self.service.row(identity)
        with lib.journal() as journal:
            intent = journal.execute("SELECT 1 FROM collection_runs WHERE job_id=?", (identity,)).fetchone()
        if intent is None:
            receipts.prepared(self.service, job, None, {}, intent=True)
            self.recover(lib, identity)
        self.loop(lib, identity, context, account, cookie_values, time_slice)

    def consume(self, identity, **deltas):
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            row = self.service.row(identity, db)
            counts = json.loads(row["counters_json"])
            for key, value in deltas.items():
                counts["round"][key] += value
                if key == "download_bytes":
                    counts["download_bytes"] += value
            db.execute("UPDATE collection_jobs SET counters_json=? WHERE id=?", (canonical(counts), identity))

    def claim(self, job, *, media_task=False):
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            current = self.service.row(job["id"], db)
            if current["desired_state"] != "run" or current["execution_epoch"] != job["execution_epoch"]:
                return None
            row = db.execute("SELECT * FROM collection_tasks WHERE job_id=? AND kind " + ("=" if media_task else "<>") + " 'media_download' AND state IN ('queued','retry_wait') AND retry_at_ms<=? ORDER BY priority DESC,task_row LIMIT 1",
                             (job["id"], int(time.time() * 1000))).fetchone()
            if row is None:
                return None
            task = dict(row)
            task.update(claimed_epoch=job["execution_epoch"], claim_token=str(uuid.uuid4()), state="running", attempts=task["attempts"] + 1)
            previous = task["result_receipt"]
            directory = receipts.staging(self.service, job, task["claim_token"])
            if previous:
                old = safe_managed_path(self.state.root, self.state.root / previous).parent
                if old.is_dir() and read_json(old / "owner.json")["job_id"] == job["id"]:
                    # A released execution cannot mutate these files. Partial.load validates
                    # hashes, validators and the fixed locator before range continuation.
                    for suffix in (".downloaded", ".download.json", ".partial", ".partial.json"):
                        candidate = safe_managed_path(self.state.root, old / (task["id"] + suffix))
                        if candidate.is_file():
                            shutil.copyfile(candidate, directory / candidate.name)
            marker = str((directory / "result.json").relative_to(self.state.root))
            db.execute("UPDATE collection_tasks SET state='running',claimed_epoch=?,claim_token=?,attempts=?,result_receipt=?,reason=NULL,updated_at=? WHERE id=?",
                       (task["claimed_epoch"], task["claim_token"], task["attempts"], marker, utc(), task["id"]))
            task["directory"] = directory
            return task

    def failure(self, job, task, error):
        code = exception_code(error)
        if code == "CANCELLED":
            state = "queued"
        elif code in {"COLLECTION_CREDENTIAL_REQUIRED", "COLLECTION_SCOPE_CHANGED"}:
            state = "waiting_credentials"
        elif code in {"UPDATE_SPACE", "COLLECTION_IO"}:
            state = "waiting_resources"
        elif code == "COLLECTION_NOT_ACCESSIBLE":
            state = "unavailable"
        elif code == "COLLECTION_SOURCE_CHANGED":
            state = "unavailable"
        elif code == "COLLECTION_REMOTE_UNAVAILABLE" and task["attempts"] < 8:
            state = "retry_wait"
        else:
            state = "needs_review"
        if state in {"unavailable", "needs_review"}:
            receipts.prepared(self.service, job, task, {}, state=state, reason=code, directory=task["directory"])
        else:
            with self.state.db() as db:
                db.execute("UPDATE collection_tasks SET state=?,reason=?,retry_at_ms=?,updated_at=? WHERE id=? AND claim_token=?",
                           (state, code, int((time.time() + max(1, getattr(error, "retry_after", 0), min(300, 2 ** task["attempts"]))) * 1000) if state == "retry_wait" else 0, utc(), task["id"], task["claim_token"]))
        if state == "waiting_credentials":
            raise error

    def metadata(self, job, task, context, client):
        payload, kind = json.loads(task["payload_json"]), task["kind"]
        if kind == "relationship_page":
            with self.state.db() as db:
                depth = db.execute("SELECT min_depth FROM collection_entities WHERE job_id=? AND kind=? AND source_id=?", (job["id"], payload["root_kind"], payload["root_id"])).fetchone()[0]
            if depth >= json.loads(job["definition_json"])["discovery"]["max_depth"]:
                return receipts.prepared(self.service, job, task, {}, state="excluded", reason="depth_limit", directory=task["directory"])
        self.consume(job["id"], api_requests=1)
        response = client.request(kind, payload)
        subject_kind = {"author_profile": "author", "author_directory": "directory", "work_detail": "work", "media_manifest": "media", "relationship_page": "relation"}[kind]
        receipt_id = str(uuid.uuid4())
        captured = normalize.capture(job["lake_id"], context, receipt_id, response.endpoint, subject_kind, task["subject_key"], response.body,
                                     observed_at=response.observed_at, request=dict(parameters=response.parameters, content_encoding=response.content_encoding))
        if kind == "author_profile":
            normalized = normalize.author(captured)
        elif kind == "author_directory":
            normalized = normalize.directory(captured, job["id"])
        elif kind == "work_detail":
            normalized = normalize.work(captured)
        elif kind == "media_manifest":
            normalized = normalize.manifest(captured, payload["detail"])
        else:
            normalized = self.relationship(captured, job, payload)
        records = {"visibility_contexts": [context], "captures": [captured], **normalized}
        checkpoints = []
        for snapshot in normalized.get("discovery_snapshots", []):
            with self.state.db() as db:
                old = db.execute("SELECT revision FROM collection_checkpoints WHERE job_id=? AND stream_key=?", (job["id"], snapshot["stream_key"])).fetchone()
            revision = old[0] if old else 0
            checkpoints.append(dict(stream_key=snapshot["stream_key"], expected_revision=revision, next_revision=revision + 1,
                                    next_cursor=json.loads(snapshot["next_cursor_json"]) if snapshot["next_cursor_json"] else {},
                                    exhausted=snapshot["traversal_exhausted"], capture_id=captured["capture_id"]))
        state, reason = "done", None
        if kind == "work_detail" and not planner.scope_allows(normalized["work_observations"][0], json.loads(job["definition_json"])):
            state, reason = "excluded", "scope_filter"
        if kind == "media_manifest" and not normalized["media_manifests"][0]["complete"]:
            state, reason = "needs_review", normalized["media_manifests"][0]["reason"]
        return receipts.prepared(self.service, job, task, records, checkpoints=checkpoints, state=state, reason=reason,
                                 directory=task["directory"], receipt_id=receipt_id)

    @staticmethod
    def relationship(captured, job, payload):
        body = normalize.body(captured)
        relation, offset = payload["relation"], payload["cursor"]
        if relation == "recommendations":
            values = body.get("illusts")
            remaining = body.get("nextIds", [])
            if not isinstance(values, list) or not isinstance(remaining, list):
                raise IntegrityError("Unrecognized recommendations response")
            identities = list(dict.fromkeys([normalize.source_id(v["id"]) for v in values] + [normalize.source_id(v) for v in remaining]))
            members, next_cursor, exhausted = [("work", v) for v in identities], None, True
        else:
            values = body.get("users" if relation == "following" else "works")
            if not isinstance(values, list) or len(values) > 100:
                raise IntegrityError("Unrecognized relationship page")
            members = [("author" if relation == "following" else "work", normalize.source_id(v["userId" if relation == "following" else "id"])) for v in values]
            total = body.get("total")
            exhausted = len(values) < 100 or (isinstance(total, int) and offset + len(values) >= total)
            next_cursor = None if exhausted else dict(offset=offset + len(values), directory_snapshot_id=payload.get("directory_snapshot_id"))
        return normalize.discovery(captured, job_id=job["id"], stream_key=f"{payload['root_kind']}:{payload['root_id']}:{relation}",
                                   relation=relation, root_kind=payload["root_kind"], members=members, page_key=str(offset), next_cursor=next_cursor, exhausted=exhausted)

    def loop(self, lib, identity, context, account, cookies, time_slice):
        settings = self.service.pipeline()
        config = settings["value"]
        self.resources.configure(settings["shared_limits"])
        self.resources.configure_source("pixiv", config["pixiv"])
        duration = config["time_slice_seconds"] if time_slice is None else time_slice
        started, accounted = time.monotonic(), time.monotonic()
        job = self.service.row(identity)
        spec = json.loads(job["definition_json"])
        def cancelled():
            if self.cancelled(identity):
                return True
            current_account = self.service.accounts.row(account["id"])
            return current_account["revision"] != account["revision"] or current_account["state"] != "valid"

        client = self.client_factory(self.state.root, account, cookies, cancelled=cancelled, api_rate=config["pixiv"]["api_requests_per_second"])
        sessions = MediaSessions(self.image_http)
        futures, reservations, publishing, publish_error = {}, {}, None, None
        try:
            if account["mode"] == "session":
                if json.loads(job["counters_json"])["round"]["api_requests"] >= spec["run_budget"]["api_requests"]:
                    self.finish(identity, None)
                    return
                self.consume(identity, api_requests=1)
                try:
                    verified = client.probe()
                except UpdateError as error:
                    if error.code == "COLLECTION_CREDENTIAL_REQUIRED":
                        self.service.accounts.expire(account["id"], account["revision"])
                    raise
                if verified["comparison_key"] != context["comparison_key"]:
                    raise UpdateError("COLLECTION_SCOPE_CHANGED", "Server-confirmed session identity or visibility changed")
            with ThreadPoolExecutor(max_workers=config["pixiv"]["download_concurrency"] + 1) as pool:
                while True:
                    self.recover(lib, identity)
                    if publishing and publishing.done():
                        try:
                            publishing.result()
                            publish_error = None
                        except Exception as error:
                            publish_error = error
                        publishing = None
                    if publishing is None and publish_error is None:
                        publishing = pool.submit(receipts.publish_ack, self.service, lib, identity)
                    receipts.release(self.service, lib, identity, resources=self.resources)
                    for future, (task, reservation, slot) in list(futures.items()):
                        if not future.done():
                            continue
                        del futures[future]
                        slot.release()
                        try:
                            result = future.result()
                            receipts.prepared(self.service, self.service.row(identity), task, {}, **result, directory=task["directory"])
                            reservations[task["claim_token"]] = reservation
                        except Exception as error:
                            reservation.release()
                            self.failure(self.service.row(identity), task, error)
                    for claim, reservation in list(reservations.items()):
                        if not (self.state.root / "collection-spool" / identity / claim).exists():
                            reservation.release()
                            del reservations[claim]
                    stamp = time.monotonic()
                    if stamp - accounted > 1:
                        self.consume(identity, wall_seconds=stamp - accounted)
                        accounted = stamp
                    with self.state.db() as db:
                        db.execute("BEGIN IMMEDIATE")
                        current = self.service.row(identity, db)
                        planner.advance(db, current)
                    current = self.service.row(identity)
                    round_counts = json.loads(current["counters_json"])["round"]
                    budget = spec["run_budget"]
                    limited = any(round_counts[k] >= budget[k] for k in ("api_requests", "download_bytes", "wall_seconds"))
                    from ..online_storage import connect
                    from ..util import contained

                    pointer = read_json(lib.cache / "ONLINE.json")
                    online = connect(contained(lib.cache, pointer["file"]))
                    try:
                        served = int(online.execute("SELECT value FROM online_state WHERE key='served_seq'").fetchone()[0])
                    finally:
                        online.close()
                    with lib.journal() as journal:
                        backlog = journal.execute("SELECT coalesce(sum(json_extract(f.value,'$.bytes')),0) FROM commits c,json_each(c.manifest_json,'$.files') f WHERE c.seq>?", (served,)).fetchone()[0]
                    ending = cancelled() or stamp - started >= duration or limited or publish_error is not None or backlog >= config["publication_backlog_mib"] * 1024**2
                    if not ending:
                        if len(futures) < config["pixiv"]["download_concurrency"]:
                            slot = self.resources.try_download("pixiv")
                            if slot:
                                task = self.claim(current, media_task=True)
                                if task:
                                    payload = json.loads(task["payload_json"])
                                    plan = media.Plan(estimate(spec["media"]["image_policy"], payload["entry"], self.resources), spec["media"]["retain_original"])
                                    reservation = self.resources.try_reserve(task["directory"], task["id"], plan.peak())
                                    if reservation:
                                        self.resources.plan(task["directory"], task["id"], plan)
                                        def progress(**values):
                                            if values.get("downloaded_bytes_delta"):
                                                self.consume(identity, download_bytes=values["downloaded_bytes_delta"])
                                        future = pool.submit(media.acquire, lib, task, spec, task["directory"], client, sessions, self.resources,
                                                             cancelled, progress)
                                        futures[future] = (task, reservation, slot)
                                    else:
                                        slot.release()
                                        self.failure(current, task, UpdateError("UPDATE_SPACE", "Shared staging is full"))
                                else:
                                    slot.release()
                        with self.state.db() as db:
                            pending_media = db.execute("SELECT coalesce(sum(n),0) FROM collection_counts WHERE job_id=? AND kind='media_download' AND state NOT IN ('done','unavailable','excluded','needs_review','cancelled')", (identity,)).fetchone()[0]
                        task = self.claim(current) if pending_media < config["pending_media_limit"] else None
                        if task:
                            reservation = self.resources.try_reserve(task["directory"], task["id"], MAX_RESPONSE * 3)
                            if reservation is None:
                                self.failure(current, task, UpdateError("UPDATE_SPACE", "Shared metadata staging is full"))
                                continue
                            try:
                                self.metadata(current, task, context, client)
                                reservations[task["claim_token"]] = reservation
                            except Exception as error:
                                reservation.release()
                                self.failure(current, task, error)
                            continue
                    if futures:
                        self.stop.wait(.05)
                        continue
                    if ending:
                        break
                    with self.state.db() as db:
                        due = db.execute("SELECT 1 FROM collection_tasks WHERE job_id=? AND state='queued' LIMIT 1", (identity,)).fetchone()
                    if due:
                        continue
                    break
                # Drain publication before returning the execution lock. Already accepted
                # control facts remain valid even when serving fails here.
                self.recover(lib, identity)
                if publishing:
                    try:
                        publishing.result()
                    except Exception as error:
                        publish_error = error
                if publish_error is None:
                    receipts.publish_ack(self.service, lib, identity)
                    receipts.release(self.service, lib, identity, resources=self.resources)
                if account["mode"] == "session":
                    renewed = client.cookie_snapshot()
                    if renewed:
                        self.service.accounts.renew(account["id"], account["revision"], renewed)
        finally:
            for _, reservation, slot in futures.values():
                reservation.release()
                slot.release()
            for reservation in reservations.values():
                reservation.release()
            # Executor context has joined all users of these sessions.
            client.close()
            sessions.close()
            self.consume(identity, wall_seconds=max(0, time.monotonic() - accounted))
        self.finish(identity, publish_error)

    def finish(self, identity, publish_error):
        with self.state.db() as db:
            job = self.service.row(identity, db)
            counts = json.loads(job["counters_json"])
            progress = self.service.progress(job, db)
            pending = list(db.execute("SELECT state,sum(n) FROM collection_counts WHERE job_id=? AND n>0 GROUP BY state", (identity,)))
            states = {s: n for s, n in pending}
            candidates = db.execute("SELECT 1 FROM collection_entities WHERE job_id=? AND state='candidate' LIMIT 1", (identity,)).fetchone() is not None
            budget = json.loads(job["definition_json"])["run_budget"]
        if publish_error:
            self.update_state(identity, "publishing", exception_code(publish_error), retry=30)
        elif any(s not in TASK_TERMINAL for s in states) or candidates:
            if any(counts["round"][k] >= budget[k] for k in ("api_requests", "download_bytes", "wall_seconds")) or (candidates and counts["round"]["admitted_authors"] >= budget["admitted_authors"]):
                self.update_state(identity, "waiting_budget", "run_budget_exhausted")
            elif states.get("waiting_resources"):
                self.update_state(identity, "waiting_resources", "shared_resource_wait", retry=5)
            elif states.get("waiting_credentials"):
                self.update_state(identity, "waiting_credentials", "COLLECTION_CREDENTIAL_REQUIRED")
            elif states.get("retry_wait") and not states.get("queued"):
                with self.state.db() as db:
                    retry = db.execute("SELECT min(retry_at_ms) FROM collection_tasks WHERE job_id=? AND state='retry_wait'", (identity,)).fetchone()[0]
                self.update_state(identity, "waiting_retry", "remote_retry", retry=max(1, retry / 1000 - time.time()))
            else:
                self.update_state(identity, "queued")
        else:
            gaps = states.get("unavailable", 0) + states.get("needs_review", 0)
            receipts.prune_finished(self.service, identity, self.resources)
            self.update_state(identity, "completed_with_gaps" if gaps or not all(progress["closure"].values()) else "completed")
