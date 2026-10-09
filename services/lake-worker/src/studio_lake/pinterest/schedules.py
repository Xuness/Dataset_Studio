"""Recurring Pinterest scans: fixed definitions, coalesced downtime, and no overlap on a lake."""

from datetime import datetime, timezone
import json
import time
import uuid

from . import model
from ..canonical import canonical, utc
from ..updates.protocol import timestamp
from ..updates.sites import UpdateError


class Schedules:
    def __init__(self, service):
        self.service, self.state = service, service.state

    @staticmethod
    def public(row):
        return dict(id=row["id"], definition=json.loads(row["definition_json"]), every_seconds=row["every_seconds"],
            next_run_at=datetime.fromtimestamp(row["next_at"], timezone.utc).isoformat(), enabled=bool(row["enabled"]),
            revision=row["revision"], last_job=row["last_job"])

    def list(self, args):
        model.fields(args, (), ("cursor", "limit", "library_id"))
        limit = model.integer(args.get("limit", 50), 1, 100)
        filters = dict(schedules=args.get("library_id"))
        after = self.service.position(args.get("cursor"), filters)
        clause = " AND lake_id=?" if args.get("library_id") else ""
        with self.state.db() as db:
            rows = list(db.execute("SELECT rowid,* FROM pinterest_schedules WHERE rowid>?" + clause + " ORDER BY rowid LIMIT ?",
                                  (after, *([model.identity(args["library_id"])] if clause else []), limit + 1)))
        items, size = [], 0
        for row in rows[:limit]:
            item = self.public(row)
            length = len(canonical(item).encode())
            if items and size + length > 1800 * 1024:
                break
            items.append(item)
            size += length
        return dict(items=items, next_cursor=self.service.cursor(filters, rows[len(items)-1]["rowid"]) if len(rows) > len(items) and items else None)

    def save(self, args):
        model.fields(args, ("request_key", "id", "expected_revision", "definition", "every_seconds", "first_run_at", "enabled"))
        identity = model.identity(args["id"])
        revision = model.integer(args["expected_revision"])
        spec = self.service.preview(dict(definition=args["definition"]))["definition"]
        seconds = model.integer(args["every_seconds"], 60, 31622400)
        if type(args["enabled"]) is not bool:
            model.invalid("Schedule enabled must be a boolean")
        next_at = timestamp(args["first_run_at"]).timestamp()
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            request = self.service._request(db, "schedule_save", args, identity)
            if request["state"] == "succeeded":
                return json.loads(request["result_json"])
            old = db.execute("SELECT revision FROM pinterest_schedules WHERE id=?", (identity,)).fetchone()
            if (old[0] if old else 0) != revision:
                raise UpdateError("PINTEREST_CONFLICT", "Pinterest schedule changed")
            at = utc()
            db.execute("""INSERT INTO pinterest_schedules VALUES(?,?,?,?,?,?,?,NULL,?,?) ON CONFLICT(id) DO UPDATE SET
                definition_json=excluded.definition_json,lake_id=excluded.lake_id,every_seconds=excluded.every_seconds,
                next_at=excluded.next_at,enabled=excluded.enabled,revision=excluded.revision,updated_at=excluded.updated_at""",
                (identity, canonical(spec), spec["library_id"], seconds, next_at, int(args["enabled"]), revision+1, at, at))
            value = self.public(db.execute("SELECT * FROM pinterest_schedules WHERE id=?", (identity,)).fetchone())
            db.execute("UPDATE pinterest_requests SET state='succeeded',result_json=? WHERE request_key=?", (canonical(value), args["request_key"]))
        return value

    def remove(self, args):
        model.fields(args, ("id", "request_key", "expected_revision"))
        model.identity(args["id"])
        model.integer(args["expected_revision"], 1)
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            request = self.service._request(db, "schedule_remove", args, args["id"])
            if request["state"] != "succeeded":
                changed = db.execute("DELETE FROM pinterest_schedules WHERE id=? AND revision=?", (args["id"], args["expected_revision"])).rowcount
                if changed != 1:
                    raise UpdateError("PINTEREST_CONFLICT", "Pinterest schedule changed or was removed")
                db.execute("UPDATE pinterest_requests SET state='succeeded',result_json=? WHERE request_key=?", ('{"removed":true}', args["request_key"]))
        return dict(removed=True)

    def tick(self, at=None):
        at = time.time() if at is None else at
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            rows = list(db.execute("""SELECT s.* FROM pinterest_schedules s WHERE enabled=1 AND next_at<=?
                AND NOT EXISTS(SELECT 1 FROM lake_relocations r WHERE r.lake_id=s.lake_id AND r.phase NOT IN ('complete','cancelled'))
                ORDER BY next_at,id LIMIT 100""", (at,)))
            for row in rows:
                # Check after each creation as two schedules for one lake may be due in this same transaction.
                if db.execute("SELECT 1 FROM pinterest_jobs WHERE lake_id=? AND state NOT IN ('completed','completed_with_gaps','cancelled') LIMIT 1", (row["lake_id"],)).fetchone():
                    continue
                occurrence = row["next_at"] + int((at-row["next_at"]) // row["every_seconds"]) * row["every_seconds"]
                key = str(uuid.uuid5(uuid.NAMESPACE_URL, f"pinterest-schedule:{row['id']}:{row['revision']}:{occurrence}"))
                identity = self.service.create_job_db(db, json.loads(row["definition_json"]), key)
                db.execute("INSERT INTO pinterest_schedule_runs VALUES(?,?,?,?)", (row["id"], row["revision"], occurrence, identity))
                db.execute("UPDATE pinterest_schedules SET next_at=?,last_job=?,updated_at=? WHERE id=?", (occurrence+row["every_seconds"], identity, utc(), row["id"]))
