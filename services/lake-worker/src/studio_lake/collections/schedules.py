"""Coalesced, revision-checked recurring snapshots in the shared worker."""

from datetime import datetime, timezone
import json
import time
import uuid

from . import TERMINAL, model, requests as ledger
from ..media_lake.schema import canonical, utc
from ..updates.protocol import timestamp
from ..updates.sites import UpdateError


class Schedules:
    def __init__(self, service):
        self.service, self.state = service, service.state

    @staticmethod
    def public(row):
        return dict(id=row["id"], definition=json.loads(row["definition_json"]), every_seconds=row["every_seconds"],
                    next_run_at=datetime.fromtimestamp(row["next_at"], timezone.utc).isoformat(),
                    enabled=bool(row["enabled"]), revision=row["revision"], last_job=row["last_job"])

    def list(self, args):
        model.fields(args, (), ("cursor", "limit", "library_id"))
        limit = model.integer(args.get("limit", 50), 1, 200)
        filters = dict(schedules=args.get("library_id"))
        after = self.service.position(args.get("cursor"), filters)
        clause = " AND lake_id=?" if args.get("library_id") else ""
        with self.state.db() as db:
            rows = list(db.execute("SELECT rowid,* FROM collection_schedules WHERE rowid>?" + clause + " ORDER BY rowid LIMIT ?",
                                   (after, *([args["library_id"]] if clause else []), limit + 1)))
        return dict(items=[self.public(r) for r in rows[:limit]], next_cursor=self.service.cursor(filters, rows[limit-1]["rowid"]) if len(rows) > limit else None)

    def save(self, args):
        model.fields(args, ("request_key", "id", "expected_revision", "definition", "every_seconds", "first_run_at", "enabled"))
        identity = model.identity(args["id"])
        revision = model.integer(args["expected_revision"])
        spec = self.service.preview(args["definition"])["definition"]
        seconds = model.integer(args["every_seconds"], 60, 31622400)
        enabled = model.boolean(args["enabled"])
        next_at = timestamp(args["first_run_at"]).timestamp()
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            request, _ = ledger.begin(db, "schedule_save", args["request_key"], args, identity)
            if request["state"] == "succeeded":
                return json.loads(request["result_json"])
            old = db.execute("SELECT revision FROM collection_schedules WHERE id=?", (identity,)).fetchone()
            if (old[0] if old else 0) != revision:
                raise UpdateError("REVISION_CONFLICT", "Collection schedule changed")
            at = utc()
            db.execute("""INSERT INTO collection_schedules VALUES(?,?,?,?,?,?,?,NULL,?,?) ON CONFLICT(id) DO UPDATE SET
                definition_json=excluded.definition_json,lake_id=excluded.lake_id,every_seconds=excluded.every_seconds,
                next_at=excluded.next_at,enabled=excluded.enabled,revision=excluded.revision,updated_at=excluded.updated_at""",
                       (identity, canonical(spec), spec["library_id"], seconds, next_at, int(enabled), revision + 1, at, at))
            value = self.public(db.execute("SELECT * FROM collection_schedules WHERE id=?", (identity,)).fetchone())
            ledger.succeed(db, args["request_key"], identity, value)
        return value

    def remove(self, args):
        model.fields(args, ("id", "request_key", "expected_revision"))
        model.identity(args["id"])
        model.integer(args["expected_revision"], 1)
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            request, _ = ledger.begin(db, "schedule_remove", args["request_key"], args, args["id"])
            if request["state"] != "succeeded":
                changed = db.execute("DELETE FROM collection_schedules WHERE id=? AND revision=?", (args["id"], args["expected_revision"])).rowcount
                if changed != 1:
                    raise UpdateError("REVISION_CONFLICT", "Collection schedule changed or was removed")
                ledger.succeed(db, args["request_key"], args["id"])
        return dict(removed=True)

    def tick(self, at=None):
        at = time.time() if at is None else at
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            rows = list(db.execute("""SELECT s.* FROM collection_schedules s WHERE s.enabled=1 AND s.next_at<=?
                AND NOT EXISTS(SELECT 1 FROM lake_relocations r WHERE r.lake_id=s.lake_id AND r.phase NOT IN ('complete','cancelled'))
                AND NOT EXISTS(SELECT 1 FROM collection_jobs j WHERE j.id=s.last_job
                  AND j.state NOT IN (""" + ",".join("?" for _ in TERMINAL) + ")) ORDER BY s.next_at,s.id LIMIT 100", (at, *sorted(TERMINAL))))
            for row in rows:
                # Paused, waiting for credentials/budget and review are all unfinished.
                # Preserve them for the user instead of accumulating later copies.
                occurrence = row["next_at"] + int((at - row["next_at"]) // row["every_seconds"]) * row["every_seconds"]
                key = str(uuid.uuid5(uuid.NAMESPACE_URL, f"pixiv-schedule:{row['id']}:{row['revision']}:{occurrence}"))
                identity, _, _ = self.service.create_job_db(db, json.loads(row["definition_json"]), key)
                db.execute("UPDATE collection_schedules SET next_at=?,last_job=?,updated_at=? WHERE id=?",
                           (occurrence + row["every_seconds"], identity, utc(), row["id"]))
