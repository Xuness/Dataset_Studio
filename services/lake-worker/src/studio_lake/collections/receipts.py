"""Durable result outbox: acquire -> archive -> independent control and serving acknowledgements."""

import base64
import json
import uuid

from . import planner, validation
from ..media_lake.library import replay_receipt
from ..media_lake.online import load_records
from ..media_lake.schema import canonical, utc
from ..util import IntegrityError, atomic_json, contained, digest, failpoint, file_hash, read_json, safe_managed_path, stable_id


def staging(service, job, claim):
    path = safe_managed_path(service.state.root, service.state.root / "collection-spool" / job["id"] / claim)
    path.mkdir(parents=True, exist_ok=True)
    owner = dict(job_id=job["id"], claim_token=claim, definition_sha256=job["definition_sha256"])
    marker = path / "owner.json"
    if marker.exists() and read_json(marker) != owner:
        raise IntegrityError("Collection staging owner changed")
    atomic_json(marker, owner)
    return path


def prepared(service, job, task, records, *, files=(), acquired_at=None, download_bytes=0, reused=False, summary=None,
             checkpoints=(), state="done", reason=None, intent=False, directory=None, receipt_id=None):
    detail = json.loads(task["payload_json"]).get("detail") if task else None
    validation.records(records, job["lake_id"], detail=detail)
    receipt_id = receipt_id or str(uuid.uuid4())
    claim = task["claim_token"] if task else str(uuid.uuid4())
    directory = directory or staging(service, job, claim)
    payload = {name: [dict(r) for r in rows] for name, rows in records.items()}
    for captured in payload.get("captures", []):
        captured["raw_base64"] = base64.b64encode(captured.pop("raw_body")).decode("ascii")
    value = dict(version=2, receipt_id=receipt_id, job_id=job["id"], task_id=task["id"] if task else None,
                 execution_epoch=task["claimed_epoch"] if task else job["execution_epoch"], claim_token=claim,
                 definition_sha256=job["definition_sha256"], input_fingerprint=digest(task["payload_json"].encode()) if task else job["definition_sha256"],
                 records=payload, files=list(files), acquired_at=acquired_at or utc(), download_bytes=download_bytes, reused=reused,
                 checkpoints=list(checkpoints), outcome_state=state, reason=reason, intent=intent, summary=summary or {})
    from .batches import validate_envelope

    validate_envelope(value)
    path = directory / "result.json"
    atomic_json(path, value)
    fingerprint = file_hash(path)
    key = (stable_id("collection-intent-v1", job["lake_id"], job["id"], job["definition_sha256"]) if intent
           else stable_id("collection-batch-v1", job["lake_id"], job["id"], [receipt_id]))
    with service.state.db() as db:
        db.execute("INSERT INTO collection_outbox(id,job_id,task_id,execution_epoch,claim_token,dedupe_key,intent_path,content_sha256,state,created_at,updated_at,outcome_state) VALUES(?,?,?,?,?,?,?,?,'prepared',?,?,?)",
                   (receipt_id, job["id"], value["task_id"], value["execution_epoch"], claim, key, str(path.relative_to(service.state.root)), fingerprint, utc(), utc(), state))
        if task:
            changed = db.execute("UPDATE collection_tasks SET state='staged',result_receipt=?,updated_at=? WHERE id=? AND claimed_epoch=? AND claim_token=? AND state='running'",
                                 (str(path.relative_to(service.state.root)), utc(), task["id"], task["claimed_epoch"], claim)).rowcount
            if changed != 1:
                raise IntegrityError("Result no longer belongs to this task claim")
            if files:
                current = service.row(job["id"], db)
                counters = json.loads(current["counters_json"])
                counters.setdefault("staged_media", {})[receipt_id] = "reused" if reused else "downloaded"
                db.execute("UPDATE collection_jobs SET counters_json=? WHERE id=?", (canonical(counters), job["id"]))
    failpoint("collection_after_receipt")
    return receipt_id


