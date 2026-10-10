"""Physical bundle publication, staging ownership, and restart cleanup."""

from pathlib import Path

from . import garbage, outbox, receipts, reuse
from .lake.library import Batch
from .lake import storage
from ..canonical import utc
from ..updates.sites import UpdateError
from ..util import IntegrityError, contained, failpoint

OVERHEAD = 64 * 1024


def restore_job(state, lib, job_id, resources):
    with state.db() as db:
        rows = [dict(r) for r in db.execute("SELECT * FROM pinterest_outbox WHERE job_id=? AND cleanup_complete=0 AND state<>'cancelled' ORDER BY created_at,receipt_id LIMIT 128", (job_id,))]
    for row in rows:
        root = receipts.directory(lib, job_id)
        size = sum(p.stat().st_size for p in (root / (row["receipt_id"] + ".prepared.json"), root / (row["receipt_id"] + ".response.json")) if p.is_file())
        resources.restore(root, row["receipt_id"], size + OVERHEAD)
        if row["download_key"]:
            directory = lib.cache / "pinterest_downloads" / job_id
            size = max((p.stat().st_size for p in (directory / (row["download_key"] + ".downloaded"), directory / (row["download_key"] + ".partial")) if p.is_file()), default=0)
            resources.restore(directory, row["download_key"], size * 2 + OVERHEAD,
                              related=[(lib.cache / "pack_staging", row["receipt_id"])])
    with state.db() as db:
        plans = [dict(r) for r in db.execute("SELECT * FROM pinterest_batches WHERE job_id=? AND cleanup_complete=0 AND state<>'cancelled' ORDER BY created_at LIMIT 128", (job_id,))]
    for plan in plans:
        combine(state, lib, plan, resources)


def restore_all(state, service, resources):
    # Startup precedes every source worker's admission. Track real files even for offline archives.
    with state.db() as db:
        lakes = [dict(r) for r in db.execute("SELECT l.id,l.index_root FROM lakes l JOIN pinterest_lakes p ON p.lake_id=l.id")]
    for lake in lakes:
        cache = Path(lake["index_root"])
        if not cache.is_dir():
            continue
        for folder in ("pinterest_receipts", "pinterest_downloads"):
            (cache / folder).mkdir(exist_ok=True)
            resources.track(cache / folder / "tracked-job")
        resources.track(cache / "pack_staging")
        with state.db() as db:
            jobs = [r[0] for r in db.execute("SELECT id FROM pinterest_jobs WHERE lake_id=? AND (EXISTS(SELECT 1 FROM pinterest_outbox WHERE job_id=id AND cleanup_complete=0 AND state<>'cancelled') OR EXISTS(SELECT 1 FROM pinterest_batches WHERE job_id=id AND cleanup_complete=0 AND state<>'cancelled'))", (lake["id"],))]
        for identity in jobs:
            try:
                with service.execution_lock(identity):
                    restore_job(state, service.library(lake["id"]), identity, resources)
            except (IntegrityError, OSError, UpdateError, RuntimeError):
                # Source-specific recovery owns diagnostics; this must not stop unrelated lakes.
                continue


def combine(state, lib, plan, resources):
    existing = resources.retained.get((lib.cache / "pack_staging", plan["batch_id"]))
    if existing:
        return existing
    leases = []
    for row in outbox.members(state, plan):
        lease = resources.retained_lease(receipts.directory(lib, row["job_id"]), row["receipt_id"])
        if lease:
            leases.append(lease)
        if row["download_key"]:
            lease = resources.retained_lease(lib.cache / "pinterest_downloads" / row["job_id"], row["download_key"])
            if lease:
                leases.append(lease)
    if not leases:
        return resources.restore(lib.cache / "pack_staging", plan["batch_id"], OVERHEAD)
    return resources.combine(leases, lib.cache / "pack_staging", plan["batch_id"])


