"""Cross-process device admission, reentrant only for the owning thread."""

from contextlib import contextmanager
import threading
import os
from pathlib import Path

from ..util import FileLock, digest

_held = threading.local()


@contextmanager
def device_lock(root, media):
    path = Path(root) / ("device-" + digest(str(Path(media).stat().st_dev).encode())[:16] + ".lock")
    key = os.path.normcase(str(path.resolve()))
    if key.startswith("\\\\?\\"):
        key = key[4:]
    held = getattr(_held, "locks", {})
    _held.locks = held
    if key in held:
        held[key] += 1
        try:
            yield
        finally:
            held[key] -= 1
        return
    with FileLock(path):
        held[key] = 1
        try:
            yield
        finally:
            del held[key]