def fence(service, receipt):
    job = service.row(receipt["job_id"])
    if job["execution_epoch"] != receipt["execution_epoch"] or job["definition_sha256"] != receipt["definition_sha256"]:
        raise IntegrityError("Stale collection execution receipt")
    if receipt["task_id"]:
        with service.state.db() as db:
            task = db.execute("SELECT * FROM collection_tasks WHERE id=? AND job_id=?", (receipt["task_id"], receipt["job_id"])).fetchone()
        if not task or task["claimed_epoch"] != receipt["execution_epoch"] or task["claim_token"] != receipt["claim_token"] or digest(task["payload_json"].encode()) != receipt["input_fingerprint"]:
            raise IntegrityError("Stale collection task claim")


def accept(service, lib, outbox):
    """Compatibility entry point for accepting exactly one logical result."""
    from .batches import accept_one

    return accept_one(service, lib, outbox)


def quarantine(service, lib, row, error):
    """Preserve original bytes and their expected hash, outside cancellable scratch."""
    with lib.journal() as journal:
        if row["batch_id"] and journal.execute("SELECT 1 FROM commits WHERE batch_id=?", (row["batch_id"],)).fetchone():
            raise IntegrityError("An authoritative archive cannot be quarantined as scratch")
    source = safe_managed_path(service.state.root, contained(service.state.root, row["intent_path"]).parent)
    destination = safe_managed_path(service.state.root, service.state.root / "collection-quarantine" / row["job_id"] / row["id"])
    directory = source if source.exists() else destination
    owner = read_json(directory / "owner.json")
    if owner.get("job_id") != row["job_id"] or owner.get("claim_token") != row["claim_token"]:
        raise IntegrityError("Quarantine owner mismatch")
    evidence = directory / "quarantine.json"
    if evidence.exists():
        report = read_json(evidence)
        if report.get("receipt_id") != row["id"] or report.get("expected_sha256") != row["content_sha256"]:
            raise IntegrityError("Quarantine evidence changed")
    else:
        files = []
        for child in directory.iterdir():
            child = safe_managed_path(service.state.root, child)
            if not child.is_file():
                raise IntegrityError("Unexpected nested quarantine evidence")
            files.append(dict(path=child.name, bytes=child.stat().st_size, sha256=file_hash(child)))
        report = dict(version=1, receipt_id=row["id"], job_id=row["job_id"], task_id=row["task_id"],
                      expected_sha256=row["content_sha256"], observed_sha256=file_hash(directory / "result.json") if (directory / "result.json").exists() else None,
                      detected_at=utc(), reason_code="COLLECTION_RESULT_INVALID", error_type=type(error).__name__, files=files)
        atomic_json(evidence, report)
    if directory == source:
        destination.parent.mkdir(parents=True, exist_ok=True)
        source.rename(destination)
    failpoint("collection_after_quarantine_move")
    relative = str((destination / "quarantine.json").relative_to(service.state.root))
    with service.state.db() as db:
        db.execute("BEGIN IMMEDIATE")
        db.execute("INSERT INTO collection_quarantines VALUES(?,?,?,?,?,?,NULL) ON CONFLICT(receipt_id) DO NOTHING",
                   (row["id"], report["reason_code"], relative, report["expected_sha256"], report["observed_sha256"], report["detected_at"]))
        changed = db.execute("UPDATE collection_outbox SET state='quarantined',updated_at=? WHERE id=? AND state IN ('prepared','quarantined') AND archive_seq IS NULL",
                             (utc(), row["id"])).rowcount
        if changed != 1:
            raise IntegrityError("Quarantine no longer owns an uncommitted result")
        if row["task_id"]:
            db.execute("UPDATE collection_tasks SET state='needs_review',reason=?,summary_json=?,updated_at=? WHERE id=? AND claimed_epoch=? AND claim_token=?",
                       (report["reason_code"], canonical(dict(quarantined_receipt_id=row["id"], evidence_path=relative)), utc(), row["task_id"], row["execution_epoch"], row["claim_token"]))
        job = service.row(row["job_id"], db)
        counters = json.loads(job["counters_json"])
        counters.get("staged_media", {}).pop(row["id"], None)
        counters["quarantined_receipts"] = db.execute("SELECT count(*) FROM collection_outbox WHERE job_id=? AND state='quarantined'", (row["job_id"],)).fetchone()[0]
        db.execute("UPDATE collection_jobs SET counters_json=?,updated_at=? WHERE id=?", (canonical(counters), utc(), row["job_id"]))


