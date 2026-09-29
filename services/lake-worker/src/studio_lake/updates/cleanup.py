"""Durable cancellation cleanup, separate from the business terminal state."""

from contextlib import ExitStack
import time

from ..util import FileLock, IntegrityError, failpoint, now
from .archive import reconcile
from .sites import UpdateError
from . import spool


DDL = """
CREATE TABLE IF NOT EXISTS job_cleanup(
 job_id TEXT PRIMARY KEY REFERENCES jobs(id),execution INTEGER NOT NULL,
 phase TEXT NOT NULL CHECK(phase IN ('pending','reconciled','complete')),
 retry_at REAL NOT NULL DEFAULT 0,error_code TEXT,updated_at TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS cleanup_due ON job_cleanup(retry_at,updated_at,job_id)
 WHERE phase<>'complete';
CREATE TRIGGER IF NOT EXISTS cancel_cleanup AFTER UPDATE OF state ON jobs WHEN new.state='cancelled' BEGIN
 INSERT OR IGNORE INTO job_cleanup(job_id,execution,phase,updated_at)
 VALUES(new.id,new.execution,'pending',new.updated_at); END;
INSERT OR IGNORE INTO job_cleanup(job_id,execution,phase,updated_at)
 SELECT id,execution,'pending',updated_at FROM jobs WHERE state='cancelled';
CREATE INDEX IF NOT EXISTS jobs_runnable_lake ON jobs(lake_id,created_at,id,retry_at)
 WHERE state IN ('queued','running','waiting_retry','waiting_space');
CREATE TABLE IF NOT EXISTS lake_dispatch(lake_id TEXT PRIMARY KEY REFERENCES lakes(id),sequence INTEGER NOT NULL);
"""


def record(state, identity, *, phase=None, error_code=None, delay=0):
    with state.db() as db:
        db.execute(
            "UPDATE job_cleanup SET phase=coalesce(?,phase),retry_at=?,error_code=?,updated_at=? "
            "WHERE job_id=? AND phase<>'complete'",
            (phase, time.time() + delay if delay else 0, error_code, now(), identity),
        )


def run(state, identity, resources=None):
    # Taking the actual lock, not observing execution_active, proves that all stage workers exited.
    try:
        with ExitStack() as locks:
            try:
                locks.enter_context(state.execution_lock(identity))
            except RuntimeError:
                record(state, identity, error_code="UPDATE_BUSY", delay=1)
                return
            with state.db() as db:
                row = db.execute(
                    "SELECT j.state,j.execution,j.lake_id,c.execution AS cleanup_execution,c.phase "
                    "FROM jobs j JOIN job_cleanup c ON c.job_id=j.id WHERE j.id=?", (identity,),
                ).fetchone()
            if row is None or row["phase"] == "complete":
                return
            if row["state"] != "cancelled" or row["execution"] != row["cleanup_execution"]:
                raise UpdateError("UPDATE_CLEANUP_UNSAFE", "Cleanup no longer owns this execution")
            lib = state.library(row["lake_id"])
            try:
                locks.enter_context(FileLock(lib.cache / ".daily-run.lock", timeout=0))
            except RuntimeError:
                record(state, identity, error_code="UPDATE_BUSY", delay=1)
                return
            path = spool.verify_owner(state, lib, identity)
            if row["phase"] == "pending":
                # Recover formal archives first. Unsealed, unverifiable archive material remains
                # on the main disk; cleanup only owns this task's disposable SSD spool.
                lib.recover()
                reconcile(state, lib, identity)
                from .runner import lease

                lease(lib, identity)
                failpoint("after_cancel_lease_released")
                record(state, identity, phase="reconciled")
                failpoint("after_cancel_reconciled")
            if resources is not None:
                resources.retire(path)
            if spool.remove_chunk(lib, path):
                failpoint("after_cancel_files_removed")
                record(state, identity, phase="complete")
            else:
                record(state, identity)
    except Exception as error:
        code = error.code if isinstance(error, UpdateError) else (
            "UPDATE_CLEANUP_UNSAFE" if isinstance(error, IntegrityError) else
            "UPDATE_CLEANUP_IO" if isinstance(error, OSError) else "UPDATE_CLEANUP_INTEGRITY"
        )
        # No paths, response URLs or credentials enter the public receipt.
        record(state, identity, error_code=code, delay=30)
