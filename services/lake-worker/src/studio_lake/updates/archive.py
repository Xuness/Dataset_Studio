"""Archive-first checkpoints and replay into the disposable execution queue."""

from contextlib import contextmanager
import json
import shutil
from pathlib import Path

import apsw
import pyarrow as pa
import pyarrow.parquet as pq

from ..library import Batch
from ..image_policy import profile_id
from ..metadata import asset
from ..online_schema import settings
from ..util import contained, digest, now, stable_id, read_json, failpoint
from .sites import UpdateError
from .io import device_lock


def record_update_commit(db, manifest, seq):
    db.execute(
        "CREATE TABLE IF NOT EXISTS update_run_batches(job_id TEXT NOT NULL,seq INTEGER NOT NULL,"
        "batch_id TEXT NOT NULL,PRIMARY KEY(job_id,seq))"
    )
    db.execute(
        "INSERT INTO update_run_batches VALUES(?,?,?)",
        (manifest["source"]["update_job_id"], seq, manifest["batch_id"]),
    )


@contextmanager
def online(lib):
    pointer = read_json(lib.cache / "ONLINE.json")
    if pointer["library_id"] != lib.info["library_id"]:
        raise UpdateError("SOURCE_ID_MISMATCH", "Online identity changed")
    db = apsw.Connection(str(contained(lib.cache, pointer["file"])), flags=apsw.SQLITE_OPEN_READONLY)
    db.set_busy_timeout(5000)
    db.execute("PRAGMA query_only=ON; PRAGMA cache_size=-16384")
    try:
        yield db, settings(db)
    finally:
        db.close()


def record_by_id(db, query, args):
    cur = db.execute(query, args)
    try:
        names = [x[0] for x in cur.get_description()]
    except apsw.ExecutionCompleteError:
        return None
    row = next(cur, None)
    return dict(zip(names, row)) if row else None


def io_lock(root, lib):
    return device_lock(root, lib.root)


def checkpoint_key(identity):
    return "update_checkpoint:" + identity


def commit_page(
    state,
    lib,
    job,
    site,
    response,
    rows,
    selected,
    cursor,
    *,
    expected_ids=None,
    error=None,
    retry=False,
    tag_types=None,
):
    role = "error" if error else "retry" if retry else "page"
    key = stable_id(
        "update-response-v1",
        job["id"],
        job["cursor"].get("pages", 0),
        digest(response.body),
        role,
        job["execution"] if retry else 0,
        response.request.get("endpoint"),
        response.request.get("parameters"),
    )
    old = lib.committed_key(key)
    if old:
        lib.sync_online()
        return old
    source = {
        "kind": "update_api",
        "site": site.name,
        "update_job_id": job["id"],
        "update_role": role,
        "definition": job["definition"],
        "observed_at": response.request.get("replayed_observed_at") or now(),
        "response_sha256": digest(response.body),
        "request": {**response.request, "status": response.status},
        "adapter_version": 1,
        "cursor": cursor,
        "expected_ids": expected_ids,
        "selected_ids": sorted(selected),
        "tag_types": tag_types,
    }
    if error:
        source["error_code"] = error.code
    with io_lock(state.root, lib), lib.writer_lock(), online(lib) as (db, status):
        if shutil.disk_usage(lib.root).free < 2 * 1024**3 + len(response.body) * 3:
            raise UpdateError("UPDATE_SPACE", "Waiting for archive disk space")
        batch = Batch(lib, key, source)
        batch.write_bytes("response_body.bin", response.body)
        failpoint("after_update_response_saved")
        if rows:
            batch.add_source(
                pa.table(
                    {
                        "post_json": [r[0] for r in rows],
                        "tag_types_json": [
                            json.dumps(
                                {
                                    tag: tag_types.get(tag)
                                    for tag in str(r[1].get("tags") or "").split(" ")
                                    if tag
                                }
                            )
                            if tag_types is not None
                            else None
                            for r in rows
                        ],
                    }
                )
            )
        for ordinal, (_, record) in enumerate(rows):
            if record["id"] not in selected or error:
                continue
            observation = site.normalize(record, key, ordinal, source["observed_at"], tag_types)
            # A refresh does not turn a previously imported base image into a monthly addition.
            old_post = record_by_id(
                db,
                "SELECT o.publication_kind,o.publication_group FROM post_versions p "
                "JOIN observations o USING(row_id) WHERE p.post_id=? AND p.valid_from<=? "
                "AND (p.valid_until IS NULL OR p.valid_until>?) ORDER BY p.valid_from DESC LIMIT 1",
                (record["id"], int(status["served_seq"]), int(status["served_seq"])),
            )
            if old_post:
                observation.update(old_post)
            batch.observations.append(observation)
        save = (
            {}
            if error or retry
            else {checkpoint_key(job["id"]): {"definition": job["definition"], "cursor": cursor}}
        )
        seq = batch.commit(settings=save)
    return {"seq": seq, "batch_id": batch.id}


