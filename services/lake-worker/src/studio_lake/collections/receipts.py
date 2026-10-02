"""Durable result outbox: acquire -> archive -> independent control and serving acknowledgements."""

import base64
import json
import uuid

from . import planner
from ..media_lake.library import MediaBatch, replay_receipt
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


def prepared(service, job, task, records, *, files=(), acquired_at=None, download_bytes=0, reused=False,
             checkpoints=(), state="done", reason=None, intent=False, directory=None, receipt_id=None):
    receipt_id = receipt_id or str(uuid.uuid4())
    claim = task["claim_token"] if task else str(uuid.uuid4())
    directory = directory or staging(service, job, claim)
    payload = {name: [dict(r) for r in rows] for name, rows in records.items()}
    for captured in payload.get("captures", []):
        captured["raw_base64"] = base64.b64encode(captured.pop("raw_body")).decode("ascii")
    value = dict(version=1, receipt_id=receipt_id, batch_id=str(uuid.uuid4()), job_id=job["id"], task_id=task["id"] if task else None,
                 execution_epoch=task["claimed_epoch"] if task else job["execution_epoch"], claim_token=claim,
                 definition_sha256=job["definition_sha256"], input_fingerprint=digest(task["payload_json"].encode()) if task else job["definition_sha256"],
                 records=payload, files=list(files), acquired_at=acquired_at or utc(), download_bytes=download_bytes, reused=reused,
                 checkpoints=list(checkpoints), outcome_state=state, reason=reason, intent=intent)
    path = directory / "result.json"
    atomic_json(path, value)
    key = (stable_id("collection-intent-v1", job["lake_id"], job["id"], job["definition_sha256"]) if intent
           else stable_id("collection-batch-v1", job["lake_id"], job["id"], [receipt_id]))
    with service.state.db() as db:
        db.execute("INSERT INTO collection_outbox(id,job_id,task_id,execution_epoch,claim_token,dedupe_key,intent_path,content_sha256,state,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,'prepared',?,?)",
                   (receipt_id, job["id"], value["task_id"], value["execution_epoch"], claim, key, str(path.relative_to(service.state.root)), file_hash(path), utc(), utc()))
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
    path = contained(service.state.root, outbox["intent_path"])
    if file_hash(path) != outbox["content_sha256"]:
        raise IntegrityError("Prepared collection receipt changed")
    value = read_json(path)
    job = service.row(outbox["job_id"])
    if value["receipt_id"] != outbox["id"] or value["definition_sha256"] != job["definition_sha256"]:
        raise IntegrityError("Prepared collection receipt identity mismatch")
    final = lib.root / "segments" / value["batch_id"]
    staging_path = lib.root / "staging" / value["batch_id"]
    with lib.writer_lock():
        committed = lib.committed_key(outbox["dedupe_key"])
        if committed:
            if committed["batch_id"] != value["batch_id"]:
                raise IntegrityError("Collection result dedupe collision")
            seq = committed["seq"]
        elif (final / "manifest.json").exists() or (staging_path / "manifest.json").exists():
            directory = final if (final / "manifest.json").exists() else staging_path
            seq = lib.accept_manifest(directory, read_json(directory / "manifest.json"), fence=lambda _: fence(service, value))
        else:
            fence(service, value)
            with lib.journal() as journal:
                intent_row = journal.execute("SELECT intent_batch_id FROM collection_runs WHERE job_id=?", (job["id"],)).fetchone()
            batch = MediaBatch(lib, job["id"], job["definition_sha256"], [value["receipt_id"]],
                               intent_batch_id=intent_row[0] if intent_row else None,
                               definition=json.loads(job["definition_json"]) if value["intent"] else None,
                               batch_id=value["batch_id"], resume=True)
            records = value["records"]
            for captured in records.get("captures", []):
                captured["raw_body"] = base64.b64decode(captured.pop("raw_base64"), validate=True)
            for name, rows in records.items():
                batch.add(name, rows)
            origin = None
            assets = []
            for f in sorted(value["files"], key=lambda f: f["representation"] != "original"):
                stored = contained(path.parent, f["path"])
                if stored.stat().st_size != f["bytes"] or file_hash(stored) != f["sha256"]:
                    raise IntegrityError("Prepared media content changed")
                sha = batch.add_blob(stored.read_bytes(), f["stored_ext"], content_type=f["content_type"], media_category=f["media_category"], width=f["stored_width"], height=f["stored_height"])
                with service.state.db() as db:
                    task = db.execute("SELECT payload_json FROM collection_tasks WHERE id=?", (value["task_id"],)).fetchone()
                media_id = json.loads(task[0])["entry"]["media_id"]
                asset_id = stable_id("asset-record-v2", media_id, f["representation"], f["recipe_id"], value["receipt_id"], sha)
                assets.append(dict(asset_id=asset_id, media_id=media_id, sha256=sha, representation=f["representation"], recipe_id=f["recipe_id"],
                                   acquisition_receipt_id=value["receipt_id"], source_sha256=f["source_sha256"], source_bytes=f["source_bytes"],
                                   acquired_at=value["acquired_at"], last_verified_at=f["last_verified_at"], evidence=f["evidence"],
                                   derived_from_asset_id=origin if f["representation"] != "original" else None, details_json=f["details_json"]))
                if f["representation"] == "original":
                    origin = asset_id
            if assets:
                batch.add("assets", assets)
            if value["task_id"]:
                record_ids = [r[field] for name, field in (("author_observations", "observation_id"), ("work_observations", "observation_id"), ("media_manifests", "manifest_id")) for r in records.get(name, [])] + [a["asset_id"] for a in assets]
                batch.replay["task_outcomes"] = [dict(task_id=value["task_id"], execution_epoch=value["execution_epoch"], claim_token=value["claim_token"], state=value["outcome_state"], reason=value["reason"], record_ids=record_ids)]
            batch.replay["checkpoint_advances"] = value["checkpoints"]
            batch.replay["discovery_snapshot_ids"] = [r["snapshot_id"] for r in records.get("discovery_snapshots", [])]
            seq = batch.commit(fence=lambda _: fence(service, value))
    with service.state.db() as db:
        db.execute("UPDATE collection_outbox SET state='archive_committed',archive_seq=?,batch_id=?,updated_at=? WHERE id=?", (seq, value["batch_id"], utc(), outbox["id"]))
    failpoint("collection_after_outbox_archive")
    return seq


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
            outbox = db.execute("SELECT published FROM collection_outbox WHERE dedupe_key=?", (manifest["dedupe_key"],)).fetchone()
            for outcome in replay["task_outcomes"]:
                row = db.execute("SELECT * FROM collection_tasks WHERE id=? AND job_id=?", (outcome["task_id"], job["id"])).fetchone()
                if row is None:
                    raise IntegrityError("Archived task cannot be resolved from its earlier plan")
                db.execute("UPDATE collection_tasks SET state=?,reason=?,updated_at=? WHERE id=?", (outcome["state"], outcome["reason"], utc(), outcome["task_id"]))
                if row["kind"] == "media_download" and outcome["state"] == "done":
                    counters["archived_media"] += 1
                    if outbox and outbox[0]:
                        counters["published_media"] += 1
                    reused = bool(records.get("assets")) and all(a["evidence"] == "historical_reuse" for a in records["assets"])
                    counters["media_reused" if reused else "media_downloaded"] += 1
            counters["objects"] += len(records.get("objects", []))
            counters["browsable_images"] += sum(o["media_category"] == "image" for o in records.get("objects", []))
            for point in replay["checkpoint_advances"]:
                old = db.execute("SELECT revision FROM collection_checkpoints WHERE job_id=? AND stream_key=?", (job["id"], point["stream_key"])).fetchone()
                if (old[0] if old else 0) != point["expected_revision"]:
                    raise IntegrityError("Control checkpoint differs from authoritative archive")
                db.execute("INSERT INTO collection_checkpoints VALUES(?,?,?,?,?,?) ON CONFLICT(job_id,stream_key) DO UPDATE SET revision=excluded.revision,cursor_json=excluded.cursor_json,archive_seq=excluded.archive_seq,batch_id=excluded.batch_id",
                           (job["id"], point["stream_key"], point["next_revision"], canonical(dict(cursor=point["next_cursor"], exhausted=point["exhausted"], capture_id=point["capture_id"])), commit["seq"], commit["batch_id"]))
            planner.apply(db, job, records, replay["task_outcomes"])
            db.execute("UPDATE collection_jobs SET counters_json=?,revision=revision+1,updated_at=? WHERE id=?", (canonical(counters), utc(), job["id"]))
            db.execute("INSERT INTO collection_applied_batches VALUES(?,?,?,?,?,?)", (lib.info["library_id"], commit["seq"], commit["batch_id"], job["id"], hashed, utc()))
            db.execute("UPDATE collection_outbox SET control_applied=1,archive_seq=?,batch_id=?,state='archive_committed',updated_at=? WHERE dedupe_key=?",
                       (commit["seq"], commit["batch_id"], utc(), manifest["dedupe_key"]))
        after = commit["seq"]
        failpoint("collection_after_control")


