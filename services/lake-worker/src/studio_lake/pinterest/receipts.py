"""Durable outcomes and idempotent control replay after authoritative journal acceptance."""

import base64
import json

from . import garbage, model
from .http import Response
from ..canonical import canonical, utc
from ..util import IntegrityError, atomic_json, digest, read_json, stable_id


def directory(lib, job):
    result = lib.cache / "pinterest_receipts" / model.identity(job)
    result.mkdir(parents=True, exist_ok=True)
    return result


def save_response(root, identity, response):
    atomic_json(root / (identity + ".response.json"), dict(**response.metadata(), raw_body=base64.b64encode(response.body).decode("ascii")))


def load_response(root, identity):
    path = root / (identity + ".response.json")
    if not path.exists():
        return None
    if path.stat().st_size > 12 * 1024**2:
        raise IntegrityError("Pinterest response receipt exceeds its byte budget")
    value = read_json(path)
    return Response(body=base64.b64decode(value.pop("raw_body"), validate=True), **value)


def save_prepared(root, replay, records, media=None, *, job=None, task=None, spool=None):
    records = {k: [dict(r) for r in rows] for k, rows in records.items()}
    for capture in records.get("captures", []):
        capture["raw_body"] = base64.b64encode(capture["raw_body"]).decode("ascii")
    value = dict(replay=replay, records=records, media=media)
    if job is not None:
        value.update(version=2, job_id=job["id"], definition_sha256=job["definition_sha256"],
                     input_sha256=digest(task["input_json"].encode()))
    size = len(canonical(value).encode())
    if size > 48 * 1024**2:
        raise IntegrityError("Pinterest prepared receipt exceeds its byte budget")
    if spool is not None:
        spool.metadata(replay["receipt_id"], size * 2 + 64 * 1024)
    atomic_json(root / (replay["receipt_id"] + ".prepared.json"), value)


def load_prepared(root, identity):
    path = root / (identity + ".prepared.json")
    if not path.exists():
        return None
    if path.stat().st_size > 48 * 1024**2:
        raise IntegrityError("Pinterest prepared receipt exceeds its byte budget")
    value = read_json(path)
    if value["replay"]["receipt_id"] != identity:
        raise IntegrityError("Pinterest prepared identity differs")
    for capture in value["records"].get("captures", []):
        capture["raw_body"] = base64.b64decode(capture["raw_body"], validate=True)
    return value


def task(db, job_id, kind, pin_id, payload):
    identity = stable_id("pinterest-task-v1", job_id, kind, payload)
    db.execute("""INSERT INTO pinterest_tasks(task_id,job_id,kind,pin_id,input_json,state,updated_at)
        VALUES(?,?,?,?,?,'queued',?) ON CONFLICT(task_id) DO NOTHING""", (identity, job_id, kind, pin_id, canonical(payload), utc()))


def replay(state, lib, job_id):
    """Only read the unapplied suffix; task claims are never overwritten by an older receipt."""
    with state.db() as db:
        after = db.execute("SELECT archive_seq FROM pinterest_jobs WHERE id=?", (job_id,)).fetchone()[0]
    with lib.journal() as journal:
        sequences = [r[0] for r in journal.execute("SELECT DISTINCT seq FROM pinterest_receipts WHERE job_id=? AND seq>? ORDER BY seq LIMIT 128", (job_id, after))]
    for seq in sequences:
        with lib.journal() as journal:
            rows = [dict(r) for r in journal.execute("SELECT * FROM pinterest_receipts WHERE job_id=? AND seq=? ORDER BY rowid LIMIT 65", (job_id, seq))]
        if not 1 <= len(rows) <= 64:
            raise IntegrityError("Pinterest physical receipt group exceeds its budget")
        with state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            for row in rows:
                if not db.execute("SELECT 1 FROM pinterest_applied WHERE receipt_id=?", (row["receipt_id"],)).fetchone():
                    _apply(db, row, job_id)
                db.execute("UPDATE pinterest_outbox SET state='accepted',archive_seq=? WHERE receipt_id=? AND state='prepared'", (seq, row["receipt_id"]))
            db.execute("UPDATE pinterest_batches SET state='accepted',archive_seq=? WHERE batch_id=(SELECT batch_id FROM pinterest_outbox WHERE receipt_id=?) AND state='prepared'", (seq, rows[0]["receipt_id"]))
            db.execute("UPDATE pinterest_jobs SET archive_seq=max(archive_seq,?),updated_at=? WHERE id=?", (seq, utc(), job_id))
    return len(sequences)


