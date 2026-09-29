"""Execution handoff and atomic user commands; cancellation never waits for a worker."""

from contextlib import ExitStack
import json

from ..util import FileLock, now
from .archive import reconcile
from .sites import UpdateError


RESTART = {"resume", "retry", "replay"}
COMPLETED = {"completed", "completed_with_exclusions"}


def read_job(db, identity):
    row = db.execute("SELECT * FROM jobs WHERE id=?", (identity,)).fetchone()
    if row is None:
        raise UpdateError("NOT_FOUND", "Update job not found")
    return row


def validate(row, action):
    if row["state"] == "cancelled" and action != "cancel":
        raise UpdateError("UPDATE_CONFLICT", "Cancelled task is closed; create a new task to repeat it")
    if row["state"] == "running" and action in RESTART:
        raise UpdateError("UPDATE_CONFLICT", "Pause a running task before restarting it")
    if row["state"] in COMPLETED and action != "retry":
        raise UpdateError("UPDATE_CONFLICT", "Completed job cannot be changed")


def reset_items(db, identity):
    db.execute(
        "UPDATE items SET state=CASE WHEN observation_id IS NOT NULL AND record_json<>'{}' "
        "AND coalesce(reason,'') NOT IN ('source_deleted','source_restricted','no_image_url',"
        "'not_returned_by_api','metadata_not_available') "
        "AND NOT (coalesce(reason,'')='image_http_404' AND attempts>=8) "
        "THEN 'pending' ELSE 'pending_metadata' END,reason=NULL,retry_at=0,attempts=0 "
        "WHERE job_id=? AND state IN ('failed','needs_review','unavailable')",
        (identity,),
    )


def action(state, identity, command):
    if command not in RESTART | {"pause", "cancel"}:
        raise UpdateError("INVALID_INPUT", "Unknown update action")
    # All paths that acquire both use execution -> lake -> archive -> control.
    # Pause/cancel only use a short control transaction, even during slow I/O.
    with ExitStack() as locks:
        if command in RESTART:
            try:
                locks.enter_context(state.execution_lock(identity))
            except RuntimeError:
                raise UpdateError(
                    "UPDATE_CONFLICT", "The previous batch is still stopping; retry after it exits"
                ) from None
            with state.db() as db:
                snapshot = read_job(db, identity)
                validate(snapshot, command)
            if snapshot["state"] != "queued":
                lib = state.library(snapshot["lake_id"])
                try:
                    locks.enter_context(FileLock(lib.cache / ".daily-run.lock", timeout=0))
                except RuntimeError:
                    raise UpdateError("UPDATE_CONFLICT", "Another task owns this lake; retry after it exits") from None
                # Drain accepted/incomplete archive receipts before resetting items. Otherwise
                # the next worker could replay an older failed receipt over the new retry.
                lib.recover()
                reconcile(state, lib, identity)
        with state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            row = read_job(db, identity)
            validate(row, command)
            if command in RESTART:
                # A duplicate command for an already queued execution is a no-op.
                if row["state"] != "queued":
                    if command == "retry":
                        reset_items(db, identity)
                    cursor = json.loads(row["cursor"])
                    cursor.update(replay_saved_response=command == "replay", slice_pages=0, slice_items=0)
                    changed = db.execute(
                        "UPDATE jobs SET state='queued',cursor=?,retry_at=0,error_code=NULL,error_message=NULL,"
                        "execution=execution+1,updated_at=? WHERE id=? AND state=? AND execution=?",
                        (json.dumps(cursor), now(), identity, row["state"], row["execution"]),
                    ).rowcount
                    if changed != 1:
                        raise UpdateError("UPDATE_CONFLICT", "Task execution changed; refresh before retrying")
            else:
                db.execute(
                    "UPDATE jobs SET state=?,updated_at=? WHERE id=? AND state=? AND execution=?",
                    ("paused" if command == "pause" else "cancelled", now(), identity,
                     row["state"], row["execution"]),
                )
    return state.job(identity)