def reconcile(service, lib):
    """Archive replay is independent of current epoch and desired pause/cancel state."""
    with service.state.db() as db:
        after = db.execute("SELECT coalesce(max(seq),0) FROM collection_applied_batches WHERE lake_id=?", (lib.info["library_id"],)).fetchone()[0]
    while True:
        with lib.journal() as journal:
            commit = journal.execute("SELECT * FROM commits WHERE seq>? ORDER BY seq LIMIT 1", (after,)).fetchone()
        if commit is None:
            break
        if commit["seq"] != after + 1:
            raise IntegrityError("Collection control replay prefix is discontinuous")
        manifest = json.loads(commit["manifest_json"])
        directory = lib.root / "segments" / commit["batch_id"]
        replay = replay_receipt(directory, manifest)
        records = load_records(directory, manifest)
        hashed = digest(canonical(replay).encode())
        with service.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            previous = db.execute("SELECT receipt_sha256 FROM collection_applied_batches WHERE lake_id=? AND seq=?", (lib.info["library_id"], commit["seq"])).fetchone()
            if previous:
                if previous[0] != hashed:
                    raise IntegrityError("Control replay receipt hash changed")
                after = commit["seq"]
                continue
            job = service.row(replay["job_id"], db)
            if job["definition_sha256"] != replay["definition_sha256"]:
                raise IntegrityError("Control definition differs from archived collection intent")
            counters = json.loads(job["counters_json"])
            for receipt_id in replay["receipt_ids"]:
                counters.get("staged_media", {}).pop(receipt_id, None)
            serving = counters.get("publication", {}).get("served_seq", 0)
            archive_head = max(commit["seq"], serving, counters.get("publication", {}).get("archive_seq", 0))
            counters["publication"] = dict(archive_seq=archive_head, served_seq=serving, pending_batches=max(0, archive_head - serving))
            for outcome in replay["task_outcomes"]:
                receipt_id = outcome.get("receipt_id") or (replay["receipt_ids"][0] if len(replay["receipt_ids"]) == 1 else None)
                outbox = db.execute("SELECT published FROM collection_outbox WHERE id=?", (receipt_id,)).fetchone()
                row = db.execute("SELECT * FROM collection_tasks WHERE id=? AND job_id=?", (outcome["task_id"], job["id"])).fetchone()
                if row is None:
                    raise IntegrityError("Archived task cannot be resolved from its earlier plan")
                summary = outcome.get("summary", {})
                db.execute("UPDATE collection_tasks SET state=?,reason=?,summary_json=?,updated_at=? WHERE id=?", (outcome["state"], outcome["reason"], canonical(summary), utc(), outcome["task_id"]))
                from .incremental import apply_summary

                apply_summary(db, job, row, summary, counters)
                if row["kind"] == "media_download" and outcome["state"] == "done":
                    counters["archived_media"] += 1
                    if outbox and outbox[0]:
                        counters["published_media"] += 1
                    assets = [a for a in records.get("assets", []) if a["acquisition_receipt_id"] == receipt_id]
                    reused = bool(assets) and all(a["evidence"] == "historical_reuse" for a in assets)
                    counters["media_reused" if reused else "media_downloaded"] += 1
                if receipt_id:
                    db.execute("UPDATE collection_outbox SET outcome_state=? WHERE id=?", (outcome["state"], receipt_id))
                    if outcome["state"] == "done":
                        db.execute("UPDATE collection_quarantines SET replacement_receipt_id=? WHERE replacement_receipt_id IS NULL AND receipt_id IN (SELECT id FROM collection_outbox WHERE job_id=? AND task_id=? AND state='quarantined')",
                                   (receipt_id, job["id"], outcome["task_id"]))
            counters["objects"] += len(records.get("objects", []))
            counters["browsable_images"] += sum(o["media_category"] == "image" for o in records.get("objects", []))
            for point in replay["checkpoint_advances"]:
                old = db.execute("SELECT revision FROM collection_checkpoints WHERE job_id=? AND stream_key=?", (job["id"], point["stream_key"])).fetchone()
                if (old[0] if old else 0) != point["expected_revision"]:
                    raise IntegrityError("Control checkpoint differs from authoritative archive")
                db.execute("INSERT INTO collection_checkpoints VALUES(?,?,?,?,?,?) ON CONFLICT(job_id,stream_key) DO UPDATE SET revision=excluded.revision,cursor_json=excluded.cursor_json,archive_seq=excluded.archive_seq,batch_id=excluded.batch_id",
                           (job["id"], point["stream_key"], point["next_revision"], canonical(dict(cursor=point["next_cursor"], exhausted=point["exhausted"], capture_id=point["capture_id"])), commit["seq"], commit["batch_id"]))
            planner.apply(db, job, records, replay["task_outcomes"])
            from .incremental import apply_directory_reuse

            apply_directory_reuse(db, job, records, replay["task_outcomes"], counters)
            db.execute("UPDATE collection_jobs SET counters_json=?,revision=revision+1,updated_at=? WHERE id=?", (canonical(counters), utc(), job["id"]))
            db.execute("INSERT INTO collection_applied_batches VALUES(?,?,?,?,?,?)", (lib.info["library_id"], commit["seq"], commit["batch_id"], job["id"], hashed, utc()))
            for receipt_id in replay["receipt_ids"]:
                db.execute("UPDATE collection_outbox SET control_applied=1,archive_seq=?,batch_id=?,state='archive_committed',updated_at=? WHERE id=? AND job_id=?",
                           (commit["seq"], commit["batch_id"], utc(), receipt_id, job["id"]))
            db.execute("UPDATE collection_batches SET state='archive_committed',archive_seq=?,updated_at=? WHERE id=?", (commit["seq"], utc(), commit["batch_id"]))
        after = commit["seq"]
        failpoint("collection_after_control")