def publish_ack(service, lib, job_id, *, stop=None):
    result = lib.sync_online(stop=stop)
    with service.state.db() as db:
        db.execute("BEGIN IMMEDIATE")
        job = service.row(job_id, db)
        counters = json.loads(job["counters_json"])
        delta = db.execute("SELECT count(*) FROM collection_outbox o JOIN collection_tasks t ON t.id=o.task_id WHERE o.job_id=? AND o.published=0 AND o.control_applied=1 AND o.archive_seq<=? AND t.kind='media_download' AND t.state='done'", (job_id, result["served_seq"])).fetchone()[0]
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
            AND (o.state='archive_committed' OR (o.state='needs_review' AND EXISTS(SELECT 1 FROM collection_tasks t WHERE t.id=o.task_id AND t.state='done')))
            ORDER BY o.archive_seq LIMIT 256""", (job_id,))]
    for row in rows:
        path = contained(service.state.root, row["intent_path"])
        directory = safe_managed_path(service.state.root, path.parent)
        if row["task_id"]:
            with service.state.db() as db:
                task = db.execute("SELECT state FROM collection_tasks WHERE id=?", (row["task_id"],)).fetchone()
            if task and task[0] in {"needs_review", "unavailable"} and directory.exists() and any(directory.glob("*.downloaded")):
                # Keep a failed original as bounded diagnostic evidence until cancellation
                # or a successful retry establishes a new accepted acquisition.
                with service.state.db() as db:
                    db.execute("UPDATE collection_outbox SET state='needs_review',updated_at=? WHERE id=?", (utc(), row["id"]))
                continue
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
