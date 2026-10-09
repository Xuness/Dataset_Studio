"""Unified bounded reads for the existing lake workbench; writes retain family contracts."""

import json

from . import model
from .schedules import Schedules


class Workspace:
    def __init__(self, service):
        self.service, self.state = service, service.state

    def lakes(self, args):
        model.fields(args, (), ("cursor", "limit", "include_pinterest"))
        include = args.get("include_pinterest", False)
        if type(include) is not bool:
            model.invalid("include_pinterest must be boolean")
        limit = model.integer(args.get("limit", 200), 1, 200)
        filters = dict(workspace="lakes")
        if include:
            filters["include_pinterest"] = True
        after = self.service.position(args.get("cursor"), filters)
        with self.state.db() as db:
            rows = list(db.execute("SELECT rowid,* FROM lakes WHERE rowid>?" + ("" if include else " AND site<>'pinterest'") +
                                  " ORDER BY rowid LIMIT ?", (after, limit + 1)))
        return dict(items=[{k: r[k] for k in ("id", "site", "media", "index_root", "registered_at")} for r in rows[:limit]],
                    next_cursor=self.service.cursor(filters, rows[limit-1]["rowid"]) if len(rows) > limit else None)

    def jobs(self, args):
        model.fields(args, (), ("cursor", "limit", "library_id", "state", "include_pinterest"))
        include = args.get("include_pinterest", False)
        if type(include) is not bool:
            model.invalid("include_pinterest must be boolean")
        limit = model.integer(args.get("limit", 50), 1, 100)
        filters = dict(workspace="jobs", library_id=args.get("library_id"), state=args.get("state"))
        if include:
            filters["include_pinterest"] = True
        after = self.service.position(args.get("cursor"), filters, composite=True)
        clauses, values = [], []
        if after:
            if not isinstance(after, list) or len(after) != 2 or any(not isinstance(v, str) for v in after):
                model.invalid("Invalid workspace page cursor")
            clauses.append("(created_at,id)<(?,?)")
            values.extend(after)
        if args.get("library_id"):
            clauses.append("lake_id=?")
            values.append(args["library_id"])
        state = args.get("state")
        if state == "active":
            clauses.append("state IN ('queued','running','pausing','cancelling','publishing','waiting_retry','waiting_space','waiting_resources')")
        elif state == "attention":
            clauses.append("state IN ('needs_review','waiting_credentials','waiting_budget','completed_with_gaps','completed_with_exclusions','failed')")
        elif state:
            clauses.append("state=?")
            values.append(state)
        where = " WHERE " + " AND ".join(clauses) if clauses else ""
        with self.state.db() as db:
            rows = list(db.execute("SELECT * FROM (SELECT id,lake_id,state,created_at,'update' family FROM jobs UNION ALL "
                                   "SELECT id,lake_id,state,created_at,'collection' family FROM collection_jobs" +
                                   (" UNION ALL SELECT id,lake_id,state,created_at,'pinterest' family FROM pinterest_jobs" if include else "") + ")" + where +
                                   " ORDER BY created_at DESC,id DESC LIMIT ?", (*values, limit + 1)))
        items, size = [], 0
        for row in rows[:limit]:
            if row["family"] == "pinterest":
                from ..pinterest.service import Service
                job = Service(self.state).job(row["id"])
            else:
                job = self.service.job(row["id"]) if row["family"] == "collection" else self.state.job(row["id"])
            value = dict(family=row["family"], job=job)
            encoded_size = len(json.dumps(value, ensure_ascii=False).encode())
            if items and size + encoded_size > 1800 * 1024:
                break
            items.append(value)
            size += encoded_size
        last = rows[len(items)-1] if items else None
        return dict(items=items, next_cursor=self.service.cursor(filters, [last["created_at"], last["id"]]) if last and len(rows) > len(items) else None)

    def schedules(self, args):
        model.fields(args, (), ("cursor", "limit", "library_id"))
        limit = model.integer(args.get("limit", 50), 1, 200)
        filters = dict(workspace="schedules", library_id=args.get("library_id"))
        after = self.service.position(args.get("cursor"), filters, composite=True)
        if after and not isinstance(after, str):
            model.invalid("Invalid schedule cursor")
        clause, values = "", []
        if args.get("library_id"):
            clause = " AND lake_id=?"
            values = [args["library_id"]]
        with self.state.db() as db:
            rows = list(db.execute("""SELECT * FROM (SELECT id,json_extract(definition,'$.library_id') lake_id,'update' family FROM schedules
                UNION ALL SELECT id,lake_id,'collection' family FROM collection_schedules) WHERE id>?""" + clause + " ORDER BY id LIMIT ?", (after or "", *values, limit+1)))
            items = []
            for row in rows[:limit]:
                if row["family"] == "collection":
                    value = Schedules.public(db.execute("SELECT * FROM collection_schedules WHERE id=?", (row["id"],)).fetchone())
                else:
                    from datetime import datetime, timezone

                    value = dict(db.execute("SELECT * FROM schedules WHERE id=?", (row["id"],)).fetchone())
                    value["definition"] = json.loads(value["definition"])
                    value["enabled"] = bool(value["enabled"])
                    value["every_seconds"] = value["every_seconds"] or None
                    value["next_run_at"] = datetime.fromtimestamp(value.pop("next_at"), timezone.utc).isoformat()
                items.append(dict(family=row["family"], schedule=value))
        return dict(items=items, next_cursor=self.service.cursor(filters, rows[limit-1]["id"]) if len(rows) > limit else None)
