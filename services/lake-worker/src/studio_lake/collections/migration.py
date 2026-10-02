"""Repair the derived indexes omitted by early development builds marked v8.

Called only while the control upgrade owns the daemon and every execution lock.
Facts remain in the archive; no network, publication or job admission happens here.
"""

import json
from pathlib import Path

import pyarrow.parquet as pq

from ..media_lake.library import MediaLibrary, verify_manifest, replay_receipt
from ..util import contained, read_json


def repair_indexes(db):
    names = {r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
    missing_edges = "collection_discovery_edges" not in names
    if "expanded_depth" not in {r[1] for r in db.execute("PRAGMA table_info(collection_entities)")}:
        db.execute("ALTER TABLE collection_entities ADD COLUMN expanded_depth INTEGER CHECK(expanded_depth>=0)")
    # Keep the canonical v8 definitions in one place, including its triggers.
    source = Path(__file__).with_name("schema.sql").read_text(encoding="utf-8")
    derived = source[source.index("-- Rebuildable control indexes"):]
    for statement in ("CREATE TABLE", "CREATE INDEX", "CREATE TRIGGER"):
        derived = derived.replace(statement, statement + " IF NOT EXISTS")
    db.executescript(derived)
    db.execute("DELETE FROM collection_counts")
    db.execute("INSERT INTO collection_counts SELECT job_id,kind,state,count(*) FROM collection_tasks GROUP BY job_id,kind,state")
    if not missing_edges:
        return
    libraries = {}
    affected = set()
    rows = db.execute("""SELECT a.lake_id,a.batch_id,a.job_id,l.media,j.definition_json FROM collection_applied_batches a
        JOIN lakes l ON l.id=a.lake_id JOIN collection_jobs j ON j.id=a.job_id ORDER BY a.lake_id,a.seq""")
    for row in rows:
        if row["lake_id"] not in libraries:
            libraries[row["lake_id"]] = MediaLibrary.archive(row["media"])
        lib = libraries[row["lake_id"]]
        directory = contained(lib.root, "segments/" + row["batch_id"])
        manifest = read_json(directory / "manifest.json")
        verify_manifest(lib, directory, manifest, hash_media=False)
        records = {}
        for name in ("discovery_snapshots", "discovery_members", "work_observations"):
            path = name + ".parquet"
            records[name] = pq.read_table(directory / path).to_pylist() if path in manifest["files"] else []
        edges = []
        for snapshot in records["discovery_snapshots"]:
            for member in records["discovery_members"]:
                if member["snapshot_id"] == snapshot["snapshot_id"]:
                    edges.append((row["job_id"], snapshot["root_kind"], snapshot["root_id"], member["target_kind"], member["target_id"],
                                  0 if snapshot["relation"] == "author_works" else 1, snapshot["snapshot_id"]))
        if json.loads(row["definition_json"])["seeds"]["kind"] == "authors":
            edges.extend((row["job_id"], "work", o["work_id"], "author", o["author_id"], 0, o["observation_id"])
                         for o in records["work_observations"] if o["author_id"])
            for outcome in replay_receipt(directory, manifest)["task_outcomes"]:
                summary = outcome.get("summary", {})
                retained = summary.get("retained_works", []) + ([summary["retained_work"]] if summary.get("retained_work") else [])
                edges.extend((row["job_id"], "work", v["work_id"], "author", v["author_id"], 0, v["observation_id"])
                             for v in retained if v["author_id"])
        db.executemany("INSERT INTO collection_discovery_edges VALUES(?,?,?,?,?,?,?) ON CONFLICT DO NOTHING", edges)
        affected.add(row["job_id"])
    for job_id in affected:
        db.execute("""UPDATE collection_entities SET expanded_depth=NULL,provenance_count=(SELECT count(*) FROM collection_discovery_edges e
            WHERE e.job_id=collection_entities.job_id AND e.target_kind=collection_entities.kind AND e.target_id=collection_entities.source_id)
            WHERE job_id=?""", (job_id,))
