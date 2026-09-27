"""One bounded per-job aggregator; transfer threads never overwrite each other's progress."""

from collections import deque
import threading
import time


class Telemetry:
    def __init__(self, state, job):
        self.state, self.identity = state, job["id"]
        self.lock = threading.Lock()
        old = job.get("telemetry") or {}
        self.base = {
            k: old.get(k, 0)
            for k in ("downloaded_bytes", "image_requests", "api_requests", "throttled_requests",
                      "resumed_requests", "transport_failures")
        }
        self.last_transfer_error = old.get("last_transfer_error")
        self.totals = {k: 0 for k in self.base}
        self.times = dict(old.get("timings_seconds") or {})
        self.files = {}
        self.published = 0
        self.samples = deque([(time.monotonic(), 0, 0)])
        self.last_flush = 0.0

    def add(self, post_id=None, **values):
        with self.lock:
            for key, value in values.items():
                if key == "last_transfer_error":
                    self.last_transfer_error = value
                elif key.endswith("_seconds_delta"):
                    name = key.removesuffix("_seconds_delta")
                    self.times[name] = self.times.get(name, 0) + value
                elif key.endswith("_delta"):
                    name = key.removesuffix("_delta")
                    self.totals[name] = self.totals.get(name, 0) + value
                elif post_id is not None and key != "current_post_id":
                    self.files.setdefault(post_id, {"post_id": post_id, "phase": "queued"})[key] = value

    def callback(self, post_id):
        return lambda **values: self.add(post_id, **values)

    def published_results(self, results):
        with self.lock:
            self.published += sum(r["state"] == "stored" for r in results)
            for result in results:
                self.files.pop(result["post_id"], None)

    def flush(self, *, scanning=False, stopping=None, force=False):
        stamp = time.monotonic()
        if not force and stamp - self.last_flush < 0.5:
            return
        with self.state.db() as db:
            row = db.execute(
                "SELECT n FROM counts WHERE job_id=? AND state='pending'", (self.identity,)
            ).fetchone()
            pending = row[0] if row else 0
        with self.lock:
            self.last_flush = stamp
            self.samples.append((stamp, self.totals["downloaded_bytes"], self.published))
            while len(self.samples) > 2 and self.samples[1][0] <= stamp - 30:
                self.samples.popleft()
            first = self.samples[0]
            seconds = max(stamp - first[0], 0.001)
            waiting_download = max(
                0, pending - sum(f.get("queue_state") == "pending" for f in self.files.values())
            )
            files = [
                {k: v for k, v in self.files[pid].items() if k != "queue_state"} for pid in sorted(self.files)
            ]
            if stopping:
                files = []
            downloads = sum(
                f["phase"] in {"rate_wait", "connecting", "downloading", "verifying"} for f in files
            )
            encodes = sum(f["phase"] == "processing_image" for f in files)
            waiting = sum(f["phase"] == "waiting_encode" for f in files)
            ready = sum(f["phase"] == "ready" for f in files)
            current = next((f for f in files if f["phase"] == "downloading"), files[0] if files else {})
            value = {
                **{k: self.base.get(k, 0) + v for k, v in self.totals.items()},
                "phase": stopping
                or ("pipeline" if files else "metadata" if scanning else "waiting_resources"),
                "current_post_id": current.get("post_id"),
                "current_bytes": current.get("current_bytes")
                if current.get("phase") == "downloading"
                else None,
                "current_total_bytes": current.get("current_total_bytes")
                if current.get("phase") == "downloading"
                else None,
                "download_rate_bps": 0
                if stopping
                else (self.totals["downloaded_bytes"] - first[1]) / seconds,
                "publish_rate_images_per_second": 0 if stopping else (self.published - first[2]) / seconds,
                "rate_window_seconds": seconds,
                "active_downloads": downloads,
                "waiting_download": pending if stopping else waiting_download,
                "active_encodes": encodes,
                "waiting_encode": waiting,
                "ready_images": ready,
                "metadata_active": scanning and not stopping,
                "files": files[:48],
                "timings_seconds": dict(self.times),
                "last_transfer_error": self.last_transfer_error,
            }
        self.state.progress(self.identity, **value)
