"""Immutable fact insertion and the common incremental/rebuild projection rules."""

import zlib

from .schema import FACTS, NORMALIZER, columns, primary_key
from ..util import IntegrityError, stable_id


def row_dict(cursor):
    names = [c[0] for c in cursor.get_description()]
    return [dict(zip(names, r)) for r in cursor]


def one(db, query, args=()):
    cursor = db.cursor().execute(query, args)
    row = cursor.fetchone()
    return dict(zip([v[0] for v in cursor.get_description()], row)) if row is not None else None


def online_row(name, row, batch):
    row = dict(row)
    if name == "captures":
        row["raw_zlib"] = zlib.compress(row.pop("raw_body"), 1)
    elif name == "objects":
        row["pack_path"] = "segments/" + batch + "/" + row.pop("pack_file")
        row.pop("member_name")
    return row


def insert_fact(db, name, row, seq, batch):
    row = online_row(name, row, batch)
    if name == "work_tags":
        tag = row.pop("tag")
        db.execute("INSERT INTO tags(tag) VALUES(?) ON CONFLICT(tag) DO NOTHING", (tag,))
        row["tag_id"] = db.execute("SELECT tag_id FROM tags WHERE tag=? COLLATE BINARY", (tag,)).fetchone()[0]
    fields, keys = columns(name), primary_key(name)
    where = " AND ".join(k + "=?" for k in keys)
    old = one(db, "SELECT * FROM " + name + " WHERE " + where, tuple(row[k] for k in keys))
    if old:
        # Physical content may be copied into a later pack; keep its first verified location.
        compare = [c for c in fields if name != "objects" or c not in {"pack_path", "offset"}]
        if any(old[c] != row[c] for c in compare):
            raise IntegrityError("Conflicting immutable " + name + " record")
        return
    names = list(row)
    if name not in {"work_tags", "animation_frames", "discovery_members"}:
        row["first_seq" if name in {"authors", "works", "objects"} else "commit_seq"] = seq
        names = list(row)
    db.execute("INSERT INTO " + name + "(" + ",".join(names) + ") VALUES(" + ",".join("?" for _ in names) + ")",
               tuple(row[n] for n in names))


def validate_relations(db, records, library_id):
    """SQL enforces local constraints; validate identities, completeness and coupled shapes."""
    for row in records.get("captures", []):
        if row["capture_id"] != stable_id("capture-v1", library_id, row["request_receipt_id"]):
            raise IntegrityError("Capture identity mismatch")
    for name, field, prefix in (("work_observations", "work_id", "work-observation-v1"),
                                ("author_observations", "author_id", "author-observation-v1")):
        for row in records.get(name, []):
            if row["observation_id"] != stable_id(prefix, row["capture_id"], row[field], row["normalizer_version"]):
                raise IntegrityError("Observation identity mismatch")
    for row in records.get("media_manifests", []):
        if row["manifest_id"] != stable_id("media-manifest-v1", row["capture_id"], row["work_id"],
                                             row["detail_observation_id"] or "", row["normalizer_version"]):
            raise IntegrityError("Manifest identity mismatch")
        members = list(db.execute("SELECT media_id,ordinal,kind FROM media_entries WHERE manifest_id=? ORDER BY ordinal", (row["manifest_id"],)))
        if len(members) != row["item_count"]:
            raise IntegrityError("Manifest count differs from its members")
        if row["complete"]:
            if not members or [m[1] for m in members] != list(range(len(members))):
                raise IntegrityError("A complete media manifest requires a nonempty contiguous sequence")
            if row["kind"] == "ugoira" and (len(members) != 1 or members[0][2] != "ugoira"):
                raise IntegrityError("An animation manifest requires exactly one animation")
            detail = one(db, "SELECT page_count,work_type FROM work_observations WHERE observation_id=?", (row["detail_observation_id"],))
            if row["kind"] == "image_pages" and detail and detail["page_count"] is not None and detail["page_count"] != len(members):
                raise IntegrityError("Detail and pages count disagree")
    for row in records.get("media_entries", []):
        if row["media_id"] != stable_id("media-entry-v1", row["manifest_id"], row["slot_key"]):
            raise IntegrityError("Media identity mismatch")
        expected = "page:" + str(row["ordinal"]) if row["kind"] == "image" else "animation:0"
        if row["slot_key"] != expected:
            raise IntegrityError("Media slot mismatch")
        if row["kind"] == "ugoira":
            frames = list(db.execute("SELECT ordinal FROM animation_frames WHERE media_id=? ORDER BY ordinal", (row["media_id"],)))
            if not frames or [f[0] for f in frames] != list(range(len(frames))):
                raise IntegrityError("Animation frame sequence is incomplete")
    for row in records.get("assets", []):
        if row["asset_id"] != stable_id("asset-record-v2", row["media_id"], row["representation"], row["recipe_id"],
                                         row["acquisition_receipt_id"], row["sha256"]):
            raise IntegrityError("Asset identity mismatch")
        if row["derived_from_asset_id"]:
            original = one(db, "SELECT media_id FROM assets WHERE asset_id=?", (row["derived_from_asset_id"],))
            if original is None or original["media_id"] != row["media_id"]:
                raise IntegrityError("Derived asset belongs to another media occurrence")
    for row in records.get("discovery_snapshots", []):
        if row["snapshot_id"] != stable_id("discovery-v1", row["capture_id"], row["stream_key"], "pixiv-plan-v1"):
            raise IntegrityError("Discovery snapshot identity mismatch")


