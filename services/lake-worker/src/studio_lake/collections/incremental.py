"""Bounded lookups against a published snapshot, with archived reuse evidence.

Freshness never changes the observation time of an earlier capture. A missing
recipe, page or comparable visibility context always causes a fresh request.
"""

from contextlib import contextmanager
from datetime import datetime, timedelta, timezone
import json
import time

from .planner import scope_allows
from ..image_policy import profile_id
from ..media_lake.records import one
from ..online_storage import connect
from ..updates.sites import UpdateError
from ..util import contained, read_json


@contextmanager
def snapshot(lib):
    pointer = read_json(lib.cache / "ONLINE.json")
    db = connect(contained(lib.cache, pointer["file"]))
    try:
        db.execute("BEGIN")
        seq = int(db.execute("SELECT value FROM online_state WHERE key='served_seq'").fetchone()[0])
        yield db, seq
    finally:
        db.close()


def requirements(spec, kind):
    media = spec["media"]
    profile = media["image_policy"]["profile"]
    if profile == "metadata_only" or (kind == "ugoira" and media["ugoira"] == "metadata_only"):
        return []
    result = [("original", "original")] if media["retain_original"] or kind == "ugoira" else []
    if kind == "ugoira":
        result.append(("poster", "ugoira-poster-v1"))
    elif profile != "original":
        result.append(("derived", profile_id(media["image_policy"])))
    return result


def reusable_work(lib, identity, spec, context, *, at=None):
    refresh = spec.get("refresh", {})
    if refresh.get("mode") != "missing_or_stale":
        return None
    cutoff = ((at or datetime.now(timezone.utc)) - timedelta(hours=refresh["max_age_hours"])).isoformat().replace("+00:00", "Z")
    with snapshot(lib) as (db, seq):
        detail = one(db, """SELECT o.*,v.manifest_id,v.manifest_state,c.comparison_key
            FROM work_versions v JOIN work_observations o ON o.observation_id=v.observation_id
            JOIN captures p ON p.capture_id=o.capture_id JOIN visibility_contexts c ON c.context_id=p.context_id
            WHERE v.work_id=? AND v.valid_from<=? AND (v.valid_until IS NULL OR v.valid_until>?)""", (identity, seq, seq))
        if not detail or detail["observed_at"] < cutoff or detail["comparison_key"] != context["comparison_key"]:
            return None
        # An excluded work is rechecked too: a cached marker must not silently
        # suppress a work that has become visible or moved into the requested scope.
        if not scope_allows(detail, spec) or detail["manifest_state"] != "ready":
            return None
        manifest = one(db, "SELECT m.*,c.comparison_key FROM media_manifests m JOIN visibility_contexts c USING(context_id) WHERE m.manifest_id=? AND m.commit_seq<=?",
                       (detail["manifest_id"], seq))
        if not manifest or not manifest["complete"] or manifest["observed_at"] < cutoff or manifest["comparison_key"] != context["comparison_key"]:
            return None
        required = requirements(spec, detail["work_type"])
        count = db.execute("SELECT count(*) FROM media_entries WHERE manifest_id=? AND commit_seq<=?", (manifest["manifest_id"], seq)).fetchone()[0]
        if count != manifest["item_count"]:
            return None
        for representation, recipe in required:
            # Reject absent files/recipes, without loading every page or asset ID.
            missing = db.execute("""SELECT 1 FROM media_entries m WHERE m.manifest_id=? AND m.commit_seq<=? AND NOT EXISTS(
                SELECT 1 FROM media_asset_versions v JOIN assets a USING(asset_id) JOIN objects o USING(sha256)
                WHERE v.media_id=m.media_id AND v.representation=? AND v.recipe_id=?
                  AND v.valid_from<=? AND (v.valid_until IS NULL OR v.valid_until>?) AND a.commit_seq<=? AND o.first_seq<=?) LIMIT 1""",
                                 (manifest["manifest_id"], seq, representation, recipe, seq, seq, seq, seq)).fetchone()
            if missing:
                return None
        if required:
            packs = db.execute("""SELECT o.pack_path,max(o.offset+o.length) FROM media_entries m JOIN media_asset_versions v USING(media_id)
                JOIN assets a USING(asset_id) JOIN objects o USING(sha256) WHERE m.manifest_id=? AND v.valid_from<=?
                AND (v.valid_until IS NULL OR v.valid_until>?) GROUP BY o.pack_path""", (manifest["manifest_id"], seq, seq))
            for pack, minimum_size in packs:
                path = contained(lib.root, pack)
                if not path.is_file() or path.stat().st_size < minimum_size:
                    raise UpdateError("COLLECTION_INTEGRITY", "Published media pack is missing or truncated; restore the archive before reuse")
        return dict(work_id=identity, author_id=detail["author_id"], observation_id=detail["observation_id"],
                    manifest_id=manifest["manifest_id"], observed_at=detail["observed_at"], served_seq=seq,
                    comparison_key=context["comparison_key"], media_count=count if required else 0,
                    requirements=[dict(representation=r, recipe_id=p) for r, p in required])


