"""Bounded physical packing of independently identified, immutable receipts."""

import base64
from dataclasses import dataclass
from datetime import datetime, timezone
import json
import re
import uuid

from . import receipts, validation
from .failures import InvalidReceipt
from ..media_lake.library import MediaBatch
from ..media_lake.schema import MAX_BATCH_METADATA_BYTES, InvalidCanonicalResult, canonical, utc
from ..util import IntegrityError, contained, digest, failpoint, file_hash, read_json, stable_id

MAX_RECEIPTS = 64
MAX_ROWS = 90_000
MAX_METADATA = 48 * 1024**2
MAX_MEDIA = 128 * 1024**2
MAX_WAIT_SECONDS = 1


@dataclass(frozen=True)
class PreparedReceipt:
    outbox: dict
    value: dict
    row_count: int
    metadata_bytes: int
    media_bytes: int


def read(service, lib, row):
    path = contained(service.state.root, row["intent_path"])
    if not path.exists():
        evidence = service.state.root / "collection-quarantine" / row["job_id"] / row["id"] / "quarantine.json"
        if evidence.is_file() or (path.parent / "owner.json").is_file():
            raise InvalidReceipt("Prepared receipt is absent or quarantine was interrupted")
        raise IntegrityError("Prepared receipt is missing")
    if path.stat().st_size > MAX_BATCH_METADATA_BYTES * 2 or file_hash(path) != row["content_sha256"]:
        raise InvalidReceipt("Prepared receipt bytes changed")
    try:
        value = read_json(path)
        for key, expected in (("receipt_id", row["id"]), ("job_id", row["job_id"]), ("task_id", row["task_id"]),
                              ("execution_epoch", row["execution_epoch"]), ("claim_token", row["claim_token"])):
            if value[key] != expected:
                raise InvalidReceipt("Prepared receipt identity mismatch")
        if value["version"] not in {1, 2} or value["outcome_state"] not in {"done", "unavailable", "excluded", "needs_review"}:
            raise InvalidReceipt("Unsupported result contract")
        validate_envelope(value)
        for capture in value["records"].get("captures", []):
            capture["raw_body"] = base64.b64decode(capture.pop("raw_base64"), validate=True)
        with service.state.db() as db:
            task = db.execute("SELECT payload_json FROM collection_tasks WHERE id=?", (value["task_id"],)).fetchone()
        detail = json.loads(task[0]).get("detail") if task else None
        count, size = validation.records(value["records"], lib.info["library_id"], detail=detail)
        media_bytes = 0
        for file in value["files"]:
            stored = contained(path.parent, file["path"])
            if stored.stat().st_size != file["bytes"] or file_hash(stored) != file["sha256"]:
                raise InvalidReceipt("Prepared media bytes changed")
            media_bytes += file["bytes"]
        size += len(canonical(value.get("summary", {})).encode())
        return PreparedReceipt(dict(row), value, count + len(value["files"]) * 2, size, media_bytes)
    except (ValueError, TypeError, KeyError, AttributeError, OverflowError, FileNotFoundError) as error:
        raise InvalidReceipt("Prepared result shape is invalid") from error


