"""Durable outcomes and idempotent control replay after authoritative journal acceptance."""

import base64
import json

from . import model
from .http import Response
from ..canonical import canonical, utc
from ..util import IntegrityError, atomic_json, read_json, stable_id


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


def save_prepared(root, replay, records, media=None):
    records = {k: [dict(r) for r in rows] for k, rows in records.items()}
    for capture in records.get("captures", []):
        capture["raw_body"] = base64.b64encode(capture["raw_body"]).decode("ascii")
    atomic_json(root / (replay["receipt_id"] + ".prepared.json"), dict(replay=replay, records=records, media=media))


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
        rows = [dict(r) for r in journal.execute("SELECT * FROM pinterest_receipts WHERE job_id=? AND seq>? ORDER BY seq LIMIT 128", (job_id, after))]
    for row in rows:
        value = json.loads(row["replay_json"])
        with state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            if db.execute("SELECT 1 FROM pinterest_applied WHERE receipt_id=?", (row["receipt_id"],)).fetchone():
                continue
            for stream in value.get("streams", []):
                db.execute("""INSERT INTO pinterest_streams(scan_id,job_id,entrypoint,subject_id,root_json,
                    parameters_json,depth,state,updated_at) VALUES(?,?,?,?,?,?,?,'active',?)
                    ON CONFLICT(scan_id) DO NOTHING""", (stream["scan_id"], job_id, stream["entrypoint"], stream["subject_id"],
                    canonical(stream["root"]), canonical(stream["parameters"]), stream["depth"], utc()))
            for admission in value.get("admissions", []):
                db.execute("INSERT INTO pinterest_admitted VALUES(?,?,?,?) ON CONFLICT DO NOTHING",
                    (job_id, admission["kind"], admission["source_id"], row["receipt_id"]))
            for name, count in value.get("metrics", {}).items():
                db.execute("INSERT INTO pinterest_metrics VALUES(?,?,?) ON CONFLICT(job_id,name) DO UPDATE SET value=value+excluded.value",
                           (job_id, name, count))
            if "checkpoint" in value:
                point = value["checkpoint"]
                db.execute("INSERT INTO pinterest_stream_pages VALUES(?,?,?) ON CONFLICT DO NOTHING",
                           (point["scan_id"], point["page_key"], row["receipt_id"]))
                db.execute("UPDATE pinterest_streams SET state=?,cursor_json=?,reason=?,pages=pages+1,members=members+?,last_turn=?,updated_at=? WHERE scan_id=? AND job_id=?",
                    (point["state"], canonical(point["cursor"]), point["reason"], point["members"], row["seq"], utc(), point["scan_id"], job_id))
            if "sample" in value:
                sample = value["sample"]
                db.execute("UPDATE pinterest_streams SET samples_checked=samples_checked+1,mismatches=mismatches+?,force_detail=max(force_detail,?) WHERE scan_id=? AND job_id=?",
                           (int(sample["differs"]), int(sample["differs"]), sample["scan_id"], job_id))
            if value["kind"] == "intent":
                for item in value["next_tasks"]:
                    task(db, job_id, item["kind"], item["pin_id"], item["input"])
            else:
                old = db.execute("SELECT claim_token,receipt_id,state FROM pinterest_tasks WHERE task_id=? AND job_id=?", (value["task_id"], job_id)).fetchone()
                if not old or (old["claim_token"], old["receipt_id"]) != (value["claim_token"], row["receipt_id"]):
                    raise IntegrityError("Pinterest accepted outcome no longer owns its claim")
                db.execute("UPDATE pinterest_tasks SET state=?,reason=?,retry_at=?,updated_at=? WHERE task_id=?",
                           (value["state"], value.get("reason"), value.get("retry_at", 0), utc(), value["task_id"]))
                for item in value["next_tasks"]:
                    task(db, job_id, item["kind"], item["pin_id"], item["input"])
            db.execute("INSERT INTO pinterest_applied VALUES(?,?,?)", (row["receipt_id"], job_id, row["seq"]))
            db.execute("UPDATE pinterest_jobs SET archive_seq=max(archive_seq,?),updated_at=? WHERE id=?", (row["seq"], utc(), job_id))
    return len(rows)
