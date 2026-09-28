"""Bounded stage scheduling over the existing durable queue and archive checkpoints."""

from concurrent.futures import ThreadPoolExecutor
import threading
import time

from .media import ImageSessions, download, encode_download, paths, reserve
from .sites import UpdateError
from .telemetry import Telemetry
from .settings import MIB
from .resources import Reservation
from .publication import Publications
from .candidates import eligible

API_RETRY_SECONDS = 60


class Pipeline:
    def __init__(self, runner, lib, job, site, check, cancelled):
        self.runner, self.lib, self.job, self.site = runner, lib, job, site
        self.state, self.resources = runner.state, runner.resources
        self.base_check, self.base_cancelled = check, cancelled
        self.aborting = threading.Event()
        self.publisher = Publications()
        self.publication = self.publisher.lock
        self.stats = Telemetry(self.state, self.state.job(job["id"]))
        self.sessions = ImageSessions(site.name, runner.image_http)
        self.downloads, self.encodes, self.waiting, self.ready, self.leases = {}, {}, [], [], {}
        self.download_slots = {}
        self.scan = None
        self.first_ready = None
        self.coverage_written = False
        self.deferred = {}
        previous = self.state.job(job["id"]).get("telemetry") or {}
        self.metadata_retry_at = previous.get("metadata_retry_at") or 0
        self.metadata_error_code = previous.get("metadata_error_code")
        self.metadata_error_message = previous.get("metadata_error_message")

    def cancelled(self):
        return self.aborting.is_set() or self.base_cancelled()

    def check(self):
        self.base_check()
        if self.aborting.is_set():
            raise UpdateError("CANCELLED", "Update paused")

    def scan_page(self, retry=None):
        self.site.observer = self.stats.add
        started = time.perf_counter()
        try:
            return self.scan_page_inner(retry)
        finally:
            self.stats.add(metadata_seconds_delta=time.perf_counter() - started)
            self.site.observer = None

    def scan_page_inner(self, retry=None):
        # HTTP never holds the checkpoint coordinator; only commit + reconciliation is serialized.
        self.check()
        current = self.state.job(self.job["id"])
        if retry:
            self.runner.retry_metadata(
                self.lib, current, retry, self.site, self.check, self.cancelled, publication=self.publication
            )
        else:
            self.runner.page(
                self.lib, current, self.site, self.check, self.cancelled, publication=self.publication
            )
            latest = self.state.job(self.job["id"])["cursor"]
            if not latest.get("metadata_complete") and all(
                latest.get(k) == current["cursor"].get(k) for k in ("pages", "next_id", "position")
            ):
                raise UpdateError(
                    "UPDATE_NO_PROGRESS", "Archived page did not advance the execution checkpoint"
                )

    def result(self, item, value):
        value.update(
            post_id=item["post_id"], observation_id=item["observation_id"], attempt=item["attempts"] + 1
        )
        self.ready.append(value)
        self.stats.add(item["post_id"], phase="ready")
        if self.first_ready is None:
            self.first_ready = time.monotonic()

    def collect(self, futures, *, downloading, stopping=False):
        for future, item in list(futures.items()):
            if not future.done():
                continue
            del futures[future]
            if downloading:
                self.download_slots.pop(item["post_id"]).release()
            if future.cancelled():
                continue
            try:
                result = future.result()
            except UpdateError as error:
                if error.code == "UPDATE_SPACE" and not stopping:
                    # Header corrections or output growth can temporarily exceed
                    # admission. Keep durable bytes without consuming an attempt.
                    self.leases.pop(item["post_id"]).release()
                    self.deferred[item["post_id"]] = time.monotonic() + 1
                    self.stats.forget(item["post_id"])
                    continue
                if error.code in {"CANCELLED", "UPDATE_SPACE"}:
                    if not stopping:
                        raise
                    continue
                result = {"state": "needs_review", "reason": error.code}
            if downloading and result["state"] == "downloaded":
                if not stopping:
                    self.waiting.append((item, result))
                    self.stats.add(item["post_id"], phase="waiting_encode")
            else:
                self.result(item, result)

    def download_one(self, item, slot):
        try:
            return download(
                self.lib,
                self.job,
                item,
                self.site,
                self.resources,
                self.cancelled,
                self.sessions,
                self.stats.callback(item["post_id"]),
            )
        finally:
            slot.release()

    def collect_publication(self, *, wait=False):
        completed = self.publisher.collect(wait=wait)
        if completed is None:
            return
        kind, results = completed
        if kind == "coverage":
            self.coverage_written = True
        else:
            self.stats.published_results(results)
            for result in results:
                self.leases.pop(result["post_id"]).release()
        self.deferred.clear()

    def publish_results(self, results):
        started = time.perf_counter()
        try:
            current = self.state.job(self.job["id"])
            self.runner.publish_images(self.lib, current, results)
        finally:
            self.stats.add(publish_seconds_delta=time.perf_counter() - started)

    def publish(self, force=False):
        if not self.ready or self.publisher.future is not None:
            return False
        config = self.resources.config
        due = (
            force
            or len(self.ready) >= config["publish_items"]
            or sum(r.get("stored_bytes", 0) for r in self.ready) >= config["publish_mib"] * MIB
            or time.monotonic() - self.first_ready >= config["publish_interval_seconds"]
            or len(self.leases) >= config["buffer_images"]
        )
        if not due:
            return False
        count = len(self.ready) if force else min(len(self.ready), config["publish_items"])
        results, self.ready = self.ready[:count], self.ready[count:]
        if not self.ready:
            self.first_ready = None
        for result in results:
            self.stats.add(result["post_id"], phase="publishing_media")
        return self.publisher.submit("media", self.publish_results, results, results=results)

    def eligible(self, limit):
        return eligible(self.state, self.job["id"], limit)

    def collect_scan(self):
        if self.scan is None or not self.scan.done():
            return
        completed, self.scan = self.scan, None
        try:
            completed.result()
        except UpdateError as error:
            if error.code not in {"UPDATE_NETWORK", "UPDATE_REMOTE_ERROR"}:
                raise
            self.metadata_retry_at = time.time() + max(API_RETRY_SECONDS, error.retry_after)
            self.metadata_error_code, self.metadata_error_message = error.code, str(error)
            self.stats.add(metadata_retries_delta=1)
        else:
            self.metadata_retry_at = 0
            self.metadata_error_code = self.metadata_error_message = None
        self.flush(force=True)

    def flush(self, **kwargs):
        if not kwargs.get("force") and time.monotonic() - self.stats.last_flush < 0.5:
            return
        self.stats.flush(scanning=self.scan is not None, metadata_retry_at=self.metadata_retry_at,
                         metadata_error_code=self.metadata_error_code,
                         metadata_error_message=self.metadata_error_message,
                         resources=self.resources.snapshot(), waiting_staging=len(self.deferred), **kwargs)

    def run(self):
        scanner = ThreadPoolExecutor(max_workers=1, thread_name_prefix="lake-metadata")
        transfers = ThreadPoolExecutor(max_workers=16, thread_name_prefix="lake-download")
        encoding = ThreadPoolExecutor(max_workers=16, thread_name_prefix="lake-encode")
        self.site.observer = self.stats.add
        failure = None
        try:
            while True:
                self.check()
                self.runner.refresh_settings()
                config = self.resources.config
                self.collect_scan()
                self.collect_publication()
                self.collect(self.downloads, downloading=True)
                self.collect(self.encodes, downloading=False)
                self.publish()
                current = self.state.job(self.job["id"])
                cursor, spec = current["cursor"], current["definition"]
                complete = bool(cursor.get("metadata_complete"))
                if complete and not self.coverage_written and self.scan is None and self.publisher.future is None:
                    self.publisher.submit("coverage", self.runner.metadata_coverage, self.lib, current)
                if (
                    not complete
                    and self.scan is None
                    and (
                        cursor.get("slice_pages", 0) >= spec["page_budget"]
                        or cursor.get("slice_items", 0) >= spec["item_budget"]
                    )
                ):
                    self.state.update(
                        self.job["id"],
                        state="paused",
                        error_code="UPDATE_BUDGET",
                        error_message="Run budget reached; resume continues the same range",
                    )
                    break

                while self.waiting and len(self.encodes) < config["encode_concurrency"]:
                    item, downloaded = self.waiting.pop(0)
                    future = encoding.submit(
                        encode_download,
                        self.lib,
                        self.job,
                        item,
                        downloaded,
                        self.resources,
                        self.cancelled,
                        self.stats.callback(item["post_id"]),
                    )
                    self.encodes[future] = item

                items = self.eligible(config["buffer_images"] + len(self.leases) + 16)
                retry = next(
                    (
                        i
                        for i in items
                        if i["state"] == "pending_metadata" and i["post_id"] not in self.leases
                    ),
                    None,
                )
                can_scan = not complete and (
                    config["scan_mode"] == "metadata_first"
                    or current["counts"].get("pending", 0) < config["metadata_prefetch_records"]
                )
                if self.scan is None and time.time() >= self.metadata_retry_at and (retry or can_scan):
                    self.scan = scanner.submit(self.scan_page, retry)

                space_blocked = False
                if complete or config["scan_mode"] == "pipeline":
                    width = config["sites"][self.site.name]["download_concurrency"]
                    for item in items:
                        if len(self.downloads) >= width or len(self.leases) >= config["buffer_images"]:
                            break
                        if item["state"] == "pending_metadata" or item["post_id"] in self.leases:
                            continue
                        if self.deferred.get(item["post_id"], 0) > time.monotonic():
                            space_blocked = True
                            continue
                        slot = self.resources.try_download(self.site.name)
                        if slot is None:
                            break
                        directory, key = paths(self.lib, self.job, item)
                        try:
                            lease = reserve(
                                self.lib,
                                self.job,
                                item,
                                self.resources,
                                self.site,
                                self.sessions.injected is not None,
                            )
                        except UpdateError as error:
                            slot.release()
                            if error.code != "UPDATE_RESOURCE_LIMIT":
                                raise
                            self.leases[item["post_id"]] = Reservation(self.resources, (directory, key))
                            self.result(item, {"state": "needs_review", "reason": error.code})
                            continue
                        except Exception:
                            slot.release()
                            raise
                        if lease is None:
                            slot.release()
                            space_blocked = True
                            self.deferred[item["post_id"]] = time.monotonic() + 1
                            continue
                        self.deferred.pop(item["post_id"], None)
                        self.leases[item["post_id"]] = lease
                        self.download_slots[item["post_id"]] = slot
                        self.stats.add(item["post_id"], phase="queued", queue_state=item["state"])
                        future = transfers.submit(self.download_one, item, slot)
                        self.downloads[future] = item

                idle = not self.downloads and not self.encodes and not self.waiting
                if (idle or space_blocked) and self.ready and self.publish(force=True):
                    # Publication releases staging leases. Recheck admission next turn
                    # before treating the previous allocation failure as disk pressure.
                    space_blocked = False
                self.flush()
                if idle and not self.ready and self.scan is None and self.publisher.future is None:
                    if space_blocked:
                        if not self.resources.reservations:
                            raise UpdateError(
                                "UPDATE_SPACE", "Waiting for SSD spool space; staged files remain resumable"
                            )
                    elif complete and not items and self.coverage_written:
                        break
                self.aborting.wait(0.03)
        except Exception as error:
            failure = error
            raise
        finally:
            self.aborting.set()
            scanner.shutdown(wait=True, cancel_futures=True)
            transfers.shutdown(wait=True, cancel_futures=True)
            encoding.shutdown(wait=True, cancel_futures=True)
            try:
                # Completed stages have durable receipts even if a later stage could not publish.
                self.collect(self.downloads, downloading=True, stopping=True)
                self.collect(self.encodes, downloading=False, stopping=True)
                self.collect_publication(wait=True)
                if failure is None or isinstance(failure, UpdateError) and failure.code == "CANCELLED":
                    self.publish(force=True)
                    self.collect_publication(wait=True)
            finally:
                self.publisher.close()
                for lease in self.leases.values():
                    lease.release()
                for slot in self.download_slots.values():
                    slot.release()
                self.sessions.close()
                self.site.observer = None
                self.flush(stopping="paused" if self.base_cancelled() else "idle", force=True)