def apply_facts(db, records, seq, batch):
    for name in FACTS:
        rows = records.get(name, [])
        if name == "assets":
            rows = sorted(rows, key=lambda r: r["derived_from_asset_id"] is not None)
        for row in rows:
            insert_fact(db, name, row, seq, batch)


def affect(db, seq, work_id):
    db.execute("""INSERT INTO changes(seq,sha256,fields_json)
        SELECT ?,a.sha256,'["metadata","work_members"]' FROM assets a JOIN media_entries m USING(media_id)
        WHERE m.work_id=? AND a.commit_seq<=? GROUP BY a.sha256
        ON CONFLICT(seq,sha256) DO NOTHING""", (seq, work_id, seq))


def project_work(db, work_id, seq):
    detail = one(db, """SELECT w.*,c.context_id FROM work_observations w JOIN captures c USING(capture_id)
        WHERE w.work_id=? AND w.commit_seq<=? AND w.normalizer_version=?
        ORDER BY w.observed_at DESC,w.observation_id DESC LIMIT 1""", (work_id, seq, NORMALIZER))
    manifest = one(db, """SELECT * FROM media_manifests WHERE work_id=? AND commit_seq<=?
        AND complete=1 AND normalizer_version=? ORDER BY observed_at DESC,manifest_id DESC LIMIT 1""", (work_id, seq, NORMALIZER))
    latest = one(db, """SELECT complete FROM media_manifests WHERE work_id=? AND commit_seq<=?
        AND normalizer_version=? ORDER BY observed_at DESC,manifest_id DESC LIMIT 1""", (work_id, seq, NORMALIZER))
    state = "missing"
    if manifest:
        state = "ready"
        if latest and not latest["complete"]:
            state = "needs_refresh"
        if detail and detail["work_type"] in {"illustration", "manga", "ugoira"}:
            expected_kind = "ugoira" if detail["work_type"] == "ugoira" else "image_pages"
            if manifest["kind"] != expected_kind:
                state = "needs_refresh"
        if detail and manifest["kind"] == "image_pages" and detail["page_count"] is not None and detail["page_count"] != manifest["item_count"]:
            state = "needs_refresh"
        if detail and detail["context_id"] != manifest["context_id"]:
            contexts = list(db.execute("SELECT comparison_key FROM visibility_contexts WHERE context_id IN (?,?)",
                                       (detail["context_id"], manifest["context_id"])))
            if len(contexts) != 2 or not contexts[0][0] or contexts[0][0] != contexts[1][0]:
                state = "visibility_changed"
    values = (detail["observation_id"] if detail else None, manifest["manifest_id"] if manifest else None, state)
    old = one(db, "SELECT * FROM work_versions WHERE work_id=? AND valid_until IS NULL", (work_id,))
    if old and tuple(old[k] for k in ("observation_id", "manifest_id", "manifest_state")) == values:
        return
    if old and old["valid_from"] == seq:
        raise IntegrityError("Inconsistent work projection replay")
    db.execute("UPDATE work_versions SET valid_until=? WHERE work_id=? AND valid_until IS NULL", (seq, work_id))
    db.execute("INSERT INTO work_versions VALUES(?,?,NULL,?,?,?)", (work_id, seq, *values))
    affect(db, seq, work_id)


