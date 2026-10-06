"""Bounded lookups against a published snapshot, with archived reuse evidence.

Freshness never changes an earlier observation time. Missing media can be
planned from a compatible retained manifest; stale facts or access are rechecked.
"""

from contextlib import contextmanager
from datetime import datetime, timedelta, timezone
import json
import time

from .planner import scope_allows
from ..image_policy import profile_id
from ..media_lake.records import one
from ..media_lake.content import compatible
from ..media_lake.schema import canonical, arrow_schema
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


def complete_media(required, sequence):
    clauses, values = [], []
    for representation, recipe in required:
        clauses.append("EXISTS(SELECT 1 FROM assets a JOIN objects o USING(sha256) WHERE a.media_id=m.media_id "
                       "AND a.representation=? AND a.recipe_id=? AND a.commit_seq<=? AND o.first_seq<=?)")
        values.extend((representation, recipe, sequence, sequence))
    return " AND ".join(clauses) or "1", values


def dictionaries(db, query, args):
    cursor = db.execute(query, args)
    row = cursor.fetchone()
    if row is None:
        return
    names = [column[0] for column in cursor.get_description()]
    while row is not None:
        yield dict(zip(names, row))
        row = cursor.fetchone()


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
        tags = [r[0] for r in db.execute("SELECT t.tag FROM work_tags w JOIN tags t USING(tag_id) WHERE w.observation_id=? ORDER BY w.ordinal", (detail["observation_id"],))] if any((spec["scope"].get("tags") or {}).values()) else None
        retained = dict(work_id=identity, author_id=detail["author_id"], observation_id=detail["observation_id"],
                        manifest_id=detail["manifest_id"], observed_at=detail["observed_at"], served_seq=seq,
                        comparison_key=context["comparison_key"], media_count=0,
                        generation=db.execute("SELECT value FROM online_state WHERE key='generation'").fetchone()[0])
        if not scope_allows(detail, spec, tags):
            # Known tag rejections obey the requested freshness window too.
            # Unknown markers or different access still require another capture.
            query = spec["scope"].get("tags") or {}
            plain = {**spec, "scope": {k: v for k, v in spec["scope"].items() if k != "tags"}}
            known = json.loads(detail["source_fields_json"])["pixiv"].get("tags_known", bool(tags))
            return {**retained, "excluded": True} if any(query.values()) and known and scope_allows(detail, plain) else None
        if detail["manifest_state"] != "ready":
            return {**retained, "fetch_manifest": True}
        manifest = one(db, "SELECT m.*,c.comparison_key FROM media_manifests m JOIN visibility_contexts c USING(context_id) WHERE m.manifest_id=? AND m.commit_seq<=?",
                       (detail["manifest_id"], seq))
        if not manifest or not manifest["complete"] or manifest["observed_at"] < cutoff or manifest["comparison_key"] != context["comparison_key"]:
            return {**retained, "fetch_manifest": True}
        # Also guard online projections produced before the compatibility rule existed.
        if not compatible(db, detail, manifest):
            return {**retained, "fetch_manifest": True}
        required = requirements(spec, detail["work_type"])
        count = db.execute("SELECT count(*) FROM media_entries WHERE manifest_id=? AND commit_seq<=?", (manifest["manifest_id"], seq)).fetchone()[0]
        if count != manifest["item_count"]:
            return None
        complete, args = complete_media(required, seq)
        retained_count = db.execute("SELECT count(*) FROM media_entries m WHERE m.manifest_id=? AND m.commit_seq<=? AND " + complete,
                                    (manifest["manifest_id"], seq, *args)).fetchone()[0] if required else 0
        if required:
            variants = " OR ".join("(a.representation=? AND a.recipe_id=?)" for _ in required)
            packs = db.execute("SELECT o.pack_path,max(o.offset+o.length) FROM media_entries m "
                               "JOIN assets a USING(media_id) JOIN objects o USING(sha256) WHERE m.manifest_id=? "
                               "AND a.commit_seq<=? AND o.first_seq<=? AND (" + variants + ") AND " + complete + " GROUP BY o.pack_path",
                               (manifest["manifest_id"], seq, seq, *(v for pair in required for v in pair), *args))
            for pack, minimum_size in packs:
                path = contained(lib.root, pack)
                if not path.is_file() or path.stat().st_size < minimum_size:
                    raise UpdateError("COLLECTION_INTEGRITY", "Published media pack is missing or truncated; restore the archive before reuse")
        return {**retained, "media_count": retained_count,
                "materialize_manifest": bool(required and retained_count < count),
                "requirements": [dict(representation=r, recipe_id=p) for r, p in required]}


