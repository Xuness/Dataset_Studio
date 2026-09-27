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
            device = self.devices.setdefault(directory.parent, directory.parent.stat().st_dev)
            used = 0
            for root in self.roots:
                for path in root.glob("*/*"):
                    if path.suffix not in {".ready", ".downloaded", ".partial", ".tmp", ".json"}:
                        continue
                    stem = path.name.split(".")[0]
                    if (path.parent, stem) in self.reservations or (path.parent, stem) == identity:
                        continue
                    try:
                        used += path.stat().st_size
                    except FileNotFoundError:
                        pass
            existing = sum(p.stat().st_size for p in directory.glob(key + ".*") if p.is_file())
            need = existing + (0 if prepared else required_bytes or self.max_download_bytes * 4)
            if need > self.spool_bytes:
                raise UpdateError(
                    "UPDATE_RESOURCE_LIMIT", "One image's staging allocation exceeds the SSD spool budget"
                )
            if used + sum(self.reservations.values()) + need > self.spool_bytes:
                return None
            reserved_here = sum(
                v for (folder, _), v in self.reservations.items() if self.devices[folder.parent] == device
            )
            if not prepared and shutil.disk_usage(directory).free < self.reserve_bytes + need + reserved_here:
                return None
            self.reservations[identity] = need
            return Reservation(self, identity)

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
