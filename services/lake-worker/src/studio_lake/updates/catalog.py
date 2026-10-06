"""Bounded, versioned post metadata reads independent of stored image objects."""

import json
import time

from ..util import stable_id
from .archive import online
from .protocol import number
from .sites import UpdateError
from .tag_query import normalize

SCAN_LIMIT = 2048


def version(lib, requested=None):
    with online(lib) as (_, status):
        generation, seq = status["generation"], int(status["served_seq"])
        if requested is not None:
            parts = requested.split(":") if isinstance(requested, str) else []
            if len(parts) != 3 or parts[:2] != ["online-v2", generation] or not parts[2].isdigit():
                raise UpdateError("SOURCE_CHANGED", "Metadata view belongs to another online generation")
            seq = int(parts[2])
        if not int(status["min_seq"]) <= seq <= int(status["served_seq"]):
            raise UpdateError("VIEW_EXPIRED", "Metadata view is no longer retained; refresh the catalog")
    return f"online-v2:{generation}:{seq}", seq, generation


def predicate(db, query):
    groups = {}
    for name, tags in query.items():
        ids = []
        for tag in tags:
            row = next(db.execute("SELECT tag_id FROM tags WHERE tag=?", (tag,)), None)
            if row:
                ids.append(f"t{row[0]:x}")
            elif name == "all":
                return "0", [], []
        groups[name] = ids
    if query["any"] and not groups["any"]:
        return "0", [], []
    terms = []
    if groups["all"]:
        terms.append("(" + " AND ".join(groups["all"]) + ")")
    if groups["any"]:
        terms.append("(" + " OR ".join(groups["any"]) + ")")
    clauses, args, seed = [], [], None
    if any(query.values()):
        clauses.append("o.tag_string IS NOT NULL")
    if terms:
        expression = " AND ".join(terms)
        rows = [r[0] for r in db.execute("SELECT rowid FROM tag_index WHERE tag_index MATCH ? LIMIT 5001", (expression,))]
        if len(rows) <= 5000:
            seed = rows
        clauses.append("EXISTS(SELECT 1 FROM tag_index WHERE rowid=o.row_id AND tag_index MATCH ?)")
        args.append(expression)
    if groups["none"]:
        clauses.append("NOT EXISTS(SELECT 1 FROM tag_index WHERE rowid=o.row_id AND tag_index MATCH ?)")
        args.append(" OR ".join(groups["none"]))
    return " AND ".join(clauses) or "1", args, seed


def read_page(lib, args, *, retain=True, scan_limit=SCAN_LIMIT):
    allowed = {"query", "after", "start_id", "end_id", "limit", "version", "post_ids", "missing_media"}
    if not isinstance(args, dict) or set(args) - allowed:
        raise UpdateError("INVALID_INPUT", "Unknown metadata catalog field")
    query = normalize(args.get("query") if args.get("query") is not None else {}, require_positive=False)
    defaults = dict(start_id=1, end_id=2**63 - 1, after=0, limit=100)
    values = {key: number(args[key] if args.get(key) is not None else default,
                          0 if key == "after" else 1, 200 if key == "limit" else 2**63 - 1)
              for key, default in defaults.items()}
    lower, upper = values["start_id"], values["end_id"]
    if lower >= upper:
        raise UpdateError("INVALID_INPUT", "Metadata bounds must be [start,end)")
    after, limit = values["after"], values["limit"]
    scan_limit = number(scan_limit, 1, SCAN_LIMIT)
    missing = args.get("missing_media", False)
    if missing is None:
        missing = False
    if not isinstance(missing, bool):
        raise UpdateError("INVALID_INPUT", "missing_media must be boolean")
    ids = args.get("post_ids")
    if ids is not None:
        if not isinstance(ids, list) or len(ids) > 10000:
            raise UpdateError("INVALID_INPUT", "Metadata selection accepts at most 10000 post IDs")
        ids = sorted({number(pid) for pid in ids})
    revision, seq, _ = version(lib, args.get("version"))
    if retain:
        from .runner import lease

        identity = "catalog:" + stable_id(revision, query, lower, upper, ids, missing)
        lease(lib, identity, seq, int((time.time() + 1800) * 1000))
    base = dict(version=revision, items=[], next_after=None, scanned=0)
    if ids == [] or after >= upper - 1:
        return base
    with online(lib) as (db, _):
        condition, values, seed = predicate(db, query)
        if seed == []:
            return base
        clauses = ["p.post_id>?", "p.post_id>=?", "p.post_id<?", "p.valid_from<=?",
                   "(p.valid_until IS NULL OR p.valid_until>?)"]
        params = [after, lower, upper, seq, seq]
        if ids is not None:
            clauses.append("p.post_id IN (SELECT value FROM json_each(?))")
            params.append(json.dumps(ids))
        if seed is not None:
            clauses.append("p.row_id IN (SELECT value FROM json_each(?))")
            params.append(json.dumps(seed))
        sql = (
            "WITH candidates AS MATERIALIZED(SELECT p.post_id,p.row_id,p.asset_id FROM post_versions p WHERE "
            + " AND ".join(clauses) + f" ORDER BY p.post_id LIMIT {scan_limit}) "
            "SELECT p.post_id,o.observation_id,o.observed_at,substr(o.tag_string,1,4096),o.rating,"
            "o.image_width,o.image_height,p.asset_id IS NOT NULL,o.source_kind,length(o.tag_string)>4096,"
            f"({condition}) FROM candidates p JOIN observations o USING(row_id) ORDER BY p.post_id"
        )
        items, scanned, size, last = [], 0, 0, after
        stopped = False
        for row in db.execute(sql, (*params, *values)):
            scanned += 1
            last = row[0]
            if not row[10] or (missing and row[7]):
                continue
            item = dict(post_id=row[0], observation_id=row[1], observed_at=row[2], tags=row[3],
                        rating=row[4], width=row[5], height=row[6], has_media=bool(row[7]),
                        source_kind=row[8], tags_truncated=bool(row[9]))
            size += len(json.dumps(item, ensure_ascii=False).encode())
            items.append(item)
            if len(items) == limit or size >= 384 * 1024:
                stopped = True
                break
    more = (stopped or scanned == scan_limit) and last < upper - 1
    return dict(version=revision, items=items, scanned=scanned, next_after=last if more else None)