def publish(state, lib, job, resources, fence, cancelled):
    plan = outbox.freeze(state, lib, job)
    if plan is None:
        return False
    restore_job(state, lib, job["id"], resources)
    combine(state, lib, plan, resources)
    rows = outbox.members(state, plan)
    sealed = next((p for p in (lib.root / "segments" / plan["batch_id"], lib.root / "staging" / plan["batch_id"])
                   if storage.exists(p / "manifest.json")), None)
    with lib.writer_lock():
        if sealed:
            manifest = storage.json_file(sealed / "manifest.json")
        else:
            snapshot = resources.snapshot()
            if snapshot["staging_reserved_bytes"] > snapshot["staging_limit_bytes"]:
                raise UpdateError("UPDATE_SPACE", "Existing Pinterest staging exceeds the current resource budget")
            values = [outbox.load(lib, row) for row in rows]
            records = outbox.merge(values)
            members, seen = [], set()
            for row, value in zip(rows, values):
                replay = {**value["replay"], "metrics": dict(value["replay"].get("metrics", {}))}
                if row["version"] == 2:
                    replay.update(prepared_sha256=row["content_sha256"], input_sha256=row["input_sha256"])
                    for asset in value["records"].get("assets", []):
                        sha = asset["sha256"]
                        metric = "existing_byte_bindings" if sha in seen or lib.lookup_object(sha) else "new_byte_objects"
                        replay["metrics"][metric] = replay["metrics"].get(metric, 0) + 1
                        seen.add(sha)
                members.append(replay)
            grouped = rows[0]["version"] == 2
            if grouped:
                lib.enable_bundles()
            replay = dict(kind="bundle", receipts=members) if grouped else members[0]
            batch = Batch(lib, job, replay, records, batch_id=plan["batch_id"])
            try:
                for value in values:
                    media = value["media"]
                    if media is None:
                        continue
                    path = contained(lib.cache, "pinterest_downloads/" + job["id"] + "/" + media["download_key"] + ".downloaded")
                    if not path.is_file():
                        acquisition = value["records"]["acquisitions"][0]
                        if not reuse.object_available(lib, acquisition, resources, cancelled):
                            raise IntegrityError("Pinterest prepared media is missing")
                        continue
                    if path.stat().st_size > resources.max_download_bytes:
                        raise UpdateError("UPDATE_SPACE", "Prepared Pinterest media exceeds the current download budget")
                    if path.stat().st_size * 2 > resources.config["decode_memory_mib"] * 1024**2:
                        raise UpdateError("UPDATE_SPACE", "Prepared Pinterest media exceeds the current packing memory budget")
                    with resources.encoding(max(1, path.stat().st_size * 2), cancelled):
                        sha = batch.add_blob(path.read_bytes(), media["ext"], width=media["width"], height=media["height"], content_type=media["content_type"])
                    if sha != media["download_sha256"]:
                        raise IntegrityError("Pinterest staged bytes changed after preparation")
                    failpoint("pinterest_during_pack")
                manifest = batch.seal()
            finally:
                if batch.tar:
                    batch.tar.close()
                if batch.pack_writer and not batch.pack_writer.closed:
                    batch.pack_writer.close()
            sealed = batch.path
        if manifest["dedupe_key"] != plan["dedupe_key"]:
            raise IntegrityError("Pinterest sealed batch differs from its frozen plan")
        lib.accept_manifest(sealed, manifest, fence=fence)
        while receipts.replay(state, lib, job["id"]) == 128:
            pass
        published = lib.sync_online()
    with state.db() as db:
        db.execute("UPDATE pinterest_jobs SET served_seq=?,updated_at=? WHERE id=?", (published["served_seq"], utc(), job["id"]))
    failpoint("pinterest_after_control_replay")
    cleanup(state, lib, job["id"], resources, published["served_seq"])
    return True


def cleanup(state, lib, job_id, resources, served_seq):
    with state.db() as db:
        db.execute("UPDATE pinterest_outbox SET state='published' WHERE job_id=? AND state='accepted' AND archive_seq<=?", (job_id, served_seq))
        db.execute("UPDATE pinterest_batches SET state='published' WHERE job_id=? AND state='accepted' AND archive_seq<=?", (job_id, served_seq))
        rows = [dict(r) for r in db.execute("SELECT * FROM pinterest_outbox WHERE job_id=? AND state='published' AND cleanup_complete=0 LIMIT 128", (job_id,))]
    garbage.collect(state, lib, job_id)
    for row in rows:
        for suffix in ("prepared.json", "response.json"):
            (receipts.directory(lib, job_id) / (row["receipt_id"] + "." + suffix)).unlink(missing_ok=True)
        failpoint("pinterest_during_cleanup")
        with state.db() as db:
            db.execute("UPDATE pinterest_outbox SET cleanup_complete=1 WHERE receipt_id=?", (row["receipt_id"],))
    with state.db() as db:
        batches = [r[0] for r in db.execute("SELECT batch_id FROM pinterest_batches b WHERE job_id=? AND state='published' AND cleanup_complete=0 AND NOT EXISTS(SELECT 1 FROM pinterest_outbox o WHERE o.batch_id=b.batch_id AND o.cleanup_complete=0) LIMIT 128", (job_id,))]
    for batch_id in batches:
        contained(lib.cache, "pack_staging/" + batch_id + ".tar.tmp").unlink(missing_ok=True)
        lease = resources.retained_lease(lib.cache / "pack_staging", batch_id)
        if lease:
            lease.release()
        with state.db() as db:
            db.execute("UPDATE pinterest_batches SET cleanup_complete=1 WHERE batch_id=?", (batch_id,))


def cancel(state, lib, job_id, resources):
    with state.db() as db:
        rows = outbox.pending(db, job_id)
        plans = [dict(r) for r in db.execute("SELECT * FROM pinterest_batches WHERE job_id=? AND state='prepared'", (job_id,))]
    for plan in plans:
        if lib.committed_key(plan["dedupe_key"]):
            raise IntegrityError("Replay accepted Pinterest batches before cancellation")
        for parent in ("staging", "segments"):
            root = contained(lib.root, parent + "/" + plan["batch_id"])
            if root.is_dir():
                for path in root.iterdir():
                    if path.is_file() and (path.suffix == ".parquet" or path.name in {"media.tar", "run.json", "replay.json", "manifest.json"}):
                        path.unlink()
                if not any(root.iterdir()):
                    root.rmdir()
        contained(lib.cache, "pack_staging/" + plan["batch_id"] + ".tar.tmp").unlink(missing_ok=True)
        lease = resources.retained_lease(lib.cache / "pack_staging", plan["batch_id"])
        if lease:
            lease.release()
    for row in rows:
        for directory, key in ((receipts.directory(lib, job_id), row["receipt_id"]),
                               (lib.cache / "pinterest_downloads" / job_id, row["download_key"])):
            lease = resources.retained_lease(directory, key) if key else None
            if lease:
                lease.release()
    with state.db() as db:
        db.execute("UPDATE pinterest_outbox SET state='cancelled',cleanup_complete=1 WHERE job_id=? AND state='prepared'", (job_id,))
        db.execute("UPDATE pinterest_batches SET state='cancelled',cleanup_complete=1 WHERE job_id=? AND state='prepared'", (job_id,))
