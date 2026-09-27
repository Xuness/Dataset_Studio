"""One host-wide API lane per site, shared by daemon, CLI and probes."""

from contextlib import contextmanager
from pathlib import Path
import time

from ..util import FileLock, atomic_json, read_json
from .sites import UpdateError


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
        due = read_json(marker).get("next_at", 0) if marker.exists() else 0
        while time.time() < due:
            if cancelled():
                raise UpdateError("CANCELLED", "Update paused")
            time.sleep(min(0.1, max(0, due - time.time())))
        started = time.time()
        try:
            yield
        finally:
            atomic_json(marker, {"next_at": started + delay})
    finally:
        gate.__exit__(None, None, None)
