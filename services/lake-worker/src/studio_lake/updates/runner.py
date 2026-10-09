"""Background coordinator: one owner, concurrent lakes, ordered archive publications."""

from concurrent.futures import ThreadPoolExecutor
from contextlib import nullcontext
import json
from pathlib import Path
import threading
import time

from ..library import Batch
from ..online_migrate import connect
from ..util import FileLock, atomic_json, contained, now, read_json, stable_id
from .archive import commit_page, commit_media, reconcile, online, io_lock, checkpoint_key
from .media import Resources
from .settings import read as read_settings
from .protocol import timestamp
from .sites import Site, UpdateError, Response
from .state import State, TERMINAL
from .references import categories
from . import cleanup, dispatch, locations


def lease(lib, identity, sequence=None, expires_ms=None):
    with FileLock(lib.cache / ".online.lock"):
        pointer = read_json(lib.cache / "ONLINE.json")
        db = connect(contained(lib.cache, pointer["file"]))
        try:
            with db:
                if sequence is None:
                    db.execute("DELETE FROM leases WHERE id=?", ("update:" + identity,))
                else:
                    floor = int(next(db.execute("SELECT value FROM online_state WHERE key='min_seq'"))[0])
                    if sequence < floor:
                        raise UpdateError("SOURCE_CHANGED", "The input version is no longer retained")
                    db.execute(
                        "INSERT INTO leases VALUES(?,?,?,?,'update-input') ON CONFLICT(id) DO UPDATE SET seq=excluded.seq,expires_ms=excluded.expires_ms",
                        ("update:" + identity, sequence, expires_ms, identity),
                    )
        finally:
            db.close()