def _apply(db, row, job_id):
    value = json.loads(row["replay_json"])
    for stream in value.get("streams", []):
        db.execute("""INSERT INTO pinterest_streams(scan_id,job_id,entrypoint,subject_id,root_json,
            parameters_json,depth,state,updated_at) VALUES(?,?,?,?,?,?,?,'active',?)
            ON CONFLICT(scan_id) DO NOTHING""", (stream["scan_id"], job_id, stream["entrypoint"], stream["subject_id"],
            canonical(stream["root"]), canonical(stream["parameters"]), stream["depth"], utc()))
    for hint in value.get("stream_totals", []):
        total = hint["source_total"]
        db.execute("UPDATE pinterest_streams SET reported_total=?,parameters_json=json_set(parameters_json,'$.source_total',json(?)) WHERE scan_id=? AND job_id=?",
                   (total["count"], canonical(total), hint["scan_id"], job_id))
    # Older accepted pages lack stream_pins; their queued admission tasks carry the same identities.
    pins = {(p["scan_id"], p["pin_id"]) for p in value.get("stream_pins", [])}
    pins.update((p["input"]["scan_id"], p["pin_id"]) for p in value["next_tasks"]
                if p["kind"] == "pin_admit" and p["input"].get("scan_id"))
    db.executemany("INSERT INTO pinterest_stream_pins VALUES(?,?) ON CONFLICT DO NOTHING", sorted(pins))
    sections = {(p["scan_id"], p["section_id"]) for p in value.get("stream_sections", [])}
    db.executemany("INSERT INTO pinterest_stream_sections VALUES(?,?) ON CONFLICT DO NOTHING", sorted(sections))
    for depth in value.get("expansion_depths", []):
        db.execute("INSERT INTO pinterest_expansion_depths VALUES(?,?,?,?,?) ON CONFLICT(job_id,root_json,kind,source_id) DO UPDATE SET min_depth=min(min_depth,excluded.min_depth)",
            (job_id, depth["root_json"], depth["kind"], depth["source_id"], depth["min_depth"]))
    for admission in value.get("admissions", []):
        db.execute("INSERT INTO pinterest_admitted VALUES(?,?,?,?) ON CONFLICT DO NOTHING",
            (job_id, admission["kind"], admission["source_id"], row["receipt_id"]))
    metrics = dict(value.get("metrics", {}))
    for item in value.get("seen", []):
        changed = db.execute("INSERT INTO pinterest_seen VALUES(?,?,?,?) ON CONFLICT DO NOTHING",
            (job_id, item["entrypoint"], item["kind"], item["value"])).rowcount
        if changed:
            name = item["entrypoint"] + ":unique_" + item["kind"]
            metrics[name] = metrics.get(name, 0) + 1
    for name, count in metrics.items():
        db.execute("INSERT INTO pinterest_metrics VALUES(?,?,?) ON CONFLICT(job_id,name) DO UPDATE SET value=value+excluded.value",
                   (job_id, name, count))
    if "checkpoint" in value:
        point = value["checkpoint"]
        db.execute("INSERT INTO pinterest_stream_pages VALUES(?,?,?) ON CONFLICT DO NOTHING",
                   (point["scan_id"], point["page_key"], row["receipt_id"]))
        db.execute("UPDATE pinterest_streams SET state=?,cursor_json=?,reason=?,pages=pages+1,members=members+?,last_turn=?,updated_at=? WHERE scan_id=? AND job_id=?",
            (point["state"], canonical(point["cursor"]), point["reason"], point["members"], row["seq"], utc(), point["scan_id"], job_id))
        if "visibility" in point:
            db.execute("UPDATE pinterest_streams SET parameters_json=json_set(parameters_json,'$.visibility',json(?)) WHERE scan_id=? AND job_id=?",
                       (canonical(point["visibility"]), point["scan_id"], job_id))
    if "sample" in value:
        sample = value["sample"]
        db.execute("UPDATE pinterest_streams SET samples_checked=samples_checked+1,mismatches=mismatches+?,force_detail=max(force_detail,?) WHERE scan_id=? AND job_id=?",
                   (int(sample["differs"]), int(sample["differs"]), sample["scan_id"], job_id))
    if "reuse_acquisition" in value:
        acquisition = value["reuse_acquisition"]
        lake = db.execute("SELECT lake_id FROM pinterest_jobs WHERE id=?", (job_id,)).fetchone()[0]
        db.execute("INSERT INTO pinterest_reuse VALUES(?,?,?) ON CONFLICT(lake_id,reuse_key) DO UPDATE SET value_json=excluded.value_json",
                   (lake, acquisition["reuse_key"], canonical(acquisition)))
    if value["kind"] == "intent":
        for item in value["next_tasks"]:
            task(db, job_id, item["kind"], item["pin_id"], item["input"])
    else:
        old = db.execute("SELECT claim_token,receipt_id,state FROM pinterest_tasks WHERE task_id=? AND job_id=?", (value["task_id"], job_id)).fetchone()
        if not old or (old["claim_token"], old["receipt_id"]) != (value["claim_token"], row["receipt_id"]):
            raise IntegrityError("Pinterest accepted outcome no longer owns its claim")
        db.execute("UPDATE pinterest_tasks SET state=?,reason=?,retry_at=?,updated_at=? WHERE task_id=?",
                   (value["state"], value.get("reason"), value.get("retry_at", 0), utc(), value["task_id"]))
        if value.get("download_scope") and value["state"] != "waiting_retry":
            garbage.enqueue(db, job_id, value["download_scope"], value["download_scope"])
        for item in value["next_tasks"]:
            task(db, job_id, item["kind"], item["pin_id"], item["input"])
    db.execute("INSERT INTO pinterest_applied VALUES(?,?,?)", (row["receipt_id"], job_id, row["seq"]))
