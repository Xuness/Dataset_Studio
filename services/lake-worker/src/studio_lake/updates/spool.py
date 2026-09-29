"""Task spool paths: flat, controller-owned, never followed through links."""

from itertools import islice
import os
import re
import stat

from ..util import safe_managed_path, read_json, failpoint, same_directory
from .sites import UpdateError


SUFFIXES = (".ready", ".json", ".downloaded", ".download.json", ".tmp", ".partial", ".partial.json")
FILE_NAME = re.compile(r"[a-f0-9]{64}(?:\.(?:ready|downloaded|partial|tmp|json)|\.(?:download|partial)\.json|"
                       r"(?:\.(?:download|partial))?\.json\.[a-f0-9]{32}\.tmp)")


def directory(lib, identity):
    if not isinstance(identity, str) or not re.fullmatch(r"[a-f0-9]{32}", identity):
        raise UpdateError("UPDATE_CLEANUP_UNSAFE", "Invalid task spool identity")
    return safe_managed_path(lib.cache, lib.cache / "updates" / identity)


def file_path(lib, task_directory, name):
    if not FILE_NAME.fullmatch(name):
        raise UpdateError("UPDATE_CLEANUP_UNSAFE", "Unrecognized file in task spool; retained for review")
    path = safe_managed_path(lib.cache, task_directory / name)
    try:
        info = path.lstat()
    except FileNotFoundError:
        return path
    if not stat.S_ISREG(info.st_mode):
        raise UpdateError("UPDATE_CLEANUP_UNSAFE", "Task spool contains a non-regular file")
    return path


def verify_owner(state, lib, identity):
    path = directory(lib, identity)
    for marker in ("cache_owner.json", "UPDATE-CONTROLLER.json"):
        value = read_json(safe_managed_path(lib.cache, lib.cache / marker))
        if value.get("library_id") != lib.info["library_id"]:
            raise UpdateError("UPDATE_CLEANUP_UNSAFE", "Task spool lake ownership changed")
        if marker == "UPDATE-CONTROLLER.json" and not same_directory(value.get("root"), state.root):
            raise UpdateError("UPDATE_CLEANUP_UNSAFE", "Task spool controller ownership changed")
    return path


def remove_chunk(lib, path, limit=256):
    """Preflight a bounded batch, unlink only regular known files, then remove an empty task dir."""
    if not path.exists():
        return True
    with os.scandir(path) as entries:
        names = [entry.name for entry in islice(entries, limit)]
    paths = [file_path(lib, path, name) for name in names]
    for candidate in paths:
        # Repeat the link check immediately before each unlink; no recursive traversal.
        file_path(lib, path, candidate.name).unlink(missing_ok=True)
        failpoint("after_cancel_file_removed")
    try:
        path.rmdir()
    except OSError:
        if not any(path.iterdir()):
            raise
        return False
    return True
