"""Bounded stage scheduling over the existing durable queue and archive checkpoints."""

from concurrent.futures import ThreadPoolExecutor
import threading
import time

from .media import ImageSessions, download, encode_download, paths, reserve
from .sites import UpdateError
from .telemetry import Telemetry
from .settings import MIB
from .resources import Reservation


class Pipeline:
    def __init__(self, runner, lib, job, site, check, cancelled):
        self.runner, self.lib, self.job, self.site = runner, lib, job, site
        self.state, self.resources = runner.state, runner.resources
        self.base_check, self.base_cancelled = check, cancelled
        self.aborting, self.publication = threading.Event(), threading.Lock()
        self.stats = Telemetry(self.state, self.state.job(job["id"]))
        self.sessions = ImageSessions(site.name, runner.image_http)
        self.downloads, self.encodes, self.waiting, self.ready, self.leases = {}, {}, [], [], {}
        self.download_slots = {}
        self.scan = None
        self.first_ready = None
        self.coverage_written = False

    def cancelled(self):
        return self.aborting.is_set() or self.base_cancelled()

    def check(self):
        self.base_check()
        if self.aborting.is_set():
            raise UpdateError("CANCELLED", "Update paused")

    def scan_page(self, retry=None):
        self.site.observer = self.stats.add
        try:
            return self.scan_page_inner(retry)
        finally:
            self.site.observer = None

    def scan_page_inner(self, retry=None):
        # HTTP never holds the checkpoint coordinator; only commit + reconciliation is serialized.
        self.check()
        started = time.perf_counter()
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
        self.stats.add(metadata_seconds_delta=time.perf_counter() - started)

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

    def publish(self, force=False):
        if not self.ready:
            return False
        config = self.resources.config
        due = (
            force
            or len(self.ready) >= config["publish_items"]
            or sum(r.get("stored_bytes", 0) for r in self.ready) >= config["publish_mib"] * MIB
            or time.monotonic() - self.first_ready >= config["publish_interval_seconds"]
            or len(self.leases) >= config["buffer_images"]
        )
        if not due or not self.publication.acquire(blocking=False):
            return False
        try:
            started = time.perf_counter()
            results = self.ready
            current = self.state.job(self.job["id"])
            self.runner.publish_images(self.lib, current, results)
            self.stats.add(publish_seconds_delta=time.perf_counter() - started)
            self.stats.published_results(results)
            self.ready, self.first_ready = [], None
            for result in results:
                self.leases.pop(result["post_id"]).release()
            return True
        finally:
            self.publication.release()

    def eligible(self, limit):
        with self.state.db() as db:
            return [
                dict(r)
                for r in db.execute(
                    "SELECT * FROM items WHERE job_id=? AND state IN ('pending','failed','pending_metadata') "
                    "AND retry_at<=? AND attempts<8 ORDER BY post_id LIMIT ?",
                    (self.job["id"], time.time(), limit),
                )
            ]

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
                if self.scan is not None and self.scan.done():
                    self.scan.result()
                    self.scan = None
                self.collect(self.downloads, downloading=True)
                self.collect(self.encodes, downloading=False)
                self.publish()
                current = self.state.job(self.job["id"])
                cursor, spec = current["cursor"], current["definition"]
                complete = bool(cursor.get("metadata_complete"))
                if complete and not self.coverage_written and self.scan is None:
                    with self.publication:
                        self.runner.metadata_coverage(self.lib, current)
                    self.coverage_written = True
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
                if self.scan is None and (retry or can_scan):
                    self.scan = scanner.submit(self.scan_page, retry)

                space_blocked = False
                if complete or config["scan_mode"] == "pipeline":
                    width = config["sites"][self.site.name]["download_concurrency"]
                    for item in items:
                        if len(self.downloads) >= width or len(self.leases) >= config["buffer_images"]:
                            break
                        if item["state"] == "pending_metadata" or item["post_id"] in self.leases:
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
                            break
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
                self.stats.flush(scanning=self.scan is not None)
                if idle and not self.ready and self.scan is None:
                    if space_blocked:
                        if not self.resources.reservations:
                            raise UpdateError(
                                "UPDATE_SPACE", "Waiting for SSD spool space; staged files remain resumable"
                            )
                    elif complete and not items:
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
                if failure is None or isinstance(failure, UpdateError) and failure.code == "CANCELLED":
                    self.publish(force=True)
            finally:
                for lease in self.leases.values():
                    lease.release()
                for slot in self.download_slots.values():
                    slot.release()
                self.sessions.close()
                self.site.observer = None
                self.stats.flush(stopping="paused" if self.base_cancelled() else "idle", force=True)
