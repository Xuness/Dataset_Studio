"""Stable controller-owned admission survives moving either physical lake root."""

from contextlib import contextmanager, ExitStack
from functools import wraps
import uuid

from ..util import FileLock, safe_managed_path
from .sites import UpdateError

DDL = """
CREATE TABLE IF NOT EXISTS lake_relocations(
 id TEXT PRIMARY KEY,lake_id TEXT NOT NULL REFERENCES lakes(id),phase TEXT NOT NULL,
 old_media TEXT NOT NULL,old_index TEXT NOT NULL,media_root TEXT,index_root TEXT,
 checkpoint TEXT,created_at TEXT NOT NULL);
CREATE UNIQUE INDEX IF NOT EXISTS relocation_active ON lake_relocations(lake_id)
 WHERE phase NOT IN ('complete','cancelled');
"""


def pending(state, identity, db=None):
    sql = "SELECT 1 FROM lake_relocations WHERE lake_id=? AND phase NOT IN ('complete','cancelled')"
    if db is not None:
        return db.execute(sql, (identity,)).fetchone() is not None
    with state.db() as db:
        return pending(state, identity, db)


def directory(state, identity):
    # Library IDs are UUIDs, not caller-controlled paths.
    try:
        if str(uuid.UUID(identity)) != identity:
            raise ValueError()
    except (ValueError, TypeError, AttributeError):
        raise UpdateError("INVALID_INPUT", "Invalid lake identity") from None
    return safe_managed_path(state.root, state.root / "location-access" / identity)


@contextmanager
def access(state, identity):
    root = directory(state, identity)
    token = root / (uuid.uuid4().hex + ".lock")
    with FileLock(root / "gate.lock", timeout=10):
        if pending(state, identity):
            raise UpdateError("UPDATE_CONFLICT", "Lake relocation is pending; finish or cancel it first")
        lease = FileLock(token, timeout=0)
        lease.__enter__()
    try:
        yield
    finally:
        lease.__exit__(None, None, None)
        try:
            token.unlink(missing_ok=True)
        except OSError:
            # A drainer may already hold this now-idle token on Windows.
            pass


@contextmanager
def drained(state, identity):
    # Caller has durably frozen admissions. Hold every surviving token until its
    # snapshot is complete; crashed holders release their OS lock automatically.
    with ExitStack() as stack:
        root = directory(state, identity)
        stack.enter_context(FileLock(root / "gate.lock", timeout=10))
        try:
            for path in root.glob("*.lock"):
                if path.name != "gate.lock":
                    stack.enter_context(FileLock(safe_managed_path(state.root, path), timeout=0))
        except RuntimeError:
            raise UpdateError("UPDATE_CONFLICT", "Existing lake operations are still stopping") from None
        yield


def input_access(by_input=False):
    def decorate(function):
        @wraps(function)
        def run(state, *args, **kwargs):
            identity = args[0] if args else kwargs["identity" if by_input else "library_id"]
            lake = state.input(identity)["lake_id"] if by_input else identity
            with access(state, lake):
                return function(state, *args, **kwargs)
        return run
    return decorate
