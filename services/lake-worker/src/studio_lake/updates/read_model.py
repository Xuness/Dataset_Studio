"""Bounded, indexed UI reads. Job history never needs client-side enumeration."""

import base64
import json

from .protocol import number
from .sites import UpdateError

STATES = {"queued", "running", "paused", "cancelled", "completed", "completed_with_exclusions",
          "waiting_retry", "waiting_space", "waiting_credentials", "needs_review"}
ACTIVE = ("queued", "running", "waiting_retry")
ATTENTION = ("waiting_space", "waiting_credentials", "needs_review", "completed_with_exclusions")
PROBLEMS = ("failed", "needs_review", "unavailable")
DDL = """
CREATE INDEX IF NOT EXISTS jobs_recent ON jobs(created_at DESC,id DESC);
CREATE INDEX IF NOT EXISTS jobs_lake_recent ON jobs(lake_id,created_at DESC,id DESC);
CREATE INDEX IF NOT EXISTS jobs_state_recent ON jobs(state,created_at DESC,id DESC);
CREATE INDEX IF NOT EXISTS jobs_lake_state_recent ON jobs(lake_id,state,created_at DESC,id DESC);
CREATE INDEX IF NOT EXISTS items_reason ON items(job_id,reason,post_id);
CREATE INDEX IF NOT EXISTS items_state_id ON items(job_id,state,post_id);
CREATE TABLE IF NOT EXISTS job_counts(lake_id TEXT NOT NULL,state TEXT NOT NULL,n INTEGER NOT NULL,
 PRIMARY KEY(lake_id,state));
INSERT OR REPLACE INTO job_counts SELECT lake_id,state,count(*) FROM jobs GROUP BY lake_id,state;
CREATE TRIGGER IF NOT EXISTS job_count_insert AFTER INSERT ON jobs BEGIN
 INSERT INTO job_counts VALUES(new.lake_id,new.state,1)
 ON CONFLICT(lake_id,state) DO UPDATE SET n=n+1; END;
CREATE TRIGGER IF NOT EXISTS job_count_change AFTER UPDATE OF state ON jobs WHEN old.state<>new.state BEGIN
 UPDATE job_counts SET n=n-1 WHERE lake_id=old.lake_id AND state=old.state;
 INSERT INTO job_counts VALUES(new.lake_id,new.state,1)
 ON CONFLICT(lake_id,state) DO UPDATE SET n=n+1; END;
"""


def jobs(state, after="", limit=50, lake_id=None, status=None):
    number(limit, 1, 200)
    where, values = [], []
    if lake_id:
        state.lake(lake_id)
        where.append("lake_id=?")
        values.append(lake_id)
    selected = ACTIVE if status == "active" else ATTENTION if status == "attention" else (status,) if status else ()
    if any(s not in STATES for s in selected):
        raise UpdateError("INVALID_INPUT", "Unknown task state")
    if after:
        try:
            if len(after) > 2048:
                raise ValueError()
            date, identity, scope = json.loads(base64.urlsafe_b64decode(after))
            if scope != [lake_id or None, status or None] or not isinstance(date, str) or not isinstance(identity, str):
                raise ValueError()
        except (ValueError, TypeError, UnicodeError):
            raise UpdateError("INVALID_INPUT", "Task cursor does not match this filter") from None
        where.append("(created_at,id)<(?,?)")
        values.extend((date, identity))
    with state.db() as db:
        rows = []
        # Fetch bounded pages per state so sparse activity never scans completed history.
        for item_state in selected or (None,):
            clauses = [*where, "state=?"] if item_state else where
            parameters = [*values, item_state] if item_state else values
            rows.extend(db.execute("SELECT id,created_at FROM jobs " + ("WHERE " + " AND ".join(clauses) if clauses else "") +
                          " ORDER BY created_at DESC,id DESC LIMIT ?", (*parameters, limit + 1)).fetchall())
        rows.sort(key=lambda r: (r["created_at"], r["id"]), reverse=True)
    items, used = [], 0
    for row in rows[:limit]:
        item = state.job(row["id"])
        size = len(json.dumps(item, separators=(",", ":")).encode())
        if items and used + size > 2 * 1024 * 1024:
            break
        items.append(item)
        used += size
    cursor = None
    if len(rows) > len(items):
        last = rows[len(items) - 1]
        cursor = base64.urlsafe_b64encode(json.dumps([last["created_at"], last["id"],
                        [lake_id or None, status or None]]).encode()).decode()
    return {"items": items, "next_cursor": cursor}


def activity(state):
    with state.db() as db:
        counts = [dict(r) for r in db.execute("SELECT * FROM job_counts WHERE n>0 ORDER BY lake_id,state")]
    return {"counts": counts, "active": jobs(state, limit=20, status="active")["items"],
            "attention": jobs(state, limit=20, status="attention")["items"]}
