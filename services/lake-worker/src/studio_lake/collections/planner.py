"""Deterministic, archive-driven author frontier and task dependencies."""

import json

from ..image_policy import profile_id
from ..media_lake.schema import canonical, utc
from ..util import IntegrityError, stable_id


def entity(db, job_id, kind, identity, depth):
    old = db.execute("SELECT * FROM collection_entities WHERE job_id=? AND kind=? AND source_id=?", (job_id, kind, identity)).fetchone()
    if old is None:
        db.execute("INSERT INTO collection_entities(job_id,kind,source_id,min_depth,state) VALUES(?,?,?,?,'candidate')", (job_id, kind, identity, depth))
        return True
    if depth < old["min_depth"]:
        db.execute("UPDATE collection_entities SET min_depth=?,state=CASE WHEN state='excluded' THEN 'candidate' ELSE state END WHERE job_id=? AND kind=? AND source_id=?", (depth, job_id, kind, identity))
        return True
    return False


def task(db, job_id, kind, subject, payload, *, key=None, priority=0, parent=None):
    key = key or kind + ":" + subject
    identity = stable_id("collection-task-v1", job_id, key)
    old = db.execute("SELECT payload_json FROM collection_tasks WHERE job_id=? AND task_key=?", (job_id, key)).fetchone()
    if old:
        # Depth/provenance are entity state, never a mutable task input.
        if old[0] != canonical(payload):
            raise IntegrityError("A frozen collection task was planned with different input")
        return identity
    at = utc()
    db.execute("""INSERT INTO collection_tasks(id,job_id,kind,subject_key,task_key,payload_json,state,priority,created_at,updated_at)
        VALUES(?,?,?,?,?,?,'queued',?,?,?)""", (identity, job_id, kind, subject, key, canonical(payload), priority, at, at))
    if parent:
        # The parent already has an archived result. Preserve the causal edge;
        # adding dependencies to an existing task is intentionally forbidden.
        db.execute("INSERT INTO collection_dependencies VALUES(?,?,?,'archived',1)", (job_id, parent, identity))
    return identity


def edge(db, job, source_kind, source_id, target_kind, target_id, step, snapshot_id):
    current = db.execute("SELECT min_depth FROM collection_entities WHERE job_id=? AND kind=? AND source_id=?", (job["id"], source_kind, source_id)).fetchone()
    if current is None:
        raise IntegrityError("Discovery root is not in the collection intent")
    changed = db.execute("INSERT INTO collection_discovery_edges VALUES(?,?,?,?,?,?,?) ON CONFLICT DO NOTHING",
                        (job["id"], source_kind, source_id, target_kind, target_id, step, snapshot_id)).rowcount
    entity(db, job["id"], target_kind, target_id, current[0] + step)
    if changed:
        db.execute("UPDATE collection_entities SET provenance_count=provenance_count+1 WHERE job_id=? AND kind=? AND source_id=?", (job["id"], target_kind, target_id))


def scope_allows(detail, spec):
    scope = spec["scope"]
    fields = json.loads(detail["source_fields_json"])["pixiv"]
    work_type, restrict, ai = detail["work_type"], fields["x_restrict"], fields["ai_type"]
    if work_type not in scope["work_types"] and not (work_type == "unknown" and scope["include_unknown_markers"]):
        return False
    rating = {0: "all_ages", 1: "r18", 2: "r18g"}.get(restrict)
    if rating is None and not scope["include_unknown_markers"]:
        return False
    if rating is not None and rating not in scope["ratings"]:
        return False
    if ai == 2 and not scope["include_ai"]:
        return False
    return ai in {1, 2} or scope["include_unknown_markers"]


