"""Bounded discovery-page coverage and immutable observation reuse across lake jobs."""

from datetime import datetime
import json
import time
import uuid

from ..raw_codec import decode
from ..util import IntegrityError, read_json, stable_id
from .sites import Site, UpdateError

DDL = """
CREATE TABLE IF NOT EXISTS source_access_epochs(site TEXT PRIMARY KEY,epoch TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS discovery_pages(
 batch_id TEXT PRIMARY KEY,lake_id TEXT NOT NULL REFERENCES lakes(id),generation TEXT NOT NULL,
 context_key TEXT NOT NULL,anchor TEXT NOT NULL,lower_id INTEGER NOT NULL,through_id INTEGER NOT NULL,
 stream_upper INTEGER NOT NULL,archive_seq INTEGER NOT NULL,observed_at TEXT NOT NULL,
 observed_unix REAL NOT NULL,record_count INTEGER NOT NULL,head_bound INTEGER NOT NULL,
 head_observed_unix REAL NOT NULL,
 CHECK(lower_id>0 AND through_id>lower_id AND through_id<=stream_upper),
 CHECK(record_count>=0 AND record_count<=200));
CREATE INDEX IF NOT EXISTS discovery_page_lookup
 ON discovery_pages(lake_id,generation,context_key,anchor,lower_id DESC,observed_unix DESC);
CREATE INDEX IF NOT EXISTS discovery_head_lookup
 ON discovery_pages(lake_id,generation,context_key,head_bound,head_observed_unix DESC);
"""


def migrate(db):
    db.executescript(DDL)
    db.executemany("INSERT OR IGNORE INTO source_access_epochs VALUES(?,?)",
                   ((site, uuid.uuid4().hex) for site in Site.URLS))


def context_key(site, epoch):
    # Opaque identities rotate on every credential change, including clear/re-add.
    # No credential material or hash of a secret is written into archive evidence.
    return stable_id("booru-discovery-context-v1", site, epoch)


def max_age(scope):
    refresh = scope.get("refresh") or {}
    return refresh.get("max_age_hours", 0) if refresh.get("mode") == "missing_or_stale" else 0


def stamp(value):
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
        if parsed.tzinfo is None:
            raise ValueError()
        return parsed.timestamp()
    except (ValueError, TypeError, AttributeError):
        raise IntegrityError("Discovery evidence has an invalid observation time") from None


def record_page(db, lake_id, source, seq, batch_id):
    page, cursor = source.get("query_page"), source.get("cursor", {})
    if not page or not cursor.get("tag_context") or not cursor.get("tag_generation"):
        return  # Older tag jobs remain readable but are not invented cache coverage.
    ids = source.get("metadata_ids", [])
    expected_next = max(ids) + 1 if ids else page["upper"]
    if (page["lower"] >= page["next_id"] or page["next_id"] != expected_next
            or any(not page["lower"] <= pid < page["upper"] for pid in ids)
            or len(ids) != len(set(ids)) or len(ids) > 200):
        raise IntegrityError("Discovery page coverage differs from its accepted metadata")
    db.execute("INSERT OR IGNORE INTO discovery_pages VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
               (batch_id, lake_id, cursor["tag_generation"], cursor["tag_context"], page["anchor"],
                page["lower"], page["next_id"], page["upper"], seq, source["observed_at"],
                stamp(source["observed_at"]), len(ids), int(cursor.get("tag_head_bound", False)),
                stamp(cursor.get("tag_head_observed_at", source["observed_at"]))))


def find_page(state, lib, context, anchor, lower, hours, *, at=None):
    if not hours:
        return None
    generation = read_json(lib.cache / "ONLINE.json")["generation"]
    cutoff = (time.time() if at is None else at) - hours * 3600
    with state.db() as db:
        # Limit inspected history before applying interval/freshness predicates.
        # A conservative cache miss costs a request, never a false completion.
        rows = [dict(r) for r in db.execute(
            "SELECT * FROM discovery_pages WHERE lake_id=? AND generation=? AND context_key=? AND anchor=? "
            "AND lower_id<=? ORDER BY lower_id DESC,observed_unix DESC LIMIT 64",
            (lib.info["library_id"], generation, context, anchor, lower))]
    eligible = [r for r in rows if r["through_id"] > lower and r["observed_unix"] >= cutoff]
    return max(eligible, key=lambda r: (r["observed_unix"], r["through_id"]), default=None)


