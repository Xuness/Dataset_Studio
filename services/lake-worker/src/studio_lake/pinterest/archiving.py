"""Idle-lake maintenance survives terminal jobs without changing their state."""

import json
import time

from .lake.compaction import compact
from ..canonical import canonical, utc
from ..updates import locations
from ..util import FileLock


class Archiving:
    def __init__(self, service, stop):
        self.service, self.state, self.stop = service, service.state, stop
        self.checked_at, self.after = 0.0, ""
        self.last = None

    def candidate(self, active=(), *, at=None):
        at = time.monotonic() if at is None else at
        if at < self.checked_at + 30:
            return None
        self.checked_at = at
        excluded = " AND l.id NOT IN (" + ",".join("?" for _ in active) + ")" if active else ""
        eligible = " AND coalesce((SELECT json_extract(value,'$.retry_at') FROM settings WHERE key='pinterest_archive:'||l.id),0)<=?"
        with self.state.db() as db:
            row = db.execute("SELECT l.id FROM lakes l JOIN pinterest_lakes p ON p.lake_id=l.id WHERE l.id>? "
                "AND NOT EXISTS (SELECT 1 FROM lake_relocations r WHERE r.lake_id=l.id AND r.phase NOT IN ('complete','cancelled'))" + excluded +
                eligible + " ORDER BY l.id LIMIT 1", (self.after, *active, time.time())).fetchone()
            if row is None and self.after:
                self.after = ""
                row = db.execute("SELECT l.id FROM lakes l JOIN pinterest_lakes p ON p.lake_id=l.id WHERE "
                    "NOT EXISTS (SELECT 1 FROM lake_relocations r WHERE r.lake_id=l.id AND r.phase NOT IN ('complete','cancelled'))" + excluded +
                    eligible + " ORDER BY l.id LIMIT 1", (*active, time.time())).fetchone()
        if row:
            self.after = row[0]
            return row[0]
        return None

    def run(self, identity):
        try:
            with locations.access(self.state, identity):
                lib = self.service.library(identity)
                with FileLock(lib.cache / ".daily-run.lock", timeout=0):
                    result = compact(lib, stop=self.stop.is_set)
        except RuntimeError as error:
            result = dict(state="busy" if str(error).startswith("另一个进程正在使用此工作区") else "needs_review",
                          error_code="PINTEREST_ARCHIVE_CHECK_REQUIRED", error_message=str(error)[:512])
        except Exception as error:
            result = dict(state="needs_review", error_code="PINTEREST_ARCHIVE_CHECK_REQUIRED", error_message=str(error)[:512])
        delay = 3600 if result["state"] == "needs_review" else 300 if result["state"] in {"waiting_space", "busy"} else 0
        self.last = dict(lake_id=identity, checked_at=utc(), retry_at=time.time() + delay, **result)
        with self.state.db() as db:
            db.execute("INSERT INTO settings VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                       ("pinterest_archive:" + identity, canonical(self.last)))
        return self.last

    def status(self, identity):
        with self.state.db() as db:
            row = db.execute("SELECT value FROM settings WHERE key=?", ("pinterest_archive:" + identity,)).fetchone()
        return json.loads(row[0]) if row else None
