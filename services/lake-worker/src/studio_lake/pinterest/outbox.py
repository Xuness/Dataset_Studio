"""Bounded immutable logical results and frozen membership of physical batches."""

import json
import time
import uuid

import pyarrow as pa

from . import receipts
from .lake import schema
from ..canonical import canonical, utc
from ..util import IntegrityError, digest, failpoint, file_hash, stable_id

MAX_RECEIPTS = 64
MAX_ROWS = 8_000
MAX_METADATA = 24 * 1024**2
MAX_MEDIA = 128 * 1024**2
MAX_AGE = 120


def pending(db, job_id):
    return [dict(r) for r in db.execute("SELECT * FROM pinterest_outbox WHERE job_id=? AND state='prepared' ORDER BY created_at,receipt_id LIMIT 65", (job_id,))]


def register(state, lib, job, task, value):
    identity = task["receipt_id"]
    path = receipts.directory(lib, job["id"]) / (identity + ".prepared.json")
    version = value.get("version", 1)
    input_sha = digest(task["input_json"].encode())
    if version == 2 and (value.get("job_id") != job["id"] or value.get("definition_sha256") != job["definition_sha256"]
                         or value.get("input_sha256") != input_sha):
        raise IntegrityError("Pinterest prepared execution inputs changed")
    replay, records = value["replay"], value["records"]
    if (replay["receipt_id"], replay["task_id"], replay["claim_token"]) != (identity, task["task_id"], task["claim_token"]):
        raise IntegrityError("Pinterest prepared claim changed")
    schema.check_rows(records)
    rows = sum(map(len, records.values())) + int(value["media"] is not None)
    metadata = sum(pa.Table.from_pylist(v, schema=schema.arrow_schema(k)).nbytes for k, v in records.items()) + len(canonical(replay).encode())
    if rows > schema.MAX_ROWS or metadata > schema.MAX_METADATA:
        raise IntegrityError("Pinterest logical result exceeds the archive hard limit")
    media_bytes = max((r["download_bytes"] for r in records.get("acquisitions", [])), default=0) if value["media"] else 0
    with state.db() as db:
        db.execute("BEGIN IMMEDIATE")
        old = db.execute("SELECT content_sha256 FROM pinterest_outbox WHERE receipt_id=?", (identity,)).fetchone()
        content_sha = file_hash(path)
        if old:
            if old[0] != content_sha:
                raise IntegrityError("Pinterest durable logical result changed")
            return
        if len(pending(db, job["id"])) >= MAX_RECEIPTS:
            raise IntegrityError("Pinterest logical result buffer exceeded its bound")
        changed = db.execute("UPDATE pinterest_tasks SET state='prepared',updated_at=? WHERE task_id=? AND job_id=? AND state='running' AND claim_token=? AND receipt_id=?",
            (utc(), task["task_id"], job["id"], task["claim_token"], identity)).rowcount
        if not changed:
            raise IntegrityError("Pinterest logical result no longer owns its claim")
        db.execute("""INSERT INTO pinterest_outbox(receipt_id,job_id,task_id,claim_token,version,definition_sha256,input_sha256,
            content_sha256,replay_json,row_count,metadata_bytes,media_bytes,media_records,download_key,context_key,checkpoint_key,state,created_at)
            VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?, 'prepared',?)""",
            (identity, job["id"], task["task_id"], task["claim_token"], version, job["definition_sha256"], input_sha, content_sha,
             canonical(replay), rows, metadata, media_bytes, len(records.get("assets", [])), replay.get("download_key"),
             digest(job["context_json"].encode()), replay.get("checkpoint", {}).get("scan_id"), time.time()))


def load(lib, row):
    root = receipts.directory(lib, row["job_id"])
    path = root / (row["receipt_id"] + ".prepared.json")
    if not path.is_file() or file_hash(path) != row["content_sha256"]:
        raise IntegrityError("Pinterest durable logical result missing or changed")
    value = receipts.load_prepared(root, row["receipt_id"])
    if canonical(value["replay"]) != row["replay_json"]:
        raise IntegrityError("Pinterest durable replay differs from its prepared result")
    return value