def retained_detail(lib, retained, context):
    with snapshot(lib) as (db, current):
        generation = db.execute("SELECT value FROM online_state WHERE key='generation'").fetchone()[0]
        if generation != retained["generation"] or retained["served_seq"] > current or retained["comparison_key"] != context["comparison_key"]:
            raise UpdateError("COLLECTION_SCOPE_CHANGED", "Retained detail access or generation changed")
        detail = one(db, "SELECT o.*,v.comparison_key FROM work_observations o JOIN captures c USING(capture_id) "
                     "JOIN visibility_contexts v USING(context_id) WHERE o.observation_id=? AND o.commit_seq<=?",
                     (retained["observation_id"], retained["served_seq"]))
        if not detail or detail["work_id"] != retained["work_id"] or detail["comparison_key"] != context["comparison_key"]:
            raise UpdateError("COLLECTION_INTEGRITY", "Retained detail differs from its capture")
        return {key: detail[key] for key in arrow_schema("work_observations").names}


def manifest_page(lib, retained, spec, context, after):
    """Plan only missing media from a retained immutable manifest, without HTTP."""
    if not isinstance(after, int) or isinstance(after, bool) or after < -1:
        raise UpdateError("COLLECTION_PROTOCOL", "Invalid retained manifest position")
    with snapshot(lib) as (db, current):
        generation = db.execute("SELECT value FROM online_state WHERE key='generation'").fetchone()[0]
        seq = retained["served_seq"]
        if generation != retained["generation"] or seq > current or retained["comparison_key"] != context["comparison_key"]:
            raise UpdateError("COLLECTION_SCOPE_CHANGED", "Retained manifest access or generation changed")
        detail = one(db, "SELECT * FROM work_observations WHERE observation_id=? AND commit_seq<=?", (retained["observation_id"], seq))
        manifest = one(db, "SELECT m.*,c.comparison_key FROM media_manifests m JOIN visibility_contexts c USING(context_id) WHERE manifest_id=? AND m.commit_seq<=?",
                       (retained["manifest_id"], seq))
        if (not detail or not manifest or not manifest["complete"] or manifest["work_id"] != retained["work_id"]
                or manifest["comparison_key"] != context["comparison_key"] or not compatible(db, detail, manifest)):
            raise UpdateError("COLLECTION_INTEGRITY", "Retained manifest no longer matches its captured detail")
        required = requirements(spec, detail["work_type"])
        complete, args = complete_media(required, seq)
        entries = list(dictionaries(db, "SELECT m.* FROM media_entries m WHERE manifest_id=? AND ordinal>? "
                       "AND m.commit_seq<=? AND NOT (" + complete + ") ORDER BY ordinal LIMIT 65",
                       (manifest["manifest_id"], after, seq, *args)))
        values, size = [], 0
        entry_fields, frame_fields = arrow_schema("media_entries").names, arrow_schema("animation_frames").names
        for row in entries[:64]:
            entry = {key: row[key] for key in entry_fields}
            frames = []
            for frame in dictionaries(db, "SELECT * FROM animation_frames WHERE media_id=? ORDER BY ordinal LIMIT 100001", (entry["media_id"],)):
                item = {key: frame[key] for key in frame_fields}
                size += len(canonical(item).encode())
                if len(frames) >= 100000 or size > 16 * 1024**2:
                    raise UpdateError("COLLECTION_LIMIT", "Retained animation metadata exceeds its planning budget")
                frames.append(item)
            size += len(canonical(entry).encode())
            if size > 16 * 1024**2:
                raise UpdateError("COLLECTION_LIMIT", "Retained manifest page exceeds its planning budget")
            values.append(dict(entry=entry, frames=frames))
    return dict(work_id=retained["work_id"], manifest_id=retained["manifest_id"],
                members=values, next_after=values[-1]["entry"]["ordinal"] if len(entries) > 64 else None)


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
            changed = db.execute("UPDATE collection_tasks SET state=?,reason=?,summary_json=? WHERE id=? AND state='queued'",
                                 ("excluded" if retained.get("excluded") else "done", "scope_filter_cached" if retained.get("excluded") else "fresh_existing_snapshot",
                                  json.dumps(summary, sort_keys=True, separators=(",", ":")), identity)).rowcount
            if changed:
                task = db.execute("SELECT * FROM collection_tasks WHERE id=?", (identity,)).fetchone()
                apply_summary(db, job, task, summary, counters)


