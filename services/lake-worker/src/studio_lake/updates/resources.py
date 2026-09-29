"""Shared admission for staged bytes, decoded pixels, encoding slots and bandwidth."""

from contextlib import contextmanager
import copy
from pathlib import Path
import shutil
import threading
import time

from .settings import DEFAULTS, MIB
from .sites import UpdateError


class Reservation:
    def __init__(self, owner, key):
        self.owner, self.key = owner, key
        self.released = False

    def release(self):
        with self.owner.condition:
            if not self.released:
                self.owner.reservations.pop(self.key, None)
                self.owner.plans.pop(self.key, None)
                self.released = True
            self.owner.condition.notify_all()


class Resources:
    def __init__(self, max_download_bytes=None, spool_bytes=None, reserve_bytes=None):
        self.config = copy.deepcopy(DEFAULTS)
        self.overrides = {
            "max_download_bytes": max_download_bytes,
            "spool_bytes": spool_bytes,
            "reserve_bytes": reserve_bytes,
        }
        self.condition = threading.Condition()
        self.reservations = {}
        self.plans = {}
        self.roots = set()
        self.devices = {}
        self.encode_jobs = self.decode_bytes = 0
        self.download_jobs = {}
        self.bandwidth_next = 0.0

    def configure(self, config):
        with self.condition:
            if config["download_mib_per_second"] != self.config["download_mib_per_second"]:
                self.bandwidth_next = 0.0
            self.config = config
            self.condition.notify_all()

    def try_download(self, site):
        with self.condition:
            active = self.download_jobs.get(site, 0)
            if active >= self.config["sites"][site]["download_concurrency"]:
                return None
            self.download_jobs[site] = active + 1
            return DownloadSlot(self, site)

    @property
    def max_download_bytes(self):
        return (
            self.overrides["max_download_bytes"]
            if self.overrides["max_download_bytes"] is not None
            else self.config["max_download_mib"] * MIB
        )

    @property
    def spool_bytes(self):
        return (
            self.overrides["spool_bytes"]
            if self.overrides["spool_bytes"] is not None
            else self.config["spool_mib"] * MIB
        )

    @property
    def reserve_bytes(self):
        return (
            self.overrides["reserve_bytes"]
            if self.overrides["reserve_bytes"] is not None
            else self.config["reserve_mib"] * MIB
        )

    def try_reserve(self, directory, key, required_bytes=None, *, prepared=False):
        directory = Path(directory)
        directory.mkdir(parents=True, exist_ok=True)
        identity = (directory, key)
        with self.condition:
            if identity in self.reservations:
                return None
            self.roots.add(directory.parent)
            self.devices.setdefault(directory.parent, directory.parent.stat().st_dev)
            need = required_bytes if required_bytes is not None else (0 if prepared else self.max_download_bytes * 4)
            if not self._resize(identity, need, prepared=prepared):
                return None
            return Reservation(self, identity)

    def _actual(self):
        sizes = {}
        for root in self.roots:
            for path in root.glob("*/*"):
                if path.suffix not in {".ready", ".downloaded", ".partial", ".tmp", ".json"}:
                    continue
                identity = (path.parent, path.name.split(".")[0])
                try:
                    sizes[identity] = sizes.get(identity, 0) + path.stat().st_size
                except FileNotFoundError:
                    pass
        return sizes

    def _resize(self, identity, need, *, prepared=False):
        actual = self._actual()
        existing = actual.get(identity, 0)
        need = max(need, existing)
        if identity in self.reservations and need <= self.reservations[identity]:
            self.reservations[identity] = need
            self.condition.notify_all()
            return True
        if need > self.spool_bytes:
            raise UpdateError("UPDATE_RESOURCE_LIMIT", "One image's staging allocation exceeds the SSD spool budget")
        used = sum(max(size, self.reservations.get(key, 0)) for key, size in actual.items() if key != identity)
        used += sum(size for key, size in self.reservations.items() if key not in actual and key != identity)
        if used + need > self.spool_bytes:
            return False
        directory = identity[0]
        device = self.devices[directory.parent]
        promised = sum(max(0, size - actual.get(key, 0)) for key, size in self.reservations.items()
                       if key != identity and self.devices[key[0].parent] == device)
        # Existing bytes have already reduced disk_usage.free: reserve only future writes.
        growth = max(0, need - existing)
        if growth and shutil.disk_usage(directory).free < self.reserve_bytes + growth + promised:
            return False
        self.reservations[identity] = need
        self.condition.notify_all()
        return True

    def plan(self, directory, key, plan):
        with self.condition:
            self.plans[(directory, key)] = plan

    def download_size(self, directory, key, size):
        with self.condition:
            identity = (directory, key)
            plan = self.plans.get(identity)
            if plan is None:  # Standalone HTTP-range tests do not own a media lease.
                return
            if not self._resize(identity, plan.peak(size)):
                raise UpdateError("UPDATE_SPACE", "Waiting for download staging space; checkpoint retained")

    def stage_size(self, directory, key, needed):
        with self.condition:
            identity = (directory, key)
            if identity not in self.reservations:
                return
            if not self._resize(identity, needed):
                raise UpdateError("UPDATE_SPACE", "Waiting for encoding staging space; original retained")

    def snapshot(self):
        with self.condition:
            actual = self._actual()
            return {
                "staging_bytes": sum(actual.values()),
                "staging_reserved_bytes": sum(max(n, actual.get(key, 0)) for key, n in self.reservations.items())
                + sum(n for key, n in actual.items() if key not in self.reservations),
                "staging_limit_bytes": self.spool_bytes,
                "decode_reserved_bytes": self.decode_bytes,
                "decode_limit_bytes": self.config["decode_memory_mib"] * MIB,
            }

    def retire(self, directory):
        """Caller holds the terminal job's execution lock; no workers can use these promises."""
        with self.condition:
            for key in list(self.reservations):
                if key[0] == directory:
                    self.reservations.pop(key, None)
                    self.plans.pop(key, None)
            self.condition.notify_all()

    def check_staged_size(self, directory, key, needed):
        with self.condition:
            available = self.reservations.get((directory, key), self.max_download_bytes * 4)
            if needed > available:
                raise UpdateError(
                    "UPDATE_RESOURCE_LIMIT", "Encoded output exceeds its reserved SSD staging bytes"
                )

    @contextmanager
    def reservation(self, root):
        # Compatibility for standalone callers; the pipeline retains leases until publication.
        lease = self.try_reserve(root, "standalone-" + str(threading.get_ident()))
        if lease is None:
            raise UpdateError("UPDATE_SPACE", "Waiting for SSD spool space")
        try:
            yield
        finally:
            lease.release()

    @contextmanager
    def encoding(self, estimated_bytes, cancelled):
        with self.condition:
            while True:
                if cancelled():
                    raise UpdateError("CANCELLED", "Update paused")
                limit = self.config["decode_memory_mib"] * MIB
                if estimated_bytes > limit:
                    raise UpdateError(
                        "UPDATE_RESOURCE_LIMIT", "Image exceeds configured decoded-memory estimate"
                    )
                if (
                    self.encode_jobs < self.config["encode_concurrency"]
                    and self.decode_bytes + estimated_bytes <= limit
                ):
                    self.encode_jobs += 1
                    self.decode_bytes += estimated_bytes
                    break
                self.condition.wait(0.1)
        try:
            yield
        finally:
            with self.condition:
                self.encode_jobs -= 1
                self.decode_bytes -= estimated_bytes
                self.condition.notify_all()

    def bandwidth(self, length, cancelled):
        with self.condition:
            rate = self.config["download_mib_per_second"]
            if rate is None:
                return
            due = max(time.monotonic(), self.bandwidth_next)
            self.bandwidth_next = due + length / (rate * MIB)
        while time.monotonic() < due:
            if cancelled():
                raise UpdateError("CANCELLED", "Update paused")
            if self.config["download_mib_per_second"] is None:
                return
            time.sleep(max(0, min(0.1, due - time.monotonic())))


class DownloadSlot:
    def __init__(self, owner, site):
        self.owner, self.site, self.released = owner, site, False

    def release(self):
        with self.owner.condition:
            if not self.released:
                self.owner.download_jobs[self.site] -= 1
                self.released = True
                self.owner.condition.notify_all()