def apply(db, job, records, outcomes):
    spec = json.loads(job["definition_json"])
    parents = {o["task_id"]: db.execute("SELECT kind,subject_key FROM collection_tasks WHERE id=? AND job_id=?", (o["task_id"], job["id"])).fetchone() for o in outcomes}
    def parent(kind, subject):
        return next((identity for identity, row in parents.items() if row and row[0] == kind and row[1] == subject), None)

    for snapshot in records.get("discovery_snapshots", []):
        members = [m for m in records.get("discovery_members", []) if m["snapshot_id"] == snapshot["snapshot_id"]]
        root_kind, root_id = snapshot["root_kind"], snapshot["root_id"]
        for member in members:
            edge(db, job, root_kind, root_id, member["target_kind"], member["target_id"],
                 0 if snapshot["relation"] == "author_works" else 1, snapshot["snapshot_id"])
        if snapshot["relation"] == "author_works":
            db.execute("UPDATE collection_entities SET state='processed' WHERE job_id=? AND kind='author' AND source_id=?", (job["id"], root_id))
            # Store only the archived snapshot reference in frozen relationship inputs.
            for entrypoint in spec["discovery"]["entrypoints"]:
                if entrypoint == "recommendations":
                    for work_id in recommendation_seeds([m["target_id"] for m in members], spec["discovery"]["recommendation_seeds_per_author"]):
                        task(db, job["id"], "relationship_page", work_id,
                             dict(root_kind="work", root_id=work_id, relation="recommendations", cursor=0, directory_snapshot_id=None),
                             key=f"relationship:{work_id}:recommendations:0", priority=-10, parent=parent("author_directory", root_id))
                    continue
                task(db, job["id"], "relationship_page", root_id,
                     dict(root_kind="author", root_id=root_id, relation=entrypoint, cursor=0, directory_snapshot_id=snapshot["snapshot_id"]),
                     key=f"relationship:{root_id}:{entrypoint}:0", priority=-10, parent=parent("author_directory", root_id))
        elif snapshot["next_cursor_json"]:
            cursor = json.loads(snapshot["next_cursor_json"])
            relation = snapshot["relation"]
            task(db, job["id"], "relationship_page", root_id,
                 dict(root_kind=root_kind, root_id=root_id, relation=relation, cursor=cursor["offset"], directory_snapshot_id=cursor.get("directory_snapshot_id")),
                 key=f"relationship:{root_id}:{relation}:{cursor['offset']}", priority=-10)
    for detail in records.get("work_observations", []):
        work_id = detail["work_id"]
        if spec["seeds"]["kind"] == "authors" and detail["author_id"]:
            edge(db, job, "work", work_id, "author", detail["author_id"], 0, detail["observation_id"])
        allowed = scope_allows(detail, spec)
        db.execute("UPDATE collection_entities SET state=? WHERE job_id=? AND kind='work' AND source_id=?", ("processed" if allowed else "excluded", job["id"], work_id))
        if allowed:
            task(db, job["id"], "media_manifest", work_id, dict(detail=detail), priority=5, parent=parent("work_detail", work_id))
    for manifest in records.get("media_manifests", []):
        if not manifest["complete"]:
            continue
        media = spec["media"]
        if media["image_policy"]["profile"] == "metadata_only" or (manifest["kind"] == "ugoira" and media["ugoira"] == "metadata_only"):
            continue
        for entry in records.get("media_entries", []):
            if entry["manifest_id"] != manifest["manifest_id"]:
                continue
            frames = [f for f in records.get("animation_frames", []) if f["media_id"] == entry["media_id"]]
            key = "media:" + entry["media_id"] + ":" + profile_id(media["image_policy"])
            task(db, job["id"], "media_download", entry["media_id"], dict(entry=entry, frames=frames), key=key, priority=10,
                 parent=parent("media_manifest", manifest["work_id"]))


def advance(db, job, *, max_steps=1000):
    """Relax stored graph edges before admissions. Never refetch a path already captured."""
    spec = json.loads(job["definition_json"])
    max_depth = spec["discovery"]["max_depth"]
    rows = list(db.execute("SELECT * FROM collection_entities WHERE job_id=? AND (expanded_depth IS NULL OR expanded_depth>min_depth) ORDER BY min_depth,kind,source_id LIMIT ?", (job["id"], max_steps)))
    for row in rows:
        if row["min_depth"] > max_depth:
            db.execute("UPDATE collection_entities SET state='excluded',expanded_depth=min_depth WHERE job_id=? AND kind=? AND source_id=?", (job["id"], row["kind"], row["source_id"]))
            continue
        if row["expanded_depth"] is not None and row["expanded_depth"] > row["min_depth"] and row["min_depth"] < max_depth:
            db.execute("UPDATE collection_tasks SET state='queued',reason=NULL WHERE job_id=? AND kind='relationship_page' AND subject_key=? AND json_extract(payload_json,'$.root_kind')=? AND state='excluded' AND reason='depth_limit'",
                       (job["id"], row["source_id"], row["kind"]))
        for link in db.execute("SELECT target_kind,target_id,depth_step FROM collection_discovery_edges WHERE job_id=? AND source_kind=? AND source_id=?", (job["id"], row["kind"], row["source_id"])):
            entity(db, job["id"], link[0], link[1], row["min_depth"] + link[2])
        db.execute("UPDATE collection_entities SET expanded_depth=min_depth WHERE job_id=? AND kind=? AND source_id=?", (job["id"], row["kind"], row["source_id"]))
    counters = json.loads(job["counters_json"])
    remaining = spec["run_budget"]["admitted_authors"] - counters["round"]["admitted_authors"]
    for row in db.execute("SELECT source_id FROM collection_entities WHERE job_id=? AND kind='author' AND state='candidate' AND min_depth<=? ORDER BY min_depth,length(source_id),source_id LIMIT ?", (job["id"], max_depth, max(0, min(remaining, max_steps)))):
        identity = row[0]
        task(db, job["id"], "author_profile", identity, dict(author_id=identity), priority=2)
        task(db, job["id"], "author_directory", identity, dict(author_id=identity), priority=1)
        db.execute("UPDATE collection_entities SET state='admitted' WHERE job_id=? AND kind='author' AND source_id=?", (job["id"], identity))
        counters["round"]["admitted_authors"] += 1
    for row in db.execute("SELECT source_id FROM collection_entities WHERE job_id=? AND kind='work' AND state='candidate' AND min_depth<=? ORDER BY min_depth,length(source_id),source_id LIMIT ?", (job["id"], max_depth, max_steps)):
        task(db, job["id"], "work_detail", row[0], dict(work_id=row[0]))
        db.execute("UPDATE collection_entities SET state='admitted' WHERE job_id=? AND kind='work' AND source_id=?", (job["id"], row[0]))
    db.execute("UPDATE collection_jobs SET counters_json=? WHERE id=?", (canonical(counters), job["id"]))


def recommendation_seeds(ids, count):
    ids = sorted(set(ids), key=int)
    count = min(len(ids), count)
    if count <= 1:
        return ids[:count]
    return [ids[i * (len(ids) - 1) // (count - 1)] for i in range(count)]