def apply_summary(db, job, task, summary, counters):
    reused = summary.get("retained_work")
    if reused:
        from .planner import edge, task as plan_task

        if reused["work_id"] != task["subject_key"] or task["kind"] != "work_detail":
            raise ValueError("Reused work does not match its archived task")
        spec = json.loads(job["definition_json"])
        if spec["seeds"]["kind"] == "authors" and reused["author_id"]:
            edge(db, job, "work", reused["work_id"], "author", reused["author_id"], 0, reused["observation_id"])
        db.execute("UPDATE collection_entities SET state=? WHERE job_id=? AND kind='work' AND source_id=?", ("excluded" if reused.get("excluded") else "processed", job["id"], reused["work_id"]))
        counters["retained_works"] = counters.get("retained_works", 0) + 1
        counters["retained_media"] = counters.get("retained_media", 0) + reused["media_count"]
        if reused.get("materialize_manifest"):
            plan_task(db, job["id"], "media_manifest", reused["work_id"],
                      dict(retained_manifest=reused, after_ordinal=-1),
                      key=f"retained-manifest:{reused['manifest_id']}:-1", priority=5, parent=task["id"])
        elif reused.get("fetch_manifest"):
            plan_task(db, job["id"], "media_manifest", reused["work_id"], dict(retained_detail=reused),
                      key="retained-detail-manifest:" + reused["observation_id"], priority=5, parent=task["id"])
    page = summary.get("retained_manifest_page")
    if page:
        from .planner import task as plan_task

        payload = json.loads(task["payload_json"])
        retained = payload.get("retained_manifest")
        if (task["kind"] != "media_manifest" or not retained or page["manifest_id"] != retained["manifest_id"]
                or page["work_id"] != task["subject_key"] or len(page["members"]) > 64):
            raise ValueError("Retained manifest receipt does not match its frozen task")
        spec = json.loads(job["definition_json"])
        for member in page["members"]:
            entry = member["entry"]
            if entry["manifest_id"] != page["manifest_id"] or entry["work_id"] != page["work_id"]:
                raise ValueError("Retained media entry escaped its manifest")
            plan_task(db, job["id"], "media_download", entry["media_id"], member,
                      key="media:" + entry["media_id"] + ":" + profile_id(spec["media"]["image_policy"]),
                      priority=10, parent=task["id"])
        if page["next_after"] is not None:
            if not page["members"] or page["next_after"] <= payload["after_ordinal"]:
                raise ValueError("Retained manifest did not advance")
            plan_task(db, job["id"], "media_manifest", page["work_id"],
                      dict(retained_manifest=retained, after_ordinal=page["next_after"]),
                      key=f"retained-manifest:{page['manifest_id']}:{page['next_after']}", priority=5, parent=task["id"])
    delta = summary.get("directory_delta")
    if delta:
        totals = counters.setdefault("directory_delta", dict(added=0, no_longer_listed=0, unchanged=0))
        for key in totals:
            totals[key] += delta.get(key, 0)
