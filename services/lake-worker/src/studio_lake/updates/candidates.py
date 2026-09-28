"""Bounded candidate reads that never walk a task's completed history."""

import time


def eligible(state, identity, limit):
    rows = []
    with state.db() as db:
        # State can change while the publisher commits; merge one consistent view.
        db.execute("BEGIN")
        for status in ("pending", "failed", "pending_metadata"):
            rows.extend(dict(row) for row in db.execute(
                "SELECT * FROM items INDEXED BY items_state_id WHERE job_id=? AND state=? "
                "AND retry_at<=? AND attempts<8 ORDER BY post_id LIMIT ?",
                (identity, status, time.time(), limit),
            ))
    return sorted(rows, key=lambda row: row["post_id"])[:limit]
