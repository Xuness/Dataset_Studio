"""Durable cleanup of settled or superseded acquisition generations, under the job lock."""

import json
import re

from .downloader import download_key
from ..util import IntegrityError, contained


def enqueue(db, job_id, task, entry):
    db.execute("INSERT INTO pinterest_download_cleanup VALUES(?,?,?,?,?) ON CONFLICT DO NOTHING",
        (job_id, download_key(job_id, task, entry), entry["normalized_url"], entry["context_id"], task["download_generation"]))


def collect(state, lib, job_id):
    with state.db() as db:
        rows = db.execute("SELECT * FROM pinterest_download_cleanup WHERE job_id=? ORDER BY download_key LIMIT 128", (job_id,)).fetchall()
    for row in rows:
        key = row["download_key"]
        if not re.fullmatch("[0-9a-f]{64}", key):
            raise IntegrityError("Invalid Pinterest acquisition cleanup key")
        with state.db() as db:
            pending = db.execute("SELECT 1 FROM pinterest_tasks WHERE job_id=? AND kind='media_download' "
                "AND state IN ('queued','running','waiting_retry','waiting_budget') AND download_generation=? "
                "AND json_extract(input_json,'$.normalized_url')=? AND json_extract(input_json,'$.context_id')=? LIMIT 1",
                (job_id, row["generation"], row["normalized_url"], row["context_id"])).fetchone()
            saved = db.execute("SELECT acquisition_json FROM pinterest_downloads WHERE job_id=? AND download_key=?", (job_id, key)).fetchone()
        # A published object can serve other bindings after its download file is gone.
        if pending and not (saved and lib.lookup_object(json.loads(saved[0])["download_sha256"])):
            continue
        for suffix in ("downloaded", "download.json", "partial", "partial.json"):
            contained(lib.cache, "pinterest_downloads/" + job_id + "/" + key + "." + suffix).unlink(missing_ok=True)
        with state.db() as db:
            db.execute("DELETE FROM pinterest_download_cleanup WHERE job_id=? AND download_key=?", (job_id, key))