def resume_response(lib, path):
    from .sites import Site, split_response

    batch = Batch.resume_metadata(lib, path)
    body = (path / "response_body.bin").read_bytes()
    if digest(body) != batch.source["response_sha256"]:
        raise UpdateError("UPDATE_INTEGRITY", "Saved API response checksum mismatch")
    role = batch.source["update_role"]
    rows = [] if role == "error" else split_response(batch.source["site"], body)
    if rows:
        types = batch.source.get("tag_types")
        batch.add_source(
            pa.table(
                {
                    "post_json": [r[0] for r in rows],
                    "tag_types_json": [
                        json.dumps(
                            {tag: types.get(tag) for tag in str(r[1].get("tags") or "").split(" ") if tag}
                        )
                        if types is not None
                        else None
                        for r in rows
                    ],
                }
            )
        )
    site = Site(batch.source["site"])
    try:
        with online(lib) as (db, status):
            for ordinal, (_, record) in enumerate(rows):
                if record["id"] not in batch.source["selected_ids"]:
                    continue
                obs = site.normalize(
                    record, batch.key, ordinal, batch.source["observed_at"], batch.source.get("tag_types")
                )
                previous = record_by_id(
                    db,
                    "SELECT o.publication_kind,o.publication_group FROM post_versions p "
                    "JOIN observations o USING(row_id) WHERE p.post_id=? AND p.valid_from<=? "
                    "AND (p.valid_until IS NULL OR p.valid_until>?) ORDER BY p.valid_from DESC LIMIT 1",
                    (record["id"], int(status["served_seq"]), int(status["served_seq"])),
                )
                if previous:
                    obs.update(previous)
                batch.observations.append(obs)
    finally:
        site.close()
    save = (
        {}
        if role in {"error", "retry"}
        else {
            checkpoint_key(batch.source["update_job_id"]): {
                "definition": batch.source["definition"],
                "cursor": batch.source["cursor"],
            }
        }
    )
    return batch.seal(settings=save)


def commit_media(state, lib, job, results, cursor):
    key = stable_id(
        "update-media-v1",
        job["id"],
        *[
            f"{r['post_id']}:{r['observation_id']}:{r['state']}:{r.get('sha256')}:{r.get('attempt', 1)}"
            for r in results
        ],
    )
    old = lib.committed_key(key)
    if old:
        lib.sync_online()
        return old
    with io_lock(state.root, lib), lib.writer_lock(), online(lib) as (db, status):
        if (
            shutil.disk_usage(lib.root).free
            < 2 * 1024**3 + sum(r.get("stored_bytes", 0) for r in results) * 2
        ):
            raise UpdateError("UPDATE_SPACE", "Waiting for archive disk space")
        batch = Batch(
            lib,
            key,
            {
                "kind": "update_media",
                "update_job_id": job["id"],
                "update_role": "media",
                "definition": job["definition"],
                "results": results,
                "cursor": cursor,
            },
        )
        for result in results:
            if result["state"] != "stored":
                continue
            observation = record_by_id(
                db,
                "SELECT * FROM observations WHERE observation_id=? AND commit_seq<=?",
                (result["observation_id"], int(status["served_seq"])),
            )
            if observation is None:
                raise UpdateError("UPDATE_INTEGRITY", "Image observation is not published")
            data = Path(result["ready_path"]).read_bytes()
            if digest(data) != result["sha256"]:
                raise UpdateError("UPDATE_INTEGRITY", "Prepared image hash mismatch")
            sha, ext = batch.add_blob(
                data,
                result["stored_ext"],
                lambda sha: record_by_id(
                    db,
                    "SELECT pack_path,offset,length FROM objects WHERE sha256=? AND first_seq<=?",
                    (sha, int(status["served_seq"])),
                ),
            )
            a = asset(
                observation, sha, ext, len(data), profile_id(job["definition"]["media"]), result["details"]
            )
            batch.assets.append(a)
            result["asset_id"] = a["asset_id"]
        seq = batch.commit()
    return {"seq": seq, "batch_id": batch.id}


