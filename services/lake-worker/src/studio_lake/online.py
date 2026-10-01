"""One ordered publisher per lake, replaying immutable archive commits in short transactions."""

from pathlib import Path
from contextlib import closing
import hashlib
import json
import sqlite3
import time

import pyarrow as pa
import pyarrow.parquet as pq

from .metadata import OBS_SCHEMA, ASSET_SCHEMA
from .online_schema import VERSION, settings, set_state
from .online_storage import connect, scalar
from .raw_codec import encode as encode_raw, source_record
from .util import FileLock, IntegrityError, contained, digest, file_hash, logical_rows, now, read_json, same_directory


def configured_root(media):
    media = Path(media)
    path = media / "online-index.json"
    if not path.exists():
        return None
    pointer = read_json(path)
    if (
        pointer.get("schema_version") != VERSION
        or pointer.get("library_id") != read_json(media / "library.json")["library_id"]
    ):
        raise IntegrityError("在线发布配置身份不一致")
    return Path(pointer["index_root"])


class Publisher:
    def __init__(self, media, index):
        self.media, self.index = Path(media).resolve(), Path(index).resolve()
        self.pointer = read_json(self.index / "ONLINE.json")
        if (
            self.pointer.get("schema_version") != VERSION
            or self.pointer["library_id"] != read_json(self.media / "library.json")["library_id"]
        ):
            raise IntegrityError("在线库身份或格式无效")
        self.path = contained(self.index, self.pointer["file"])

    def check_location(self):
        if (self.index / "LAKE-RELOCATION.json").exists():
            raise IntegrityError("数据湖位置迁移尚未完成，禁止发布")
        owner = self.index / "cache_owner.json"
        if owner.exists() and not same_directory(read_json(owner).get("root"), self.media):
            raise IntegrityError("发布器指向旧媒体位置")

    def mark_analysis(self, sequence, generation):
        with FileLock(self.index / ".online.lock"):
            self.check_location()
            with closing(sqlite3.connect((self.media / "journal.sqlite").as_uri() + "?mode=ro", uri=True)) as journal:
                archive_head = journal.execute("SELECT coalesce(max(seq),0) FROM commits").fetchone()[0]
            db = connect(self.path)
            try:
                with db:
                    state = settings(db)
                    if sequence > archive_head:
                        raise IntegrityError("分析水位不能超出已归档提交")
                    if sequence >= int(state["analysis_seq"]):
                        set_state(
                            db,
                            archive_seq=archive_head,
                            analysis_seq=sequence,
                            analysis_generation=generation,
                        )
            finally:
                db.close()

    def sync(self, *, chunk_rows=256, stop=None):
        """Chunks are durable but invisible until the final publication watermark.

        `stop(phase, sequence)` is a fault/cancellation hook used by offline tests.
        A stopped publication resumes from immutable rows; readers keep their old view.
        """
        if not 1 <= chunk_rows <= 4096:
            raise ValueError("在线发布批次需要 1–4096 行")
        with FileLock(self.index / ".online.lock"):
            self.check_location()
            db = connect(self.path)
            try:
                state = settings(db)
                if state["generation"] != self.pointer["generation"]:
                    raise IntegrityError("在线发布代次不匹配")
                with closing(sqlite3.connect(
                    (self.media / "journal.sqlite").as_uri() + "?mode=ro", uri=True
                )) as journal:
                    journal.row_factory = sqlite3.Row
                    pending = [
                        dict(r)
                        for r in journal.execute(
                            "SELECT * FROM commits WHERE seq>? ORDER BY seq", (int(state["served_seq"]),)
                        )
                    ]
                    head = journal.execute("SELECT coalesce(max(seq),0) FROM commits").fetchone()[0]
                with db:
                    set_state(db, archive_seq=head)
                for commit in pending:
                    self._apply(db, commit, chunk_rows, stop)
                return {
                    "archive_seq": head,
                    "served_seq": int(settings(db)["served_seq"]),
                    "published": len(pending),
                }
            finally:
                db.close()

    def _apply(self, db, commit, chunk_rows, stop):
        seq, batch = commit["seq"], commit["batch_id"]
        state = settings(db)
        if seq != int(state["served_seq"]) + 1:
            raise IntegrityError("在线发布提交日志不连续")
        manifest = json.loads(commit["manifest_json"])
        fingerprint = hashlib.sha256(commit["manifest_json"].encode()).hexdigest()
        directory = contained(self.media, "segments/" + batch)
        if manifest["library_id"] != self.pointer["library_id"] or manifest["batch_id"] != batch:
            raise IntegrityError("封存批次身份无效")
        for name in ["observations.parquet", "assets.parquet", "objects.parquet", "source.parquet"]:
            path = directory / name
            info = manifest.get("files", {}).get(name)
            if info and (
                not path.is_file()
                or path.stat().st_size != info["bytes"]
                or file_hash(path) != info["sha256"]
            ):
                raise IntegrityError("封存元数据校验失败: " + name)
        existing = list(
            db.execute("SELECT batch_id,fingerprint FROM pending_publication WHERE seq=?", (seq,))
        )
        if existing and existing[0] != (batch, fingerprint):
            raise IntegrityError("重放批次内容发生变化")
        with db:
            db.execute("INSERT OR IGNORE INTO pending_publication VALUES(?,?,?)", (seq, batch, fingerprint))
        if stop:
            stop("before_projection", seq)
        # Disk-backed work tables bound both Python memory and publication locks.
        # They are reconstructed from the sealed batch on an interrupted replay.
        db.execute(
            "DROP TABLE IF EXISTS temp.work_posts; DROP TABLE IF EXISTS temp.work_objects; DROP TABLE IF EXISTS temp.work_raw;"
        )
        db.execute(
            "CREATE TEMP TABLE work_posts(id INTEGER PRIMARY KEY,fields TEXT NOT NULL DEFAULT '[]'); CREATE TEMP TABLE work_objects(sha TEXT PRIMARY KEY,fields TEXT NOT NULL) WITHOUT ROWID; CREATE TEMP TABLE work_raw(archive_row INTEGER,observation_id TEXT,source_kind TEXT,PRIMARY KEY(archive_row,observation_id)) WITHOUT ROWID;"
        )

        def batches(name):
            return pq.ParquetFile(directory / name).iter_batches(batch_size=chunk_rows)

        def impact(sha, fields):
            old = list(db.execute("SELECT fields FROM work_objects WHERE sha=?", (sha,)))
            fields = sorted(set(fields) | (set(json.loads(old[0][0])) if old else set()))
            db.execute("INSERT OR REPLACE INTO work_objects VALUES(?,?)", (sha, json.dumps(fields)))

        for block in batches("objects.parquet"):
            objects = block.to_pylist()
            with db:
                db.executemany(
                    "INSERT OR IGNORE INTO objects(sha256,pack_path,offset,length,stored_ext,first_seq) VALUES(?,?,?,?,?,?)",
                    (
                        (
                            r["sha256"],
                            f"segments/{batch}/images.tar",
                            r["offset"],
                            r["length"],
                            r["stored_ext"],
                            seq,
                        )
                        for r in objects
                    ),
                )
                for r in objects:
                    impact(r["sha256"], {"media", "stored.bytes", "stored.extension"})
            if stop:
                stop("objects", seq)
        obs_names = OBS_SCHEMA.names
        obs_sql = (
            "INSERT OR IGNORE INTO observations("
            + ",".join('"' + n + '"' for n in obs_names)
            + ",batch_id,commit_seq) VALUES("
            + ",".join("?" for _ in range(len(obs_names) + 2))
            + ") RETURNING row_id"
        )
        for block in batches("observations.parquet"):
            with db:
                for r in block.to_pylist():
                    db.execute(
                        "INSERT OR IGNORE INTO work_raw VALUES(?,?,?)",
                        (r["archive_row"], r["observation_id"], r["source_kind"]),
                    )
                    if r.get("post_id") is not None:
                        old = list(
                            db.execute(
                                "SELECT "
                                + ",".join('o."' + n + '"' for n in obs_names)
                                + " FROM post_versions p JOIN observations o ON o.row_id=p.row_id WHERE p.post_id=? AND p.valid_from<? AND (p.valid_until IS NULL OR p.valid_until>=?) ORDER BY p.valid_from DESC LIMIT 1",
                                (r["post_id"], seq, seq),
                            )
                        )
                        fields = {
                            n for i, n in enumerate(obs_names) if not old or old[0][i] != scalar(r.get(n), n)
                        } | {"observations", "raw_metadata"}
                        previous = list(
                            db.execute("SELECT fields FROM work_posts WHERE id=?", (r["post_id"],))
                        )
                        if previous:
                            fields.update(json.loads(previous[0][0]))
                        db.execute(
                            "INSERT OR REPLACE INTO work_posts VALUES(?,?)",
                            (r["post_id"], json.dumps(sorted(fields))),
                        )
                    rows = list(db.execute(obs_sql, (*[scalar(r.get(n), n) for n in obs_names], batch, seq)))
                    if not rows:
                        continue
                    tokens = []
                    for tag in sorted(set((r.get("tag_string") or "").split(" ")) - {""}):
                        db.execute("INSERT OR IGNORE INTO tags(tag) VALUES(?)", (tag,))
                        tid = next(db.execute("SELECT tag_id FROM tags WHERE tag=?", (tag,)))[0]
                        tokens.append("t" + format(tid, "x"))
                    db.execute(
                        "INSERT INTO tag_index(rowid,tokens) VALUES(?,?)", (rows[0][0], " ".join(tokens))
                    )
            if stop:
                stop("observations", seq)
        names = ASSET_SCHEMA.names
        asset_sql = (
            "INSERT OR IGNORE INTO assets("
            + ",".join('"' + n + '"' for n in names)
            + ",batch_id,commit_seq) VALUES("
            + ",".join("?" for _ in range(len(names) + 2))
            + ")"
        )
        for block in batches("assets.parquet"):
            assets = block.to_pylist()
            with db:
                db.executemany(
                    asset_sql,
                    ([r.get(n) for n in names] + [batch, seq] for r in assets),
                )
                for r in assets:
                    impact(r["sha256"], {"relations"})
                    if r.get("post_id") is not None:
                        db.execute("INSERT OR IGNORE INTO work_posts(id) VALUES(?)", (r["post_id"],))
            if stop:
                stop("assets", seq)
        if next(db.execute("SELECT EXISTS(SELECT 1 FROM work_raw)"))[0]:
            source = pq.ParquetFile(directory / "source.parquet")
            schema = source.schema_arrow.serialize().to_pybytes()
            schema_id = digest(schema)
            with db:
                db.execute("INSERT OR IGNORE INTO source_schemas VALUES(?,?)", (schema_id, schema))
            offset = 0
            for block in source.iter_batches(batch_size=chunk_rows):
                raw_rows = []
                records = list(logical_rows(pa.Table.from_batches([block])))
                for row, observation_id, source_kind in db.execute(
                    "SELECT * FROM work_raw WHERE archive_row>=? AND archive_row<?",
                    (offset, offset + len(records)),
                ):
                    record = records[row - offset]
                    raw, fmt = source_record(record, source_kind)
                    raw_rows.append(
                        (
                            observation_id,
                            fmt,
                            schema_id,
                            *encode_raw(raw),
                        )
                    )
                offset += len(records)
                with db:
                    db.executemany(
                        "INSERT OR IGNORE INTO raw_metadata(observation_id,source_metadata_format,source_schema_id,raw_bytes,raw_sha256,raw_zlib) VALUES(?,?,?,?,?,?)",
                        raw_rows,
                    )
                if stop:
                    stop("raw", seq)
        after_post = -1
        while posts := list(
            db.execute(
                "SELECT id,fields FROM work_posts WHERE id>? ORDER BY id LIMIT ?", (after_post, chunk_rows)
            )
        ):
            with db:
                for post, fields in posts:
                    winner = list(
                        db.execute(
                            "SELECT row_id,md5,observation_id FROM observations WHERE post_id=? AND commit_seq<=? ORDER BY source_priority DESC,observed_at DESC,ingested_at DESC,observation_id DESC LIMIT 1",
                            (post, seq),
                        )
                    )
                    if not winner:
                        continue
                    oid, md5, observation_id = winner[0]
                    sql = "SELECT asset_id FROM assets WHERE post_id=? AND commit_seq<=?"
                    args = [post, seq]
                    if md5:
                        sql += " AND source_md5=?"
                        args.append(md5)
                    else:
                        sql += " AND observation_id=?"
                        args.append(observation_id)
                    match = list(db.execute(sql + " ORDER BY commit_seq DESC,asset_id DESC LIMIT 1", args))
                    asset = match[0][0] if match else None
                    old = list(
                        db.execute(
                            "SELECT row_id,asset_id FROM post_versions WHERE post_id=? AND valid_from<? AND (valid_until IS NULL OR valid_until>=?) ORDER BY valid_from DESC LIMIT 1",
                            (post, seq, seq),
                        )
                    )
                    if not old or old[0] != (oid, asset):
                        db.execute(
                            "UPDATE post_versions SET valid_until=? WHERE post_id=? AND valid_from<? AND valid_until IS NULL",
                            (seq, post, seq),
                        )
                        db.execute(
                            "INSERT OR REPLACE INTO post_versions VALUES(?,?,NULL,?,?)",
                            (post, seq, oid, asset),
                        )
                    for r in db.execute(
                        "SELECT DISTINCT sha256 FROM assets WHERE post_id=? AND commit_seq<=? AND sha256 IS NOT NULL",
                        (post, seq),
                    ):
                        impact(
                            r[0],
                            set(json.loads(fields))
                            | ({"relations"} if not old or old[0][1] != asset else set()),
                        )
            after_post = posts[-1][0]
            if stop:
                stop("post_versions", seq)
        after_sha = ""
        while changed := list(
            db.execute(
                "SELECT sha,fields FROM work_objects WHERE sha>? ORDER BY sha LIMIT ?",
                (after_sha, chunk_rows),
            )
        ):
            with db:
                for sha, fields in changed:
                    post = next(
                        db.execute(
                            "SELECT min(post_id) FROM assets WHERE sha256=? AND commit_seq<=?", (sha, seq)
                        )
                    )[0]
                    old = list(
                        db.execute(
                            "SELECT post_id FROM object_versions WHERE sha256=? AND valid_from<? AND (valid_until IS NULL OR valid_until>=?) ORDER BY valid_from DESC LIMIT 1",
                            (sha, seq, seq),
                        )
                    )
                    if not old or old[0][0] != post:
                        db.execute(
                            "UPDATE object_versions SET valid_until=? WHERE sha256=? AND valid_from<? AND valid_until IS NULL",
                            (seq, sha, seq),
                        )
                        db.execute(
                            "INSERT OR REPLACE INTO object_versions VALUES(?,?,NULL,?)", (sha, seq, post)
                        )
                    db.execute(
                        "INSERT OR REPLACE INTO changes VALUES(?,?,?)",
                        (seq, sha, ",".join(json.loads(fields))),
                    )
            after_sha = changed[-1][0]
            if stop:
                stop("object_versions", seq)
        if stop:
            stop("before_publish", seq)
        with db:
            current = settings(db)
            if int(current["served_seq"]) != seq - 1:
                raise IntegrityError("发布水位被其他写入改变")
            counts = {
                "objects": int(current["object_count"])
                + next(db.execute("SELECT count(*) FROM objects WHERE first_seq=?", (seq,)))[0],
                "observations": int(current["observation_count"])
                + next(db.execute("SELECT count(*) FROM observations WHERE commit_seq=?", (seq,)))[0],
                "assets": int(current["asset_count"])
                + next(db.execute("SELECT count(*) FROM assets WHERE commit_seq=?", (seq,)))[0],
            }
            db.execute(
                "INSERT INTO publications VALUES(?,?,?,?,?,?)",
                (seq, batch, now(), counts["objects"], counts["observations"], counts["assets"]),
            )
            set_state(
                db,
                served_seq=seq,
                object_count=counts["objects"],
                observation_count=counts["observations"],
                asset_count=counts["assets"],
            )
            db.execute("DELETE FROM pending_publication WHERE seq=?", (seq,))
        if stop:
            stop("published", seq)

    def collect_versions(self, *, retain_recent=64):
        with FileLock(self.index / ".online.lock"):
            self.check_location()
            db = connect(self.path)
            try:
                with db:
                    db.execute(
                        "DELETE FROM leases WHERE expires_ms IS NOT NULL AND expires_ms<?",
                        (int(time.time() * 1000),),
                    )
                    state = settings(db)
                    floor = max(int(state["base_seq"]), int(state["served_seq"]) - retain_recent)
                    lease = next(db.execute("SELECT min(seq) FROM leases"))[0]
                    if lease is not None:
                        floor = min(floor, lease)
                    floor = max(int(state["min_seq"]), floor)
                    # Advancing the floor and enforcing active leases is one write
                    # transaction; future readers cannot resurrect a collected view.
                    set_state(db, min_seq=floor)
                    for table, key in [("post_versions", "post_id"), ("object_versions", "sha256")]:
                        db.execute(
                            f"DELETE FROM {table} WHERE ({key},valid_from) IN (SELECT {key},valid_from FROM {table} WHERE valid_until<=? LIMIT 256)",
                            (floor,),
                        )
                    db.execute(
                        "DELETE FROM changes WHERE (seq,sha256) IN (SELECT seq,sha256 FROM changes WHERE seq<? LIMIT 256)",
                        (floor,),
                    )
                return floor
            finally:
                db.close()


def sync_library(lib):
    root = configured_root(lib.root)
    if root is not None:
        publisher = Publisher(lib.root, root)
        result = publisher.sync()
        publisher.collect_versions()
        return result
    return None