def project_media(db, media_id, representation, recipe_id, seq):
    chosen = one(db, """SELECT * FROM assets WHERE media_id=? AND representation=? AND recipe_id=? AND commit_seq<=?
        ORDER BY CASE evidence WHEN 'historical_reuse' THEN 1 ELSE 2 END DESC,
        last_verified_at DESC,acquired_at DESC,asset_id DESC LIMIT 1""", (media_id, representation, recipe_id, seq))
    if not chosen:
        return
    key = (media_id, representation, recipe_id)
    old = one(db, "SELECT * FROM media_asset_versions WHERE media_id=? AND representation=? AND recipe_id=? AND valid_until IS NULL", key)
    if old and old["asset_id"] == chosen["asset_id"]:
        return
    if old and old["valid_from"] == seq:
        raise IntegrityError("Inconsistent asset projection replay")
    db.execute("UPDATE media_asset_versions SET valid_until=? WHERE media_id=? AND representation=? AND recipe_id=? AND valid_until IS NULL", (seq, *key))
    db.execute("INSERT INTO media_asset_versions VALUES(?,?,?,?,NULL,?)", (*key, seq, chosen["asset_id"]))
    work = db.execute("SELECT work_id FROM media_entries WHERE media_id=?", (media_id,)).fetchone()[0]
    affect(db, seq, work)


def project(db, records, seq, stop=None):
    works = {r["work_id"] for name in ("work_observations", "media_manifests") for r in records.get(name, [])}
    for work in sorted(works):
        with db:
            project_work(db, work, seq)
        if stop:
            stop("projection", seq)
    for key in sorted({(r["media_id"], r["representation"], r["recipe_id"]) for r in records.get("assets", [])}):
        with db:
            project_media(db, *key, seq)
    for row in records.get("work_observations", []):
        with db:
            rid = db.execute("SELECT row_id FROM work_observations WHERE observation_id=?", (row["observation_id"],)).fetchone()[0]
            tags = ["t" + format(v[0], "x") for v in db.execute("SELECT DISTINCT tag_id FROM work_tags WHERE observation_id=? ORDER BY tag_id", (row["observation_id"],))]
            db.execute("INSERT OR REPLACE INTO tag_index(rowid,tokens) VALUES(?,?)", (rid, " ".join(tags)))


def publication_row(db, seq, batch, manifest_sha, archived_at):
    old = one(db, "SELECT batch_id,manifest_sha256 FROM publications WHERE seq=?", (seq,))
    if old and (old["batch_id"], old["manifest_sha256"]) != (batch, manifest_sha):
        raise IntegrityError("Publication fingerprint changed")
    if not old:
        db.execute("INSERT INTO publications VALUES(?,?,?,'preparing',?,NULL,'{}')", (seq, batch, manifest_sha, archived_at))


def counts(db, seq):
    # Aggregate this batch only; counting the entire lake at every publication
    # would make a long acquisition quadratic.
    import json

    previous = db.execute("SELECT counts_json FROM publications WHERE seq=? AND state='published'", (seq - 1,)).fetchone()
    previous = json.loads(previous[0]) if previous else {"objects": 0, "all_objects": 0, "works": 0}
    return {"objects": previous["objects"] + db.execute("SELECT count(*) FROM objects WHERE first_seq=? AND media_category='image'", (seq,)).fetchone()[0],
            "all_objects": previous["all_objects"] + db.execute("SELECT count(*) FROM objects WHERE first_seq=?", (seq,)).fetchone()[0],
            "works": previous["works"] + db.execute("SELECT count(*) FROM works WHERE first_seq=?", (seq,)).fetchone()[0]}
