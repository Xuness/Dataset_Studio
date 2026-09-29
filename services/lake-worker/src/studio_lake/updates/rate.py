"""One host-wide API lane per site, shared by daemon, CLI and probes."""

from contextlib import contextmanager
from pathlib import Path
import time

from ..util import FileLock, atomic_json, read_json
from .sites import UpdateError

NOT_FOUND_WINDOW = 60
NOT_FOUND_THRESHOLD = 8
NOT_FOUND_COOLDOWN = 60


def image_not_found(root, site):
    """A burst of missing images may be a CDN outage, not thousands of deleted posts.

    Persist a bounded counter in the existing shared image lane. Already admitted
    downloads may finish; metadata, encoders and publication remain independent.
    """
    root = Path(root)
    marker = root / ("rate-image-" + site + ".json")
    with FileLock(root / ("rate-image-" + site + ".lock")):
        saved = read_json(marker) if marker.exists() else {}
        stamp = time.time()
        since = saved.get("not_found_since", 0)
        count = saved.get("not_found_count", 0) if 0 <= stamp - since < NOT_FOUND_WINDOW else 0
        if not count:
            since = stamp
        count += 1
        saved.update(not_found_since=since, not_found_count=count)
        if count >= NOT_FOUND_THRESHOLD:
            saved.update(cooldown_until=max(saved.get("cooldown_until", 0), stamp + NOT_FOUND_COOLDOWN),
                         not_found_count=0, not_found_since=stamp)
        atomic_json(marker, saved)


@contextmanager
def admission(root, site, cancelled, delay=1.1):
    root = Path(root)
    gate = FileLock(root / ("rate-" + site + ".lock"), timeout=0)
    while True:
        if cancelled():
            raise UpdateError("CANCELLED", "Update paused")
        try:
            gate.__enter__()
            break
        except RuntimeError:
            time.sleep(0.1)
    try:
        marker = root / ("rate-" + site + ".json")
        saved = read_json(marker) if marker.exists() else {}
        due = max(saved.get("next_at", 0), saved.get("cooldown_until", 0))
        while time.time() < due:
            if cancelled():
                raise UpdateError("CANCELLED", "Update paused")
            time.sleep(min(0.1, max(0, due - time.time())))
        started = time.time()
        try:
            yield
        finally:
            atomic_json(marker, {**saved, "next_at": started + delay})
    finally:
        gate.__exit__(None, None, None)


def wait_start(root, site, cancelled, requests_per_second):
    """Reserve a media request start, releasing the lane before the HTTP transfer."""
    root = Path(root)
    marker = root / ("rate-image-" + site + ".json")
    while True:
        if cancelled():
            raise UpdateError("CANCELLED", "Update paused")
        try:
            with FileLock(root / ("rate-image-" + site + ".lock"), timeout=0):
                saved = read_json(marker) if marker.exists() else {}
                due = max(saved.get("next_at", 0), saved.get("cooldown_until", 0))
                stamp = time.time()
                if stamp >= due:
                    rate = requests_per_second()
                    atomic_json(marker, {**saved, "next_at": stamp + (1 / rate if rate else 0)})
                    return
        except RuntimeError:
            pass
        time.sleep(0.05)


def cooldown(root, site, seconds, *, image=False):
    prefix = "rate-image-" if image else "rate-"
    root = Path(root)
    marker = root / (prefix + site + ".json")
    with FileLock(root / (prefix + site + ".lock")):
        saved = read_json(marker) if marker.exists() else {}
        atomic_json(
            marker, {**saved, "cooldown_until": max(saved.get("cooldown_until", 0), time.time() + seconds)}
        )