def directory_delta(lib, records, context):
    current = records["discovery_snapshots"][0]
    with snapshot(lib) as (db, seq):
        old = one(db, """SELECT s.snapshot_id,s.observed_at FROM discovery_snapshots s JOIN visibility_contexts c USING(context_id)
            WHERE s.root_kind='author' AND s.root_id=? AND s.relation='author_works' AND s.page_complete=1
              AND s.traversal_exhausted=1 AND s.commit_seq<=? AND c.comparison_key=?
            ORDER BY s.observed_at DESC,s.snapshot_id DESC LIMIT 1""", (current["root_id"], seq, context["comparison_key"]))
        latest = {r["target_id"] for r in records.get("discovery_members", [])}
        # One profile/all response already has the same 32 MiB acquisition cap.
        # Limit the comparison too, for archives imported from a different producer.
        previous = set()
        if old:
            previous = {r[0] for r in db.execute("SELECT target_id FROM discovery_members WHERE snapshot_id=? LIMIT 100001", (old["snapshot_id"],))}
            if len(previous) > 100000:
                return dict(comparable=False, reason="comparison_limit", observed=len(latest))
        return dict(comparable=old is not None, previous_snapshot_id=old["snapshot_id"] if old else None,
                    snapshot_id=current["snapshot_id"], observed=len(latest), added=len(latest - previous),
                    no_longer_listed=len(previous - latest), unchanged=len(previous & latest),
                    missing_sample=sorted(previous - latest, key=int)[:20])


def directory_reuse(lib, records, spec, context, cancelled, *, limit=256):
    """Satisfy bounded fresh children with the directory's one archived receipt.

The individual work tasks are still materialized during control replay. A
large or slow directory leaves the rest to the normal resumable task loop.
"""
    if spec.get("refresh", {}).get("mode") != "missing_or_stale":
        return []
    values, started = [], time.monotonic()
    for member in records.get("discovery_members", [])[:limit]:
        if cancelled() or time.monotonic() - started >= 2:
            break
        retained = reusable_work(lib, member["target_id"], spec, context)
        if retained:
            values.append(retained)
    return values


def apply_directory_reuse(db, job, records, outcomes, counters):
    from .planner import task as plan_task

    members = {r["target_id"] for r in records.get("discovery_members", []) if r["target_kind"] == "work"}
    for outcome in outcomes:
        values = outcome.get("summary", {}).get("retained_works", [])
        if not values:
            continue
        parent = db.execute("SELECT kind,subject_key FROM collection_tasks WHERE id=? AND job_id=?", (outcome["task_id"], job["id"])).fetchone()
        if not parent or parent[0] != "author_directory" or len(values) > 256 or any(v["work_id"] not in members for v in values):
            raise ValueError("Retained directory children do not match the archived parent")
        for retained in values:
            work_id = retained["work_id"]
            identity = plan_task(db, job["id"], "work_detail", work_id, dict(work_id=work_id), parent=outcome["task_id"])
            summary = dict(retained_work=retained, directory_receipt_task_id=outcome["task_id"])
            changed = db.execute("UPDATE collection_tasks SET state='done',reason='fresh_existing_snapshot',summary_json=? WHERE id=? AND state='queued'",
                                 (json.dumps(summary, sort_keys=True, separators=(",", ":")), identity)).rowcount
            if changed:
                task = db.execute("SELECT * FROM collection_tasks WHERE id=?", (identity,)).fetchone()
                apply_summary(db, job, task, summary, counters)


def apply_summary(db, job, task, summary, counters):
    reused = summary.get("retained_work")
    if reused:
        from .planner import edge

        if reused["work_id"] != task["subject_key"] or task["kind"] != "work_detail":
            raise ValueError("Reused work does not match its archived task")
        spec = json.loads(job["definition_json"])
        if spec["seeds"]["kind"] == "authors" and reused["author_id"]:
            edge(db, job, "work", reused["work_id"], "author", reused["author_id"], 0, reused["observation_id"])
        db.execute("UPDATE collection_entities SET state='processed' WHERE job_id=? AND kind='work' AND source_id=?", (job["id"], reused["work_id"]))
        counters["retained_works"] = counters.get("retained_works", 0) + 1
        counters["retained_media"] = counters.get("retained_media", 0) + reused["media_count"]
    delta = summary.get("directory_delta")
    if delta:
        totals = counters.setdefault("directory_delta", dict(added=0, no_longer_listed=0, unchanged=0))
        for key in totals:
            totals[key] += delta.get(key, 0)