class Runner:
    def __init__(self, state, sites=None, resources=None, image_http=None):
        self.state = state
        self.sites = sites or {}
        self.resources = resources or Resources()
        self.image_http = image_http
        self.stop = threading.Event()
        self.site_lock = threading.Lock()
        self.settings_lock = threading.Lock()
        self.settings_revision = -1
        self.settings_checked = 0.0
        self.slice_context = threading.local()
        self.injected_sites = set(self.sites)
        self.refresh_settings(force=True)
        from ..collections.runner import Runner as CollectionRunner

        self.collections = CollectionRunner(state, resources=self.resources, stop=self.stop)

    def refresh_settings(self, force=False):
        with self.settings_lock:
            if not force and time.monotonic() - self.settings_checked < 1:
                return
            snapshot = read_settings(self.state)
            self.settings_checked = time.monotonic()
            if snapshot["revision"] == self.settings_revision:
                return
            self.resources.configure(snapshot["value"])
            self.settings_revision = snapshot["revision"]
            with self.site_lock:
                for name, site in self.sites.items():
                    if name not in self.injected_sites:
                        site.delay = 1 / snapshot["value"]["sites"][name]["api_requests_per_second"]

    def site(self, name):
        with self.site_lock:
            credentials, context = self.state.credential_snapshot(name)
            if name not in self.sites:
                self.sites[name] = Site(
                    name,
                    credentials,
                    rate_root=self.state.root,
                    delay=1 / self.resources.config["sites"][name]["api_requests_per_second"],
                )
            elif isinstance(self.sites[name], Site):
                self.sites[name].credentials = credentials
            self.sites[name].query_context = context
            return self.sites[name]

    def run(self, identity):
        initial = self.state.job(identity)
        try:
            with locations.access(self.state, initial["lake_id"]), self.state.execution_lock(identity):
                self._run_guarded(identity)
        except UpdateError as error:
            if error.code != "UPDATE_CONFLICT":
                raise
        except RuntimeError as error:
            if not str(error).startswith("另一个进程正在使用此工作区"):
                raise
        job = self.state.job(identity)
        if job["state"] == "cancelled":
            self.cleanup(identity)
        return self.state.job(identity)

    def run_slice(self, identity):
        """Daemon fairness only; explicit legacy CLI runs retain their previous semantics."""
        duration = self.collections.service.pipeline()["value"]["time_slice_seconds"]
        self.slice_context.value = {"deadline": time.monotonic() + duration, "yielded": False}
        try:
            return self.run(identity)
        finally:
            del self.slice_context.value

    def cleanup(self, identity):
        cleanup.run(self.state, identity, self.resources)

    def _run_guarded(self, identity):
        job = self.state.job(identity)
        if job["state"] in TERMINAL | {"paused", "waiting_credentials", "needs_review"}:
            return job
        try:
            lib = self.state.library(job["lake_id"])
            with FileLock(lib.cache / ".daily-run.lock", timeout=0.1):
                return self._run(lib, identity)
        except UpdateError as error:
            current = self.state.job(identity)
            if current["state"] in {"paused", "cancelled"}:
                return current
            if error.code == "CANCELLED":
                state = "queued" if self.stop.is_set() or getattr(self.slice_context, "value", {}).get("yielded") else "paused"
            elif error.code == "UPDATE_CREDENTIAL_REQUIRED":
                state = "waiting_credentials"
            elif error.code == "UPDATE_SPACE":
                state = "waiting_space"
            elif error.code in {"UPDATE_NETWORK", "UPDATE_REMOTE_ERROR"}:
                state = "waiting_retry"
            else:
                state = "needs_review"
            self.state.update(
                identity,
                state=state,
                error_code="UPDATE_INTERRUPTED" if state == "queued" else error.code,
                error_message="Update service stopped; saved work will resume automatically"
                if state == "queued" else str(error),
                retry_at=0 if state == "queued" else time.time() + max(60, error.retry_after),
            )
        except Exception as error:
            current = self.state.job(identity)
            if current["state"] in {"paused", "cancelled"}:
                return current
            if isinstance(error, RuntimeError) and str(error).startswith("另一个进程正在使用此工作区"):
                if current["state"] != "running":
                    self.state.update(
                        identity,
                        state="waiting_retry",
                        retry_at=time.time() + 30,
                        error_code="UPDATE_BUSY",
                        error_message="Another producer owns this lake; waiting",
                    )
                return self.state.job(identity)
            # Raw exceptions may include request URLs from third-party libraries. Keep them private.
            code = "UPDATE_IO" if isinstance(error, OSError) else "UPDATE_INTEGRITY"
            import errno
            import traceback

            with (self.state.root / "errors.jsonl").open("a", encoding="utf-8") as report:
                report.write(
                    json.dumps(
                        {
                            "at": now(),
                            "job_id": identity,
                            "code": code,
                            "exception": type(error).__name__,
                            "frames": [
                                {
                                    "file": Path(frame.filename).name,
                                    "line": frame.lineno,
                                    "function": frame.name,
                                }
                                for frame in traceback.extract_tb(error.__traceback__)
                            ],
                        }
                    )
                    + "\n"
                )
            self.state.update(
                identity,
                state="waiting_space"
                if isinstance(error, OSError) and error.errno == errno.ENOSPC
                else "needs_review",
                error_code=code,
                error_message="Archive or execution failed ("
                + type(error).__name__
                + "); durable checkpoints retained",
            )
        return self.state.job(identity)

    def _run(self, lib, identity):
        previous = self.state.claim(identity)
        if previous is None:
            return self.state.job(identity)
        job = self.state.job(identity)
        generation = previous["execution"]
        if job["cursor"].get("input_generation"):
            with online(lib) as (_, status):
                if status["generation"] != job["cursor"]["input_generation"]:
                    raise UpdateError(
                        "SOURCE_CHANGED", "The fixed local input belongs to another online generation"
                    )
        last_check, was_cancelled = 0.0, False
        control_lock = threading.Lock()
        slice_info = getattr(self.slice_context, "value", None)

        def cancelled():
            nonlocal last_check, was_cancelled
            if self.stop.is_set():
                return True
            if slice_info is not None and time.monotonic() >= slice_info["deadline"]:
                slice_info["yielded"] = True
                return True
            with control_lock:
                if time.monotonic() - last_check >= 0.1:
                    with self.state.db() as db:
                        current = db.execute(
                            "SELECT state,execution FROM jobs WHERE id=?", (identity,)
                        ).fetchone()
                    was_cancelled = (
                        current["state"] in {"paused", "cancelled"} or current["execution"] != generation
                    )
                    last_check = time.monotonic()
                return was_cancelled

        def check():
            nonlocal last_check
            with control_lock:
                last_check = 0
            if cancelled():
                raise UpdateError("CANCELLED", "Update paused at a safe boundary")

        check()
        interrupted = previous["state"] == "running" or previous.get("error_code") == "UPDATE_INTERRUPTED"
        if interrupted:
            self.state.progress(identity, recovery_count_delta=1, last_recovery_at=now(),
                                last_recovery_reason="worker_interrupted" if previous["state"] == "running"
                                else "service_restarted")
        self.state.progress(
            identity,
            phase="recovering",
            current_post_id=None,
            current_bytes=None,
            current_total_bytes=None,
            files=[],
            download_rate_bps=0,
            publish_rate_images_per_second=0,
            active_downloads=0,
            active_encodes=0,
            waiting_encode=0,
            downloaded_bytes_delta=0,
            metadata_bytes_delta=0,
        )
        # Recovery reads committed IDs; manifests and image verification are scoped to new batches.
        lib.recover()
        reconcile(self.state, lib, identity)
        job = self.state.job(identity)
        if job["cursor"].get("slice_execution", 0) != generation:
            cursor = {**job["cursor"], "slice_pages": 0, "slice_items": 0, "slice_execution": generation}
            self.state.update(identity, cursor=cursor)
            job = self.state.job(identity)
        site = self.site(self.state.lake(job["lake_id"])["site"])
        site.observer = lambda **values: self.state.progress(identity, **values)
        if not job["cursor"].get("initialized"):
            self.initialize(lib, job, site, check, cancelled)
        from .pipeline import Pipeline

        Pipeline(self, lib, self.state.job(identity), site, check, cancelled).run()
        job = self.state.job(identity)
        if job["state"] in {"paused", "cancelled"}:
            return job
        counts = job["counts"]
        if counts.get("failed", 0) or counts.get("needs_review", 0):
            with self.state.db() as db:
                exhausted = db.execute(
                    "SELECT 1 FROM items WHERE job_id=? AND state='failed' AND attempts>=8 LIMIT 1",
                    (identity,),
                ).fetchone()
            waiting = bool(counts.get("failed", 0)) and not counts.get("needs_review", 0) and not exhausted
            self.state.update(
                identity,
                state="waiting_retry" if waiting else "needs_review",
                retry_at=time.time() + 60,
                error_code="UPDATE_MEDIA_INCOMPLETE",
                error_message="Metadata published; media gaps remain",
            )
        elif (
            counts.get("pending", 0)
            or counts.get("pending_metadata", 0)
            or not job["cursor"].get("metadata_complete")
        ):
            raise UpdateError("UPDATE_NO_PROGRESS", "Pending records could not advance; checkpoints retained")
        else:
            self.finish(lib, job)
        return self.state.job(identity)

    def metadata_coverage(self, lib, job):
        cursor = job["cursor"]
        if not cursor.get("metadata_complete"):
            return
        if job["definition"]["range"]["kind"] == "new":
            with lib.writer_lock():
                old = lib.setting("update_new_metadata_cursor")
                if old is None or old == cursor["baseline"]:
                    lib.set_setting("update_new_metadata_cursor", cursor["upper"] - 1)
        with self.state.db() as db:
            db.execute(
                "INSERT OR IGNORE INTO coverage VALUES(?,?,?,?,?,?,?,?)",
                (
                    job["id"],
                    job["lake_id"],
                    json.dumps(job["definition"]["range"]),
                    1,
                    0,
                    job["counts"].get("unavailable", 0),
                    cursor["scope"],
                    now(),
                ),
            )

    def initialize(self, lib, job, site, check, cancelled):
        scope = job["definition"]["range"]
        kind = scope["kind"]
        cursor = {
            "initialized": True,
            "pages": 0,
            "slice_pages": 0,
            "slice_items": 0,
            "metadata_complete": False,
            "input_seq": None,
            "scope": "accessible_posts_at_request_time",
            "slice_execution": job["execution"],
        }
        if kind == "input":
            frozen = self.state.input(scope["input_id"])
            if frozen["state"] != "sealed":
                raise UpdateError("UPDATE_CONFLICT", "Input must be sealed")
            cursor.update(next_id=1, upper=2**63 - 1, input_id=frozen["id"], input_sha256=frozen["sha256"])
        elif kind == "ids":
            cursor.update(position=0, next_id=scope["ids"][0], upper=scope["ids"][-1] + 1)
        elif kind == "id_range":
            cursor.update(next_id=scope["start"], upper=scope["end"])
        elif kind == "new":
            after = scope.get("after_id")
            if after is None:
                after = lib.setting("update_new_metadata_cursor")
            if after is None and site.name == "danbooru":
                after = lib.setting("api_watermark")
            if after is None:
                raise UpdateError(
                    "UPDATE_BASELINE_REQUIRED",
                    "An explicit verified starting ID is required; maximum imported ID is not coverage",
                )
            cursor.update(next_id=after + 1, baseline=after, upper=None)
        elif kind == "tags":
            from .tag_query import initialize_cursor

            cursor = initialize_cursor(self, lib, job, site, scope, cursor)
        else:
            cursor.update(next_id=scope.get("start_id", 1), upper=scope.get("end_id"))
        if kind == "local":
            with online(lib) as (db, status):
                cursor["input_seq"] = int(status["served_seq"])
                cursor["input_generation"] = status["generation"]
                if cursor["upper"] is None:
                    cursor["upper"] = next(
                        db.execute("SELECT coalesce(max(post_id),0)+1 FROM post_versions")
                    )[0]
            lease(lib, job["id"], cursor["input_seq"])
        if cursor["upper"] is None or kind == "changes":
            params = {"limit": 1}
            if site.name == "gelbooru":
                params.update(page="dapi", s="post", q="index", json=1, tags="sort:id:desc")
            else:
                visibility = "deleted:all holds:all pending:all" if site.name == "yandere" else "status:any"
                params["tags"] = visibility + (
                    " order:change_desc" if kind == "changes" else " order:id_desc"
                )
            response = self.request_page(lib, job, site, params, cancelled)
            try:
                rows = site.parse(response)
                if kind == "changes":
                    cursor["change_through"] = max([int(r[1]["change"]) for r in rows] or [scope["after"]])
                    cursor["upper"] = cursor["upper"] or 2**63 - 1
                else:
                    cursor["upper"] = max([r[1]["id"] + 1 for r in rows] + [cursor["next_id"]])
                check()
                commit_page(self.state, lib, job, site, response, rows, set(), cursor)
            except UpdateError as e:
                commit_page(self.state, lib, job, site, response, [], set(), job["cursor"], error=e)
                raise
        else:
            key = stable_id("update-registration-v1", job["id"])
            with io_lock(self.state.root, lib), lib.writer_lock():
                if not lib.committed_key(key):
                    batch = Batch(
                        lib,
                        key,
                        {
                            "kind": "update_registration",
                            "update_job_id": job["id"],
                            "update_role": "register",
                            "definition": job["definition"],
                            "cursor": cursor,
                        },
                    )
                    batch.commit(
                        settings={
                            checkpoint_key(job["id"]): {"definition": job["definition"], "cursor": cursor}
                        }
                    )
        reconcile(self.state, lib, job["id"])

    def page(self, lib, job, site, check, cancelled, publication=None):
        self.state.progress(job["id"], phase="metadata", current_post_id=None)
        scope, cursor = job["definition"]["range"], dict(job["cursor"])
        kind = scope["kind"]
        if kind == "tags":
            from .tag_query import page

            return page(self, lib, job, site, check, cancelled, publication)
        size = site.capabilities()["page_size"]
        size = min(size, job["definition"]["item_budget"] - cursor.get("slice_items", 0))
        ids = None
        if kind == "input":
            size = 1 if site.name == "gelbooru" else size
            with self.state.db() as db:
                ids = [
                    r[0]
                    for r in db.execute(
                        "SELECT post_id FROM input_ids WHERE input_id=? AND post_id>=? ORDER BY post_id LIMIT ?",
                        (scope["input_id"], cursor["next_id"], size),
                    )
                ]
        elif kind == "ids":
            size = 1 if site.name == "gelbooru" else size
            ids = scope["ids"][cursor["position"] : cursor["position"] + size]
        elif kind == "local":
            size = 1 if site.name == "gelbooru" else size
            with online(lib) as (db, _):
                seq = cursor["input_seq"]
                # Bound scanned IDs first; filters cannot turn one request into a full-lake scan.
                rows = list(
                    db.execute(
                        "SELECT p.post_id,p.asset_id,o.observed_at FROM post_versions p JOIN observations o USING(row_id) "
                        "WHERE p.post_id>=? AND p.post_id<? AND p.valid_from<=? "
                        "AND (p.valid_until IS NULL OR p.valid_until>?) ORDER BY p.post_id LIMIT ?",
                        (cursor["next_id"], cursor["upper"], seq, seq, size),
                    )
                )
            ids = []
            for pid, aid, observed in rows:
                if scope.get("missing_media") and aid is not None:
                    continue
                if (
                    scope.get("observed_before")
                    and observed
                    and timestamp(observed) >= timestamp(scope["observed_before"])
                ):
                    continue
                ids.append(pid)
            if rows:
                cursor["next_id"] = rows[-1][0] + 1
            else:
                cursor["metadata_complete"] = True
        if job["cursor"]["next_id"] >= cursor["upper"] or ids == []:
            if kind == "local" and not cursor["metadata_complete"]:
                cursor["slice_pages"] += 1
                cursor["pages"] += 1
                if cursor["next_id"] >= cursor["upper"]:
                    cursor["metadata_complete"] = True
            else:
                cursor["metadata_complete"] = True
            with publication or nullcontext():
                self.save_cursor(lib, job, cursor)
            return
        after = (
            scope.get("after")
            if kind == "changes"
            else int(timestamp(scope["start"]).timestamp()) - 1
            if kind == "updated" and site.name == "gelbooru"
            else None
        )
        params = site.params(
            job["cursor"]["next_id"],
            cursor["upper"],
            size,
            ids=ids,
            change_after=after,
            change_through=cursor.get("change_through"),
        )
        if kind == "created" and site.name in {"danbooru", "yandere"}:
            from datetime import timedelta

            start = (timestamp(scope["start"]) - timedelta(days=1)).date().isoformat()
            end = (timestamp(scope["end"]) + timedelta(days=1)).date().isoformat()
            params["tags"] += f" date:{start}..{end}"
        response = self.request_page(lib, job, site, params, cancelled)
        try:
            rows = site.parse(response)
            received = [r[1]["id"] for r in rows]
            if (
                len(rows) > size
                or received != sorted(received)
                or (ids is not None and set(received) - set(ids))
                or any(i < job["cursor"]["next_id"] or i >= cursor["upper"] for i in received)
            ):
                raise UpdateError(
                    "UPDATE_PAGE_INVALID", "API ignored range/order or returned an overlapping page"
                )
            selected = set(received)
            if kind in {"created", "updated"}:
                field = "created_at" if kind == "created" else "updated_at"
                selected = set()
                for ordinal, (_, record) in enumerate(rows):
                    value = site.normalize(record, "range-check", ordinal, now()).get(field)
                    if value is None:
                        raise UpdateError(
                            "UPDATE_RESPONSE_INVALID", "Date cannot be established for a returned post"
                        )
                    if timestamp(scope["start"]) <= timestamp(value) < timestamp(scope["end"]):
                        selected.add(record["id"])
            if kind == "changes" and any(
                not scope["after"] < int(r[1].get("change", -1)) <= cursor["change_through"] for r in rows
            ):
                raise UpdateError("UPDATE_PAGE_INVALID", "API ignored the frozen change range")
            if kind == "ids":
                cursor["position"] += len(ids)
                cursor["metadata_complete"] = cursor["position"] == len(scope["ids"])
                cursor["next_id"] = (
                    scope["ids"][cursor["position"]] if not cursor["metadata_complete"] else cursor["upper"]
                )
            elif kind == "input":
                cursor["next_id"] = ids[-1] + 1
            elif kind != "local":
                cursor["metadata_complete"] = not received
                if received:
                    cursor["next_id"] = received[-1] + 1
            if cursor["next_id"] >= cursor["upper"]:
                cursor["metadata_complete"] = True
            cursor["pages"] += 1
            cursor["slice_pages"] += 1
            cursor["slice_items"] += len(selected)
            cursor["replay_saved_response"] = False
            tag_types = categories(self.state, lib, job, site, rows, selected, cancelled)
            self.state.progress(
                job["id"], phase="publishing_metadata", metadata_bytes_delta=len(response.body)
            )
            check()
            self.publish_page(
                lib,
                job,
                site,
                response,
                rows,
                selected,
                cursor,
                expected_ids=ids,
                tag_types=tag_types,
                publication=publication,
            )
        except UpdateError as e:
            if e.code != "CANCELLED":
                self.publish_page(
                    lib, job, site, response, [], set(), job["cursor"], error=e, publication=publication
                )
            raise

    def publish_page(self, lib, job, site, response, rows, selected, cursor, publication=None, **options):
        with publication or nullcontext():
            commit_page(self.state, lib, job, site, response, rows, selected, cursor, **options)
            reconcile(self.state, lib, job["id"])

    def request_page(self, lib, job, site, params, cancelled):
        if not job["cursor"].get("replay_saved_response"):
            return site.request(params, cancelled)
        from ..util import file_hash

        with lib.journal() as journal:
            rows = journal.execute(
                "SELECT c.manifest_json FROM commits c JOIN update_run_batches b USING(seq,batch_id) "
                "WHERE b.job_id=? ORDER BY c.seq DESC LIMIT 32",
                (job["id"],),
            ).fetchall()
        for row in rows:
            manifest = json.loads(row[0])
            source = manifest["source"]
            request = source.get("request", {})
            if (
                source.get("update_role") != "error"
                or request.get("status") != 200
                or request.get("parameters") != params
            ):
                continue
            path = lib.root / "segments" / manifest["batch_id"] / "response_body.bin"
            if file_hash(path) != source["response_sha256"]:
                raise UpdateError("UPDATE_INTEGRITY", "Saved response checksum mismatch")
            return Response(
                path.read_bytes(), 200, {**request, "replayed_observed_at": source["observed_at"]}
            )
        raise UpdateError(
            "UPDATE_REPLAY_UNAVAILABLE", "No matching saved successful response; use retry to fetch again"
        )

    def save_cursor(self, lib, job, cursor):
        key = stable_id("update-progress-v1", job["id"], cursor)
        with io_lock(self.state.root, lib), lib.writer_lock():
            if not lib.committed_key(key):
                batch = Batch(
                    lib,
                    key,
                    {
                        "kind": "update_progress",
                        "update_job_id": job["id"],
                        "update_role": "progress",
                        "definition": job["definition"],
                        "cursor": cursor,
                    },
                )
                batch.commit(
                    settings={checkpoint_key(job["id"]): {"definition": job["definition"], "cursor": cursor}}
                )
        reconcile(self.state, lib, job["id"])

    def retry_metadata(self, lib, job, item, site, check, cancelled, publication=None):
        if job["definition"]["range"]["kind"] == "tags":
            from .tag_query import validate_context

            validate_context(self.state, lib, site, job["cursor"])
        response = site.request(
            site.params(item["post_id"], item["post_id"] + 1, 1, ids=[item["post_id"]]), cancelled
        )
        try:
            rows = site.parse(response)
            if any(r[1]["id"] != item["post_id"] for r in rows):
                raise UpdateError("UPDATE_PAGE_INVALID", "Retry returned an unexpected post")
            tag_types = categories(self.state, lib, job, site, rows, {item["post_id"]}, cancelled)
            check()
            selected = {item["post_id"]}
            if job["definition"]["range"]["kind"] == "tags":
                from .tag_query import matches, tags_of

                selected = {r["id"] for _, r in rows if matches(job["definition"]["range"]["query"], tags_of(r, site.name))}
            self.publish_page(
                lib,
                job,
                site,
                response,
                rows,
                selected,
                job["cursor"],
                expected_ids=[item["post_id"]],
                retry=True,
                metadata_ids={r[1]["id"] for r in rows} if job["definition"]["range"]["kind"] == "tags" else None,
                tag_types=tag_types,
                publication=publication,
            )
        except UpdateError as error:
            if error.code == "CANCELLED":
                raise
            self.publish_page(
                lib,
                job,
                site,
                response,
                [],
                set(),
                job["cursor"],
                error=error,
                retry=True,
                publication=publication,
            )
            raise

    def publish_images(self, lib, job, results):
        self.state.progress(job["id"], phase="publishing_media", current_post_id=None)
        commit_media(self.state, lib, job, results, job["cursor"])
        reconcile(self.state, lib, job["id"])
        for result in results:
            if result.get("ready_path"):
                path = Path(result["ready_path"])
                if path.parent == lib.cache / "updates" / job["id"]:
                    path.unlink(missing_ok=True)
                    path.with_suffix(".json").unlink(missing_ok=True)
                    path.with_suffix(".downloaded").unlink(missing_ok=True)
                    path.with_suffix(".download.json").unlink(missing_ok=True)

    def finish(self, lib, job):
        exclusions = job["counts"].get("unavailable", 0)
        cursor = {**job["cursor"], "completed_at": now()}
        self.save_cursor(lib, job, cursor)
        with self.state.db() as db:
            db.execute(
                "INSERT OR REPLACE INTO coverage VALUES(?,?,?,?,?,?,?,?)",
                (
                    job["id"],
                    job["lake_id"],
                    json.dumps(job["definition"]["range"]),
                    1,
                    int(job["definition"]["media"]["profile"] != "metadata_only"),
                    exclusions,
                    cursor["scope"],
                    now(),
                ),
            )
        lease(lib, job["id"])
        self.state.update(job["id"], state="completed_with_exclusions" if exclusions else "completed")
        self.state.progress(job["id"], phase="completed", current_post_id=None, download_rate_bps=0)
        directory = contained(lib.cache / "updates", job["id"])
        try:
            directory.rmdir()
        except OSError:
            # Failed/review items may still own resumable files; remove only empty task directories.
            pass

    def serve(self, ready=None):
        background_io = False
        import os

        if os.name == "nt":
            import ctypes
            from ctypes import wintypes

            kernel = ctypes.WinDLL("kernel32", use_last_error=True)
            kernel.GetCurrentProcess.restype = wintypes.HANDLE
            kernel.SetPriorityClass.argtypes = [wintypes.HANDLE, wintypes.DWORD]
            background_io = bool(kernel.SetPriorityClass(kernel.GetCurrentProcess(), 0x00100000))
        try:
            with (
                FileLock(self.state.root / "runner.lock", timeout=0.1),
                # Three execution lanes plus one independent bounded cleanup lane.
                ThreadPoolExecutor(max_workers=4) as pool,
            ):
                try:
                    if ready is not None and not ready():
                        return
                    pool.submit(self.collections.prepare_browse_indexes)
                    self.schedule(pool, background_io)
                finally:
                    # Stop all work before ThreadPoolExecutor waits, including coordinator failures.
                    self.stop.set()
        finally:
            if background_io:
                kernel.SetPriorityClass(kernel.GetCurrentProcess(), 0x00200000)

    def schedule(self, pool, background_io):
        active = {}
        cleaning = None
        while not self.stop.is_set():
            self.refresh_settings()
            self.state.tick_schedules()
            from ..collections.schedules import Schedules

            Schedules(self.collections.service).tick()
            for lake, (future, identity, execution, family) in list(active.items()):
                if future.done():
                    del active[lake]
                    try:
                        future.result()
                    except Exception:
                        if family == "collection":
                            self.collections.update_state(identity, "waiting_retry", "COLLECTION_EXECUTOR_FAILED", retry=60)
                        else:
                            dispatch.failed(self.state, identity, execution)
            if cleaning is not None and cleaning[0].done():
                future, identity, _ = cleaning
                cleaning = None
                try:
                    future.result()
                except Exception:
                    cleanup.record(self.state, identity, error_code="UPDATE_CLEANUP_INTERRUPTED", delay=30)
            # Give a due cleanup its lake before admitting that lake's next task.
            # Continuous update backlog must not starve terminal spool reclamation.
            if cleaning is None and (pending := dispatch.next_cleanup(self.state, active)):
                identity = pending["job_id"]
                cleaning = (pool.submit(self.cleanup, identity), identity, pending["lake_id"])
            excluded = {*active, *([cleaning[2]] if cleaning else [])}
            for job in dispatch.candidates(
                self.state, excluded, self.resources.config["active_lakes"] - len(active)
            ):
                dispatch.submitted(self.state, job["lake_id"])
                execute = self.collections.run if job["family"] == "collection" else self.run_slice
                active[job["lake_id"]] = (pool.submit(execute, job["id"]), job["id"], job["execution"], job["family"])
            atomic_json(
                self.state.root / "heartbeat.json",
                {
                    "protocol_version": 1,
                    "at": now(),
                    "active_lakes": list(active),
                    "cleanup_job": cleaning[1] if cleaning else None,
                    "background_io": background_io,
                },
            )
            self.stop.wait(1)



def serve(root):
    runner = Runner(State(root))
    try:
        runner.serve()
    except KeyboardInterrupt:
        runner.stop.set()