def restore_pending_outcomes(service, lib, job_id, served_seq):
    """v10 control rows lack per-receipt outcomes; recover them from archive truth."""
    while True:
        with service.state.db() as db:
            rows = [dict(row) for row in db.execute("""SELECT id,task_id,archive_seq,batch_id FROM collection_outbox
                WHERE job_id=? AND state IN ('archive_committed','needs_review') AND control_applied=1 AND published=0
                AND task_id IS NOT NULL AND outcome_state IS NULL AND archive_seq<=?
                ORDER BY archive_seq,id LIMIT 256""", (job_id, served_seq))]
        if not rows:
            return
        outcomes, updates = {}, []
        for row in rows:
            key = (row["archive_seq"], row["batch_id"])
            if key not in outcomes:
                with lib.journal() as journal:
                    commit = journal.execute("SELECT batch_id,manifest_json FROM commits WHERE seq=?", (key[0],)).fetchone()
                if not commit or commit["batch_id"] != key[1]:
                    raise IntegrityError("Pending publication cannot resolve its archive batch")
                manifest = json.loads(commit["manifest_json"])
                directory = lib.root / "segments" / key[1]
                path, expected = directory / "collection_replay.json", manifest["files"]["collection_replay.json"]
                if path.stat().st_size != expected["bytes"] or file_hash(path) != expected["sha256"]:
                    raise IntegrityError("Pending publication archive replay hash changed")
                replay = replay_receipt(directory, manifest)
                if replay["job_id"] != job_id:
                    raise IntegrityError("Pending publication archive belongs to another job")
                outcomes[key] = {
                    outcome.get("receipt_id") or (replay["receipt_ids"][0] if len(replay["receipt_ids"]) == 1 else None): outcome
                    for outcome in replay["task_outcomes"]
                }
            outcome = outcomes[key].get(row["id"])
            if not outcome or outcome["task_id"] != row["task_id"]:
                raise IntegrityError("Pending publication cannot resolve its archived outcome")
            updates.append((outcome["state"], utc(), row["id"], job_id, row["task_id"], *key))
        # Archive reads happen outside the short control write transaction.
        with service.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            db.executemany("""UPDATE collection_outbox SET outcome_state=?,updated_at=? WHERE id=? AND job_id=?
                AND task_id=? AND archive_seq=? AND batch_id=? AND outcome_state IS NULL AND control_applied=1 AND published=0""", updates)