def cached_upper(state, lib, context, hours):
    if not hours:
        return None
    generation = read_json(lib.cache / "ONLINE.json")["generation"]
    with state.db() as db:
        row = db.execute(
            "SELECT stream_upper,head_observed_unix FROM discovery_pages WHERE lake_id=? AND generation=? "
            "AND context_key=? AND head_bound=1 AND head_observed_unix>=? "
            "ORDER BY head_observed_unix DESC LIMIT 1",
            (lib.info["library_id"], generation, context, time.time() - hours * 3600)).fetchone()
    return tuple(row) if row else None


def decode_rows(rows):
    total, values = 0, []
    for row in rows:
        post_id, observation_id, observed_at, source_kind, compressed, length, sha = row
        if source_kind not in {"api_json", "api_yandere_v1", "api_gelbooru_v1"}:
            # Imported observations can reuse a proven existing asset. Missing
            # media requires a fresh API record rather than invented source JSON.
            values.append(dict(post_id=post_id, observation_id=observation_id, observed_at=observed_at,
                               raw="{}", record={}, needs_refresh=True))
            continue
        total += max(0, length)
        if total > 16 * 1024**2:
            raise UpdateError("UPDATE_RESOURCE_LIMIT", "Reused metadata exceeds one response budget")
        if compressed is None:
            raise IntegrityError("Reused API payload is missing or exceeds its storage bound")
        raw = decode(compressed, length, sha, maximum_bytes=16 * 1024**2)
        record = json.loads(raw)
        if not isinstance(record, dict) or record.get("id") != post_id:
            raise IntegrityError("Reused source record does not match its post identity")
        values.append(dict(post_id=post_id, observation_id=observation_id, observed_at=observed_at,
                           raw=raw, record=record))
    return values


RAW_COLUMNS = ("o.post_id,o.observation_id,o.observed_at,o.source_kind,"
               "CASE WHEN o.source_kind IN ('api_json','api_yandere_v1','api_gelbooru_v1') "
               "AND r.raw_bytes BETWEEN -1 AND 16777216 AND length(r.raw_zlib)<=16842752 THEN r.raw_zlib END,"
               "r.raw_bytes,r.raw_sha256")


def page_records(lib, page, lower, upper, limit):
    from .archive import online

    with online(lib) as (db, status):
        if status["generation"] != page["generation"]:
            raise UpdateError("SOURCE_CHANGED", "Discovery cache belongs to another online generation")
        values = decode_rows(db.execute(
            f"SELECT {RAW_COLUMNS} FROM observations o JOIN raw_metadata r USING(observation_id) "
            "WHERE o.commit_seq=? AND o.batch_id=? ORDER BY o.post_id LIMIT 201",
            (page["archive_seq"], page["batch_id"])))
    if len(values) != page["record_count"]:
        raise IntegrityError("Discovery cache is missing accepted observations")
    values = [v for v in values if lower <= v["post_id"] < upper]
    if any(v.get("needs_refresh") for v in values):
        raise IntegrityError("Discovery coverage refers to an observation without its API record")
    through = min(page["through_id"], upper)
    if len(values) > limit:
        values = values[:limit]
        through = values[-1]["post_id"] + 1
    return values, through


def observation_records(lib, references):
    from .archive import online

    if len(references) > 200 or len({r["observation_id"] for r in references}) != len(references):
        raise IntegrityError("Reused observation receipt is not a bounded unique batch")
    ids = [r["observation_id"] for r in references]
    with online(lib) as (db, _):
        values = decode_rows(db.execute(
            f"SELECT {RAW_COLUMNS} FROM observations o JOIN raw_metadata r USING(observation_id) "
            "WHERE o.observation_id IN (SELECT value FROM json_each(?)) ORDER BY o.post_id",
            (json.dumps(ids),)))
    if {(r["post_id"], r["observation_id"]) for r in references} != {(r["post_id"], r["observation_id"]) for r in values}:
        raise IntegrityError("Reused observation receipt cannot be resolved")
    return values


def commit_reuse(state, lib, job, records, selected, cursor, *, page=None, publication=None):
    from contextlib import nullcontext
    from ..library import Batch
    from .archive import checkpoint_key, io_lock, reconcile

    refs = [dict(post_id=r["post_id"], observation_id=r["observation_id"], selected=r["post_id"] in selected) for r in records]
    key = stable_id("update-observation-reuse-v1", job["id"], cursor, refs)
    with publication or nullcontext():
        with io_lock(state.root, lib), lib.writer_lock():
            if not lib.committed_key(key):
                batch = Batch(lib, key, dict(kind="update_reuse", update_job_id=job["id"], update_role="reuse",
                                            definition=job["definition"], cursor=cursor, reused_observations=refs,
                                            reused_page=page["batch_id"] if page else None))
                batch.commit(settings={checkpoint_key(job["id"]): dict(definition=job["definition"], cursor=cursor)})
        reconcile(state, lib, job["id"])
