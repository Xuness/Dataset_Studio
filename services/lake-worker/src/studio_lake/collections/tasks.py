"""Short claim transactions and one mutable download workspace per logical task.

The caller holds the job execution lock until every worker future has joined.
Claim directories contain immutable result attempts; only the current owner may
write the task's partial/downloaded files. No mutable hardlinks or retry copies.
"""

import time
import uuid

from . import receipts
from ..media_lake.schema import utc
from ..util import IntegrityError, atomic_json, digest, read_json, safe_managed_path


def claim(service, job, *, media_task=False):
    with service.state.db() as db:
        db.execute("BEGIN IMMEDIATE")
        current = service.row(job["id"], db)
        if current["desired_state"] != "run" or current["execution_epoch"] != job["execution_epoch"]:
            return None
        row = db.execute("SELECT * FROM collection_tasks WHERE job_id=? AND kind " + ("=" if media_task else "<>") +
                         " 'media_download' AND state IN ('queued','retry_wait') AND retry_at_ms<=? ORDER BY priority DESC,task_row LIMIT 1",
                         (job["id"], int(time.time() * 1000))).fetchone()
        if row is None:
            return None
        task = dict(row)
        task.update(claimed_epoch=job["execution_epoch"], claim_token=str(uuid.uuid4()), state="running", attempts=task["attempts"] + 1)
        directory = service.state.root / "collection-spool" / job["id"] / task["claim_token"]
        marker = str((directory / "result.json").relative_to(service.state.root))
        db.execute("UPDATE collection_tasks SET state='running',claimed_epoch=?,claim_token=?,attempts=?,result_receipt=?,reason=NULL,updated_at=? WHERE id=?",
                   (task["claimed_epoch"], task["claim_token"], task["attempts"], marker, utc(), task["id"]))
    # All filesystem work follows the short transaction, under the execution lock.
    task["directory"] = receipts.staging(service, job, task["claim_token"])
    if media_task:
        task["download_directory"] = download_path(service, job["id"], task["id"])
    elif task["result_receipt"]:
        prune_attempt(service, job, (service.state.root / task["result_receipt"]).parent)
    return task


def download_path(service, job_id, task_id):
    return safe_managed_path(service.state.root, service.state.root / "collection-downloads" / job_id / task_id)


def remove_flat(state, directory):
    directory = safe_managed_path(state.root, directory)
    for child in directory.iterdir():
        child = safe_managed_path(state.root, child)
        if not child.is_file():
            raise IntegrityError("Unexpected nested collection scratch")
        child.unlink()
    directory.rmdir()


def prepare_download(service, job, task):
    """Called only after capacity reservation; legacy adoption is a same-volume move."""
    directory = task["download_directory"]
    directory.mkdir(parents=True, exist_ok=True)
    owner = dict(job_id=job["id"], task_id=task["id"], definition_sha256=job["definition_sha256"],
                 input_sha256=digest(task["payload_json"].encode()), generation=task["download_generation"])
    path = directory / "owner.json"
    if path.exists():
        previous = read_json(path)
        if any(previous.get(k) != owner[k] for k in owner if k != "generation"):
            raise IntegrityError("Download workspace owner changed")
        if previous.get("generation") != owner["generation"]:
            # Explicit retry_failed requests a new source fetch. Resume/retry_wait
            # keep the generation and therefore preserve the one valid checkpoint.
            remove_flat(service.state, directory)
            directory.mkdir()
    elif any(directory.iterdir()):
        raise IntegrityError("Unowned download workspace")
    atomic_json(path, owner)
    if task["result_receipt"]:
        old = safe_managed_path(service.state.root, (service.state.root / task["result_receipt"]).parent)
        if old.exists():
            old_owner = read_json(old / "owner.json")
            if old_owner.get("job_id") != job["id"] or old_owner.get("definition_sha256") != job["definition_sha256"]:
                raise IntegrityError("Legacy download workspace owner changed")
            with service.state.db() as db:
                pending = db.execute("SELECT 1 FROM collection_outbox WHERE intent_path=? AND state NOT IN ('released','needs_review') LIMIT 1",
                                     (task["result_receipt"],)).fetchone()
            if pending:
                raise IntegrityError("A pending receipt still owns the previous download")
            if task["download_generation"] == 0:
                for suffix in (".downloaded", ".download.json", ".partial", ".partial.json"):
                    source = safe_managed_path(service.state.root, old / (task["id"] + suffix))
                    target = directory / source.name
                    if source.is_file() and not target.exists():
                        source.rename(target)
            prune_attempt(service, job, old)
    return directory


def prune_attempt(service, job, directory, resources=None):
    directory = safe_managed_path(service.state.root, directory)
    if not directory.exists():
        return
    with service.state.db() as db:
        referenced = db.execute("SELECT 1 FROM collection_outbox WHERE intent_path=? AND state NOT IN ('released','quarantined') LIMIT 1",
                                (str((directory / "result.json").relative_to(service.state.root)),)).fetchone()
    if referenced:
        return
    owner = read_json(directory / "owner.json")
    if owner.get("job_id") != job["id"] or owner.get("definition_sha256") != job["definition_sha256"]:
        raise IntegrityError("Attempt cleanup owner changed")
    remove_flat(service.state, directory)
    if resources:
        resources.retire(directory)


def retire_download(service, job_id, task_id, resources=None):
    directory = download_path(service, job_id, task_id)
    if directory.exists():
        if not any(directory.iterdir()):
            directory.rmdir()
        else:
            _retire_owned_download(service, job_id, task_id, directory)
    if resources:
        resources.retire(directory)
    if directory.parent.exists() and not any(directory.parent.iterdir()):
        directory.parent.rmdir()


def _retire_owned_download(service, job_id, task_id, directory):
    owner = read_json(directory / "owner.json")
    job = service.row(job_id)
    if owner.get("job_id") != job_id or owner.get("task_id") != task_id or owner.get("definition_sha256") != job["definition_sha256"]:
        raise IntegrityError("Download cleanup owner changed")
    remove_flat(service.state, directory)


def download_evidence(task):
    directory = task.get("download_directory", task["directory"])
    path = directory / (task["id"] + ".download.json")
    if not path.is_file():
        return {}
    value = read_json(path)
    raw = directory / (task["id"] + ".downloaded")
    return dict(download=dict(sha256=value.get("sha256"), bytes=raw.stat().st_size if raw.is_file() else None,
                              generation=task.get("download_generation", 0)))