def publish_ack(service, lib, job_id, *, stop=None):
    result = lib.sync_online(stop=stop)
    restore_pending_outcomes(service, lib, job_id, result["served_seq"])
    with service.state.db() as db:
        db.execute("BEGIN IMMEDIATE")
        job = service.row(job_id, db)
        counters = json.loads(job["counters_json"])
        delta = db.execute("SELECT count(*) FROM collection_outbox o JOIN collection_tasks t ON t.id=o.task_id WHERE o.job_id=? AND o.published=0 AND o.control_applied=1 AND o.archive_seq<=? AND t.kind='media_download' AND o.outcome_state='done'", (job_id, result["served_seq"])).fetchone()[0]
        counters["published_media"] += delta
        counters["publication"] = dict(archive_seq=result["archive_seq"], served_seq=result["served_seq"], pending_batches=result["archive_seq"] - result["served_seq"])
        db.execute("UPDATE collection_outbox SET published=1,updated_at=? WHERE job_id=? AND archive_seq<=?", (utc(), job_id, result["served_seq"]))
        db.execute("UPDATE collection_jobs SET counters_json=?,updated_at=? WHERE id=?", (canonical(counters), utc(), job_id))
    failpoint("collection_after_published")
    return result


def release(service, lib, job_id, *, resources=None):
    """Only acknowledged, task-owned scratch files can be removed; archive bytes stay immutable."""
    with service.state.db() as db:
        rows = [dict(r) for r in db.execute("""SELECT o.* FROM collection_outbox o WHERE o.job_id=? AND o.control_applied=1 AND o.published=1
            AND o.state IN ('archive_committed','needs_review')
            ORDER BY o.archive_seq LIMIT 256""", (job_id,))]
    for row in rows:
        path = contained(service.state.root, row["intent_path"])
        directory = safe_managed_path(service.state.root, path.parent)
        if row["task_id"]:
            with service.state.db() as db:
                task = db.execute("SELECT state FROM collection_tasks WHERE id=?", (row["task_id"],)).fetchone()
            if task and task[0] in {"done", "needs_review", "unavailable", "cancelled"}:
                from .tasks import retire_download

                retire_download(service, job_id, row["task_id"], resources)
        if directory.exists():
            owner = read_json(directory / "owner.json")
            if owner["job_id"] != job_id or owner["claim_token"] != row["claim_token"]:
                raise IntegrityError("Scratch cleanup owner mismatch")
            # Each claim is private and all worker consumers have returned before this call.
            for child in directory.iterdir():
                checked = safe_managed_path(service.state.root, child)
                if not checked.is_file():
                    raise IntegrityError("Unexpected nested collection scratch directory")
                checked.unlink()
            directory.rmdir()
        pack = contained(lib.cache, "pack_staging/" + row["batch_id"] + ".tar.tmp")
        pack.unlink(missing_ok=True)
        if resources is not None:
            resources.retire(directory)
        with service.state.db() as db:
            db.execute("UPDATE collection_outbox SET state='released',updated_at=? WHERE id=?", (utc(), row["id"]))


def prune_finished(service, job_id, resources):
    """Remove superseded claims after all execution consumers exited, preserving diagnostic outboxes."""
    job = service.row(job_id)
    root = safe_managed_path(service.state.root, service.state.root / "collection-spool" / job_id)
    if not root.exists():
        return
    with service.state.db() as db:
        retained = {str(contained(service.state.root, r[0]).parent) for r in db.execute("SELECT intent_path FROM collection_outbox WHERE job_id=? AND state<>'released'", (job_id,))}
    for directory in root.iterdir():
        directory = safe_managed_path(service.state.root, directory)
        if str(directory) in retained:
            continue
        owner = read_json(directory / "owner.json")
        if owner.get("job_id") != job_id or owner.get("definition_sha256") != job["definition_sha256"]:
            raise IntegrityError("Superseded claim cleanup owner mismatch")
        for path in directory.iterdir():
            path = safe_managed_path(service.state.root, path)
            if not path.is_file():
                raise IntegrityError("Unexpected nested claim scratch")
            path.unlink()
        directory.rmdir()
        resources.retire(directory)
    if not any(root.iterdir()):
        root.rmdir()