def validate_envelope(value):
    """Reject permanent envelope errors before assigning a physical batch."""
    if not isinstance(value["intent"], bool) or value["intent"] != (value["task_id"] is None):
        raise InvalidReceipt("Invalid intent result")
    if value["version"] == 1 and str(uuid.UUID(value["batch_id"])) != value["batch_id"]:
        raise InvalidReceipt("Invalid legacy batch identity")
    if not isinstance(value["summary"], dict) or not isinstance(value["records"], dict) or not isinstance(value["files"], list):
        raise InvalidReceipt("Invalid result containers")
    if utc(value["acquired_at"]) != value["acquired_at"] or not isinstance(value["reused"], bool):
        raise InvalidReceipt("Invalid acquisition evidence")
    if value["intent"] and (value["files"] or value["records"] or value["checkpoints"]):
        raise InvalidReceipt("Intent cannot contain acquisition facts")
    if len(value["files"]) > 8 or not isinstance(value["checkpoints"], list):
        raise InvalidReceipt("Result exceeds representation or checkpoint contract")
    streams = set()
    for point in value["checkpoints"]:
        if (not isinstance(point["stream_key"], str) or point["stream_key"] in streams
                or isinstance(point["expected_revision"], bool) or not isinstance(point["expected_revision"], int)
                or point["expected_revision"] < 0 or point["next_revision"] != point["expected_revision"] + 1
                or not isinstance(point["next_cursor"], dict) or not isinstance(point["exhausted"], bool)):
            raise InvalidReceipt("Invalid checkpoint shape")
        streams.add(point["stream_key"])
    for file in value["files"]:
        if (not isinstance(file["path"], str) or not re.fullmatch(r"[A-Za-z0-9_.-]{1,255}", file["path"])
                or file["path"] in {".", ".."} or not isinstance(file["bytes"], int) or isinstance(file["bytes"], bool) or file["bytes"] < 0
                or not isinstance(file["sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", file["sha256"])
                or file["representation"] not in {"original", "derived", "poster"}
                or file["evidence"] not in {"downloaded", "http_validated", "historical_reuse", "derived"}):
            raise InvalidReceipt("Invalid media result descriptor")
        # These required fields are consumed by the writer even for legacy results.
        for field in ("stored_ext", "content_type", "media_category", "recipe_id", "details_json"):
            if not isinstance(file[field], str):
                raise InvalidReceipt("Invalid media descriptor field")
        for field in ("stored_width", "stored_height", "source_bytes"):
            if file[field] is not None and (isinstance(file[field], bool) or not isinstance(file[field], int) or file[field] < 0):
                raise InvalidReceipt("Invalid media dimensions or length")
        if file["last_verified_at"] is not None and utc(file["last_verified_at"]) != file["last_verified_at"]:
            raise InvalidReceipt("Invalid media verification time")
        if file["source_sha256"] is not None and not re.fullmatch(r"[0-9a-f]{64}", file["source_sha256"]):
            raise InvalidReceipt("Invalid source hash")
        canonical(json.loads(file["details_json"]))


def freeze(service, lib, prepared):
    first = prepared[0].value
    ids = [r.outbox["id"] for r in prepared]
    batch_id = first["batch_id"] if first["version"] == 1 else str(uuid.uuid4())
    key = (stable_id("collection-intent-v1", lib.info["library_id"], first["job_id"], first["definition_sha256"]) if first["intent"]
           else stable_id("collection-batch-v1", lib.info["library_id"], first["job_id"], sorted(ids)))
    at = utc()
    with service.state.db() as db:
        db.execute("BEGIN IMMEDIATE")
        db.execute("INSERT INTO collection_batches VALUES(?,?,?,?,'prepared',NULL,?,?)",
                   (batch_id, first["job_id"], canonical(ids), key, at, at))
        for receipt in prepared:
            changed = db.execute("UPDATE collection_outbox SET batch_id=?,updated_at=? WHERE id=? AND state='prepared' AND batch_id IS NULL",
                                 (batch_id, at, receipt.outbox["id"])).rowcount
            if changed != 1:
                raise IntegrityError("Receipt was assigned to another physical batch")
    failpoint("collection_after_batch_plan")
    return dict(id=batch_id, job_id=first["job_id"], receipt_ids_json=canonical(ids), dedupe_key=key)


def members(service, plan):
    ids = json.loads(plan["receipt_ids_json"])
    with service.state.db() as db:
        rows = {r["id"]: dict(r) for r in db.execute("SELECT * FROM collection_outbox WHERE batch_id=?", (plan["id"],))}
    if set(rows) != set(ids):
        raise IntegrityError("Frozen batch membership changed")
    return [rows[identity] for identity in ids]


def accept_plan(service, lib, plan, prepared=None):
    batch_id = plan["id"]
    rows = members(service, plan)
    with lib.writer_lock():
        committed = lib.committed_key(plan["dedupe_key"])
        if committed:
            if committed["batch_id"] != batch_id:
                raise IntegrityError("Physical batch deduplication collision")
            seq = committed["seq"]
        else:
            prepared = prepared or [read(service, lib, row) for row in rows]
            def fence(_):
                for result in prepared:
                    receipts.fence(service, result.value)

            final, staging = lib.root / "segments" / batch_id, lib.root / "staging" / batch_id
            if (final / "manifest.json").exists() or (staging / "manifest.json").exists():
                directory = final if (final / "manifest.json").exists() else staging
                seq = lib.accept_manifest(directory, read_json(directory / "manifest.json"), fence=fence)
            else:
                fence(None)
                job = service.row(plan["job_id"])
                with lib.journal() as journal:
                    intent = journal.execute("SELECT intent_batch_id FROM collection_runs WHERE job_id=?", (job["id"],)).fetchone()
                batch = MediaBatch(lib, job["id"], job["definition_sha256"], [r.outbox["id"] for r in prepared],
                                   intent_batch_id=intent[0] if intent else None,
                                   definition=json.loads(job["definition_json"]) if prepared[0].value["intent"] else None,
                                   batch_id=batch_id, resume=True)
                try:
                    for result in prepared:
                        add_receipt(service, batch, result)
                    seq = batch.commit(fence=fence)
                finally:
                    if batch.tar:
                        batch.tar.close()
                    if batch.pack_writer:
                        batch.pack_writer.close()
    with service.state.db() as db:
        db.execute("BEGIN IMMEDIATE")
        db.execute("UPDATE collection_batches SET state='archive_committed',archive_seq=?,updated_at=? WHERE id=?", (seq, utc(), batch_id))
        db.execute("UPDATE collection_outbox SET state='archive_committed',archive_seq=?,updated_at=? WHERE batch_id=?", (seq, utc(), batch_id))
    receipts.failpoint("collection_after_outbox_archive")
    return seq


def add_receipt(service, batch, result):
    value, records = result.value, result.value["records"]
    for name, rows in records.items():
        batch.add(name, rows)
    origin, assets = None, []
    if value["files"]:
        with service.state.db() as db:
            task = db.execute("SELECT payload_json FROM collection_tasks WHERE id=?", (value["task_id"],)).fetchone()
        media_id = json.loads(task[0])["entry"]["media_id"]
    path = contained(service.state.root, result.outbox["intent_path"])
    for file in sorted(value["files"], key=lambda item: item["representation"] != "original"):
        stored = contained(path.parent, file["path"])
        data = stored.read_bytes()
        if len(data) != file["bytes"] or digest(data) != file["sha256"]:
            raise InvalidReceipt("Prepared media changed during packing")
        sha = batch.add_blob(data, file["stored_ext"], content_type=file["content_type"], media_category=file["media_category"],
                             width=file["stored_width"], height=file["stored_height"])
        asset_id = stable_id("asset-record-v2", media_id, file["representation"], file["recipe_id"], value["receipt_id"], sha)
        assets.append(dict(asset_id=asset_id, media_id=media_id, sha256=sha, representation=file["representation"], recipe_id=file["recipe_id"],
                           acquisition_receipt_id=value["receipt_id"], source_sha256=file["source_sha256"], source_bytes=file["source_bytes"],
                           acquired_at=value["acquired_at"], last_verified_at=file["last_verified_at"], evidence=file["evidence"],
                           derived_from_asset_id=origin if file["representation"] != "original" else None, details_json=file["details_json"]))
        if file["representation"] == "original":
            origin = asset_id
    if assets:
        batch.add("assets", assets)
    if value["task_id"]:
        record_ids = [r[field] for name, field in (("author_observations", "observation_id"), ("work_observations", "observation_id"),
                                                   ("media_manifests", "manifest_id")) for r in records.get(name, [])] + [a["asset_id"] for a in assets]
        batch.replay["task_outcomes"].append(dict(receipt_id=value["receipt_id"], task_id=value["task_id"], execution_epoch=value["execution_epoch"],
                                                 claim_token=value["claim_token"], state=value["outcome_state"], reason=value["reason"],
                                                 record_ids=record_ids, summary=value.get("summary", {})))
    batch.replay["checkpoint_advances"].extend(value["checkpoints"])
    batch.replay["discovery_snapshot_ids"].extend(r["snapshot_id"] for r in records.get("discovery_snapshots", []))


def accept_one(service, lib, outbox):
    if outbox["batch_id"]:
        with service.state.db() as db:
            plan = db.execute("SELECT * FROM collection_batches WHERE id=?", (outbox["batch_id"],)).fetchone()
        if plan:
            return accept_plan(service, lib, dict(plan))
    result = read(service, lib, outbox)
    receipts.fence(service, result.value)
    return accept_plan(service, lib, freeze(service, lib, [result]), [result])


def flush(service, lib, job_id, *, force=True):
    """Flush due groups; callers force a drain before epoch changes, idle, or exit."""
    changed = False
    while True:
        with service.state.db() as db:
            plan = db.execute("SELECT * FROM collection_batches WHERE job_id=? AND state='prepared' ORDER BY created_at,id LIMIT 1", (job_id,)).fetchone()
            rows = [dict(r) for r in db.execute("SELECT * FROM collection_outbox WHERE job_id=? AND state='prepared' AND batch_id IS NULL ORDER BY created_at,id LIMIT ?",
                                              (job_id, MAX_RECEIPTS + 1))] if plan is None else []
        selected, full = [], False
        if plan is None:
            if rows and not force and len(rows) < MAX_RECEIPTS:
                age = (datetime.now(timezone.utc) - datetime.fromisoformat(rows[0]["created_at"].replace("Z", "+00:00"))).total_seconds()
                if age < MAX_WAIT_SECONDS:
                    return changed
            totals, streams = [0, 0, 0], set()
            for row in rows:
                try:
                    result = read(service, lib, row)
                    receipts.fence(service, result.value)
                except InvalidReceipt as error:
                    receipts.quarantine(service, lib, row, error)
                    changed = True
                    continue
                points = {p["stream_key"] for p in result.value["checkpoints"]}
                sizes = [result.row_count, result.metadata_bytes, result.media_bytes]
                isolated = result.value["intent"] or result.value["version"] == 1
                if selected and (isolated or streams & points or len(selected) >= MAX_RECEIPTS or
                                 any(a + b > cap for a, b, cap in zip(totals, sizes, (MAX_ROWS, MAX_METADATA, MAX_MEDIA)))):
                    full = True
                    break
                selected.append(result)
                totals = [a + b for a, b in zip(totals, sizes)]
                streams.update(points)
                if isolated:
                    full = True
                    break
            if not selected:
                return changed
            age = (datetime.now(timezone.utc) - datetime.fromisoformat(selected[0].outbox["created_at"].replace("Z", "+00:00"))).total_seconds()
            if not force and not full and len(selected) < MAX_RECEIPTS and age < MAX_WAIT_SECONDS:
                return changed
            plan = freeze(service, lib, selected)
        try:
            accept_plan(service, lib, dict(plan), selected or None)
        except (InvalidReceipt, InvalidCanonicalResult) as error:
            # The whole uncommitted plan stays inspectable; no member is falsely
            # acknowledged. Independent groups can still finish or cancel.
            for row in members(service, plan):
                receipts.quarantine(service, lib, row, error)
            quarantine_candidate(lib, plan)
            with service.state.db() as db:
                db.execute("UPDATE collection_batches SET state='quarantined',updated_at=? WHERE id=?", (utc(), plan["id"]))
        changed = True


def quarantine_candidate(lib, plan):
    """Keep a rejected physical candidate out of future archive recovery scans."""
    from ..util import safe_managed_path

    with lib.writer_lock():
        if lib.committed_key(plan["dedupe_key"]):
            raise IntegrityError("An accepted physical batch cannot be quarantined")
        for parent in ("staging", "segments"):
            source = safe_managed_path(lib.root, lib.root / parent / plan["id"])
            if not source.exists():
                continue
            owner = read_json(source / "intent.json")
            if owner.get("dedupe_key") != plan["dedupe_key"] or owner.get("batch_id") != plan["id"] or owner.get("library_id") != lib.info["library_id"]:
                raise IntegrityError("Rejected batch owner mismatch")
            target = safe_managed_path(lib.root, lib.root / "quarantine" / plan["id"])
            target.parent.mkdir(exist_ok=True)
            source.rename(target)
        scratch = safe_managed_path(lib.cache, lib.cache / "pack_staging" / (plan["id"] + ".tar.tmp"))
        scratch.unlink(missing_ok=True)
