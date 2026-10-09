"""Select due heads per idle lake; failed futures cannot tear down other lakes."""

from contextlib import ExitStack
import time

from ..util import now


def candidates(state, active, limit, at=None):
    if limit <= 0:
        return []
    at = time.time() if at is None else at
    excluded = " AND l.id NOT IN (" + ",".join("?" for _ in active) + ")" if active else ""
    with state.db() as db:
        return db.execute(
            "SELECT * FROM (SELECT j.id,j.lake_id,j.execution,'update' AS family,j.created_at,coalesce(d.sequence,0) AS service_order FROM lakes l JOIN jobs j ON j.id=("
            "SELECT id FROM jobs WHERE lake_id=l.id "
            "AND state IN ('queued','running','waiting_retry','waiting_space') "
            "AND retry_at<=? ORDER BY created_at,id LIMIT 1) "
            "LEFT JOIN lake_dispatch d ON d.lake_id=l.id WHERE NOT EXISTS "
            "(SELECT 1 FROM lake_relocations r WHERE r.lake_id=l.id AND r.phase NOT IN ('complete','cancelled'))" + excluded +
            " UNION ALL SELECT j.id,j.lake_id,j.execution_epoch,'collection',j.created_at,coalesce(d.sequence,0) "
            "FROM lakes l JOIN collection_jobs j ON j.id=(SELECT id FROM collection_jobs WHERE lake_id=l.id "
            "AND (state IN ('queued','running','waiting_retry','waiting_resources','publishing','pausing','cancelling') "
            "OR (state='cancelled' AND json_extract(counters_json,'$.cleanup')='pending')) "
            "AND retry_at_ms<=? ORDER BY updated_at,job_row LIMIT 1) LEFT JOIN lake_dispatch d ON d.lake_id=l.id "
            "WHERE NOT EXISTS (SELECT 1 FROM lake_relocations r WHERE r.lake_id=l.id AND r.phase NOT IN ('complete','cancelled'))" + excluded +
            " UNION ALL SELECT j.id,j.lake_id,j.execution_epoch,'pinterest',j.created_at,coalesce(d.sequence,0) "
            "FROM lakes l JOIN pinterest_jobs j ON j.id=(SELECT id FROM pinterest_jobs WHERE lake_id=l.id "
            "AND state IN ('queued','running','waiting_retry','waiting_resources','pausing','cancelling') "
            "AND retry_at<=? ORDER BY updated_at,job_row LIMIT 1) LEFT JOIN lake_dispatch d ON d.lake_id=l.id "
            "WHERE NOT EXISTS (SELECT 1 FROM lake_relocations r WHERE r.lake_id=l.id AND r.phase NOT IN ('complete','cancelled'))" + excluded +
            ") ORDER BY service_order,created_at,id LIMIT ?", (at, *active, int(at * 1000), *active, at, *active, limit),
        ).fetchall()


def submitted(state, lake):
    # Monotonic service order, independent of wall-clock changes and retained across restarts.
    with state.db() as db:
        db.execute(
            "INSERT INTO lake_dispatch(lake_id,sequence) VALUES(?,(SELECT coalesce(max(sequence),0)+1 FROM lake_dispatch)) "
            "ON CONFLICT(lake_id) DO UPDATE SET sequence=excluded.sequence", (lake,),
        )


def next_cleanup(state, active=(), at=None):
    excluded = " AND j.lake_id NOT IN (" + ",".join("?" for _ in active) + ")" if active else ""
    with state.db() as db:
        return db.execute(
            "SELECT c.job_id,j.lake_id FROM job_cleanup c JOIN jobs j ON j.id=c.job_id "
            "WHERE c.phase<>'complete' AND c.retry_at<=? AND NOT EXISTS "
            "(SELECT 1 FROM lake_relocations r WHERE r.lake_id=j.lake_id AND r.phase NOT IN ('complete','cancelled'))" + excluded +
            " ORDER BY c.retry_at,c.updated_at,c.job_id LIMIT 1", (time.time() if at is None else at, *active),
        ).fetchone()


def failed(state, identity, execution):
    # An escaped exception may race with a user command or another process. Never
    # change an execution we no longer own, and never overwrite pause/cancel/finish.
    with ExitStack() as locks:
        try:
            locks.enter_context(state.execution_lock(identity))
        except RuntimeError:
            return
        with state.db() as db:
            db.execute(
                "UPDATE jobs SET state='waiting_retry',retry_at=?,error_code='UPDATE_EXECUTOR_FAILED',"
                "error_message='Execution stopped unexpectedly; durable work retained',updated_at=? "
                "WHERE id=? AND execution=? AND state IN ('queued','running','waiting_retry','waiting_space')",
                (time.time() + 60, now(), identity, execution),
            )