def ready(rows):
    return bool(rows) and (len(rows) >= MAX_RECEIPTS or any(r["batch_id"] or r["version"] == 1 for r in rows)
        or sum(r["row_count"] for r in rows) >= MAX_ROWS or sum(r["metadata_bytes"] for r in rows) >= MAX_METADATA
        or sum(r["media_bytes"] for r in rows) >= MAX_MEDIA or time.time() - rows[0]["created_at"] >= MAX_AGE)


def freeze(state, lib, job):
    with state.db() as db:
        db.execute("BEGIN IMMEDIATE")
        existing = db.execute("SELECT * FROM pinterest_batches WHERE job_id=? AND state='prepared' ORDER BY created_at,batch_id LIMIT 1", (job["id"],)).fetchone()
        if existing:
            return dict(existing)
        candidates = pending(db, job["id"])
        if not candidates:
            return None
        members, checkpoints = [], set()
        totals = dict(row_count=0, metadata_bytes=0, media_bytes=0)
        limits = dict(row_count=MAX_ROWS, metadata_bytes=MAX_METADATA, media_bytes=MAX_MEDIA)
        for row in candidates:
            if row["definition_sha256"] != job["definition_sha256"]:
                raise IntegrityError("Pinterest logical result definition differs from its job")
            incompatible = (row["version"] != candidates[0]["version"] or row["context_key"] != candidates[0]["context_key"]
                            or row["checkpoint_key"] in checkpoints or row["definition_sha256"] != job["definition_sha256"])
            if members and (incompatible or candidates[0]["version"] == 1 or any(totals[k] + row[k] > limits[k] for k in limits)):
                break
            members.append(row["receipt_id"])
            if row["checkpoint_key"]:
                checkpoints.add(row["checkpoint_key"])
            for k in totals:
                totals[k] += row[k]
            if len(members) == MAX_RECEIPTS:
                break
        batch_id = members[0] if candidates[0]["version"] == 1 else str(uuid.uuid4())
        key = stable_id("pinterest-batch-v1", lib.info["library_id"], job["id"], members[0]) if candidates[0]["version"] == 1 else stable_id("pinterest-bundle-v1", lib.info["library_id"], job["id"], sorted(members))
        plan = dict(batch_id=batch_id, job_id=job["id"], receipt_ids_json=canonical(members), dedupe_key=key, state="prepared", archive_seq=None, created_at=time.time())
        db.execute("INSERT INTO pinterest_batches(batch_id,job_id,receipt_ids_json,dedupe_key,state,archive_seq,created_at) VALUES(?,?,?,?,?,?,?)", tuple(plan.values()))
        db.executemany("UPDATE pinterest_outbox SET batch_id=? WHERE receipt_id=? AND batch_id IS NULL", [(batch_id, member) for member in members])
    failpoint("pinterest_after_freeze")
    return plan


def members(state, plan):
    with state.db() as db:
        rows = {r["receipt_id"]: dict(r) for r in db.execute("SELECT * FROM pinterest_outbox WHERE batch_id=? LIMIT 65", (plan["batch_id"],))}
    identities = json.loads(plan["receipt_ids_json"])
    if len(rows) != len(identities) or set(rows) != set(identities) or not 1 <= len(rows) <= MAX_RECEIPTS:
        raise IntegrityError("Pinterest frozen batch membership changed")
    return [rows[i] for i in identities]


def merge(values):
    records = {}
    for value in values:
        for name, rows in value["records"].items():
            table = records.setdefault(name, {})
            for row in rows:
                key = tuple(row[k] for k in schema.primary_key(name))
                if key in table and table[key] != row:
                    raise IntegrityError("Conflicting Pinterest facts in one physical batch: " + name)
                table[key] = row
    result = {name: list(rows.values()) for name, rows in records.items()}
    schema.check_rows(result)
    return result