def reconcile(state, lib, identity):
    """Archive commit precedes queue mutation. Reapplying the same batch is a no-op."""
    lib.sync_online()
    with state.db() as control:
        after = control.execute(
            "SELECT coalesce(max(seq),0) FROM applied_batches WHERE job_id=?", (identity,)
        ).fetchone()[0]
    while True:
        with lib.journal() as journal:
            if not journal.execute("SELECT 1 FROM sqlite_master WHERE name='update_run_batches'").fetchone():
                return
            batches = journal.execute(
                "SELECT c.seq,c.batch_id,c.manifest_json FROM commits c JOIN update_run_batches b USING(seq,batch_id) "
                "WHERE b.job_id=? AND c.seq>? ORDER BY c.seq LIMIT 32",
                (identity, after),
            ).fetchall()
        if not batches:
            return
        for row in batches:
            manifest = json.loads(row["manifest_json"])
            source = manifest["source"]
            directory = lib.root / "segments" / row["batch_id"]
            with state.db() as db:
                if db.execute(
                    "SELECT 1 FROM applied_batches WHERE batch_id=?", (row["batch_id"],)
                ).fetchone():
                    after = row["seq"]
                    continue
                if source["update_role"] in {"page", "retry"}:
                    observations = pq.read_table(directory / "observations.parquet").to_pylist()
                    raw = pq.read_table(directory / "source.parquet").to_pylist() if observations else []
                    for obs in observations:
                        state_name = (
                            "metadata"
                            if source["definition"]["media"]["profile"] == "metadata_only"
                            else "pending"
                        )
                        db.execute(
                            "INSERT INTO items(job_id,post_id,observation_id,record_json,state) VALUES(?,?,?,?,?) "
                            "ON CONFLICT(job_id,post_id) DO UPDATE SET observation_id=excluded.observation_id,"
                            "record_json=excluded.record_json,state=excluded.state,reason=NULL",
                            (
                                identity,
                                obs["post_id"],
                                obs["observation_id"],
                                raw[obs["archive_row"]]["post_json"],
                                state_name,
                            ),
                        )
                    returned = {o["post_id"] for o in observations}
                    for missing in set(source.get("expected_ids") or []) - returned:
                        db.execute(
                            "INSERT INTO items(job_id,post_id,record_json,state,reason) VALUES(?,?,'{}','unavailable','not_returned_by_api') "
                            "ON CONFLICT(job_id,post_id) DO UPDATE SET state='unavailable',reason='not_returned_by_api'",
                            (identity, missing),
                        )
                elif source["update_role"] == "media":
                    for result in source["results"]:
                        db.execute(
                            "UPDATE items SET state=?,reason=?,asset_id=?,attempts=attempts+1,retry_at=? WHERE job_id=? AND post_id=?",
                            (
                                result["state"],
                                result.get("reason"),
                                result.get("asset_id"),
                                result.get("retry_at", 0),
                                identity,
                                result["post_id"],
                            ),
                        )
                if source["update_role"] in {"page", "register", "progress"}:
                    db.execute(
                        "UPDATE jobs SET cursor=?,updated_at=? WHERE id=?",
                        (json.dumps(source["cursor"]), now(), identity),
                    )
                db.execute(
                    "INSERT INTO applied_batches VALUES(?,?,?)", (row["batch_id"], identity, row["seq"])
                )
            after = row["seq"]
