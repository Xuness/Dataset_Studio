"""Resumable archive -> private online-v2 bootstrap, without a producer index.

The input is a fixed prefix of the immutable journal. Existing online databases,
their generations, leases and pointers are never replaced by this builder.
"""

from collections import OrderedDict
from contextlib import closing
import hashlib
import json
from pathlib import Path
import re
import sqlite3
import uuid

import pyarrow as pa
import pyarrow.parquet as pq

from .metadata import OBS_SCHEMA, ASSET_SCHEMA, NORMALIZATION_VERSION
from .online_schema import (
    VERSION,
    OBS_COLUMNS,
    ASSET_COLUMNS,
    OBJECT_COLUMNS,
    schema_sql,
    set_state,
    settings,
)
from .online_storage import MODULUS, connect, row_digest, scalar
from .raw_codec import encode as encode_raw, decode as decode_raw, source_record
from .archive_bindings import load as load_bindings, apply_to_preparation
from .util import (
    FileLock,
    IntegrityError,
    atomic_json,
    contained,
    digest,
    file_hash,
    logical_rows,
    now,
    read_json,
    safe_managed_path,
)

RAW_COLUMNS = ["observation_id", "source_metadata_format", "source_schema_id", "raw_bytes", "raw_sha256"]
CHECK_COLUMNS = {
    "objects": ["object_row", *OBJECT_COLUMNS, "first_seq"],
    "observations": OBS_COLUMNS,
    "assets": ["asset_row", *ASSET_COLUMNS],
    "raw_metadata": RAW_COLUMNS,
    "source_schemas": ["source_schema_id", "schema_ipc"],
    "tags": ["tag_id", "tag"],
}


def emit(event, **details):
    print(json.dumps({"at": now(), "event": event, **details}, ensure_ascii=False), flush=True)


def journal_snapshot(media, through=None):
    with closing(sqlite3.connect((media / "journal.sqlite").as_uri() + "?mode=ro", uri=True)) as journal:
        journal.execute("BEGIN")
        head = journal.execute("SELECT coalesce(max(seq),0) FROM commits").fetchone()[0]
        if through is None:
            through = head
        if not isinstance(through, int) or not 0 <= through <= head:
            raise IntegrityError("重建水位不在已归档范围内")
        hashed, previous, batch = hashlib.sha256(), 0, None
        for seq, batch, manifest in journal.execute(
            "SELECT seq,batch_id,manifest_json FROM commits WHERE seq<=? ORDER BY seq", (through,)
        ):
            if seq != previous + 1:
                raise IntegrityError("归档提交日志不连续")
            hashed.update(json.dumps([seq, batch, manifest], separators=(",", ":")).encode())
            previous = seq
        if previous != through:
            raise IntegrityError("归档提交日志缺少指定水位")
    return {"sequence": through, "journal_digest": hashed.hexdigest(), "head_batch": batch}


def checked_output(media, output):
    media, output = Path(media).resolve(), Path(output).resolve()
    if media == output or media in output.parents or output in media.parents:
        raise IntegrityError("重建目录必须独立于主库")
    if (output / "ONLINE.json").exists() or (output / "UPDATE-CONTROLLER.json").exists():
        raise IntegrityError("重建只能写入独立准备目录，不能覆盖已启用的在线库")
    if (media / "online-index.json").exists():
        linked = Path(read_json(media / "online-index.json")["index_root"]).resolve()
        if output == linked or linked in output.parents or output in linked.parents:
            raise IntegrityError("重建目录不能覆盖或包含现有在线目录")
    return media, output


def _quote(columns):
    return ",".join('"' + c + '"' for c in columns)


def reference_state(index):
    index = Path(index).resolve()
    pointer = read_json(index / "ONLINE.json")
    with closing(sqlite3.connect(contained(index, pointer["file"]).as_uri() + "?mode=ro", uri=True)) as db:
        state = settings(db)
    if state["generation"] != pointer["generation"] or state["library_id"] != pointer["library_id"]:
        raise IntegrityError("参考在线库身份不一致")
    return pointer, state


def comparison_lease(build, *, release=False):
    if not build.get("reference_index"):
        return
    index = Path(build["reference_index"])
    pointer, _ = reference_state(index)
    if pointer["generation"] != build["reference_generation"] or pointer["library_id"] != build["library_id"]:
        raise IntegrityError("参考在线库代次已改变")
    key, owner = "archive-validation:" + build["generation"], "archive-rebuild:" + build["library_id"]
    with FileLock(index / ".online.lock", timeout=10):
        if (index / "LAKE-RELOCATION.json").exists():
            raise IntegrityError("参考在线库正在迁移位置")
        db = connect(contained(index, pointer["file"]))
        try:
            with db:
                if release:
                    db.execute("DELETE FROM leases WHERE id=? AND owner=?", (key, owner))
                else:
                    state = settings(db)
                    if not int(state["min_seq"]) <= build["sequence"] <= int(state["served_seq"]):
                        raise IntegrityError("参考在线库已不保留所需重建快照")
                    db.execute(
                        "INSERT OR IGNORE INTO leases VALUES(?,?,NULL,?,'archive-validation')",
                        (key, build["sequence"], owner),
                    )
                    if next(db.execute("SELECT seq,owner FROM leases WHERE id=?", (key,))) != (
                        build["sequence"],
                        owner,
                    ):
                        raise IntegrityError("重建校验租约身份冲突")
        finally:
            db.close()


class Importer:
    def __init__(self, db, media, build, chunk_rows, stop):
        self.db, self.media, self.build = db, media, build
        self.chunk_rows, self.stop = chunk_rows, stop
        self.tags = OrderedDict()
        self.chunks = 0
        self.summaries = {
            name: (count, int(hashed, 16))
            for name, count, hashed in db.execute("SELECT name,rows,digest FROM build_progress")
            if name in CHECK_COLUMNS
        }
        db.execute("CREATE TEMP TABLE IF NOT EXISTS requested_tags(tag TEXT PRIMARY KEY) WITHOUT ROWID")

    def bump(self, name, rows):
        if not rows:
            return
        count, hashed = self.summaries.get(name, (0, 0))
        count += len(rows)
        hashed = (hashed + sum(row_digest(row) for row in rows)) % MODULUS
        self.summaries[name] = count, hashed
        self.db.execute(
            "INSERT OR REPLACE INTO build_progress VALUES(?,0,?,?,0)", (name, count, format(hashed, "x"))
        )

    def checkpoint(self, key, position, complete=False):
        self.db.execute(
            "INSERT OR REPLACE INTO build_progress VALUES(?,?,0,'0',?)", (key, position, int(complete))
        )

    def position(self, key):
        row = next(self.db.execute("SELECT position,complete FROM build_progress WHERE name=?", (key,)), None)
        return row if row else (0, 0)

    def intern_tags(self, rows):
        token_rows = [(row_id, sorted(set((text or "").split(" ")) - {""})) for row_id, text in rows]
        wanted = set(tag for _, tokens in token_rows for tag in tokens)
        missing = sorted(wanted - self.tags.keys())
        if missing:
            self.db.execute("DELETE FROM requested_tags")
            self.db.executemany("INSERT INTO requested_tags VALUES(?)", ((tag,) for tag in missing))
            added = list(
                self.db.execute(
                    "INSERT OR IGNORE INTO tags(tag) SELECT tag FROM requested_tags ORDER BY tag RETURNING tag_id,tag"
                )
            )
            self.bump("tags", added)
            self.tags.update(
                (tag, tid)
                for tag, tid in self.db.execute(
                    "SELECT r.tag,t.tag_id FROM requested_tags r JOIN tags t ON t.tag=r.tag"
                )
            )
        encoded = [
            (row_id, " ".join("t" + format(self.tags[tag], "x") for tag in tokens))
            for row_id, tokens in token_rows
        ]
        self.db.executemany("INSERT INTO tag_index(rowid,tokens) VALUES(?,?)", encoded)
        # A high-cardinality source cannot make this process retain an unbounded dictionary.
        for tag in wanted:
            self.tags.move_to_end(tag)
        while len(self.tags) > 250000:
            self.tags.popitem(last=False)

    def insert_rows(self, table, block, seq, batch):
        rows = block.to_pylist()
        stored, tags = [], []
        if table == "objects":
            columns = [*OBJECT_COLUMNS, "first_seq"]
            values = [
                (
                    r["sha256"],
                    f"segments/{batch}/images.tar",
                    r["offset"],
                    r["length"],
                    r.get("stored_ext"),
                    seq,
                )
                for r in rows
            ]
            identity = "object_row"
        else:
            names = OBS_SCHEMA.names if table == "observations" else ASSET_SCHEMA.names
            columns = [*names, "batch_id", "commit_seq"]
            values = [tuple(scalar(r.get(name), name) for name in names) + (batch, seq) for r in rows]
            identity = "row_id" if table == "observations" else "asset_row"
        sql = (
            f"INSERT OR IGNORE INTO {table}({_quote(columns)}) VALUES("
            + ",".join("?" for _ in columns)
            + f") RETURNING {identity}"
        )
        for row, value in zip(rows, values):
            inserted = next(self.db.execute(sql, value), None)
            if inserted is not None:
                stored.append((inserted[0], *value))
                if table == "observations":
                    tags.append((inserted[0], row.get("tag_string")))
        self.bump(table, stored)
        if tags:
            self.intern_tags(tags)

    def import_table(self, table, directory, seq, batch):
        key = f"archive:{seq}:{table}"
        position, complete = self.position(key)
        if complete:
            return
        offset = 0
        for block in pq.ParquetFile(directory / (table + ".parquet")).iter_batches(
            batch_size=self.chunk_rows
        ):
            end = offset + block.num_rows
            if end <= position:
                offset = end
                continue
            if position > offset:
                block = block.slice(position - offset)
            with self.db:
                self.insert_rows(table, block, seq, batch)
                self.checkpoint(key, end)
            self.chunks += 1
            if self.stop:
                self.stop(table, seq, self.chunks)
            offset = end
        with self.db:
            self.checkpoint(key, offset, True)

    def import_raw(self, directory, seq):
        key = f"archive:{seq}:raw"
        position, complete = self.position(key)
        if complete:
            return
        self.db.execute("DROP TABLE IF EXISTS temp.raw_requests")
        self.db.execute(
            "CREATE TEMP TABLE raw_requests(archive_row INTEGER,observation_id TEXT,"
            "source_kind TEXT,PRIMARY KEY(archive_row,observation_id)) WITHOUT ROWID"
        )
        envelopes = pq.ParquetFile(directory / "observations.parquet")
        for block in envelopes.iter_batches(
            batch_size=self.chunk_rows, columns=["archive_row", "observation_id", "source_kind"]
        ):
            rows = block.to_pylist()
            if any(r["archive_row"] is None or r["archive_row"] < 0 for r in rows):
                raise IntegrityError("原始元数据行号无效")
            self.db.executemany(
                "INSERT OR IGNORE INTO raw_requests VALUES(?,?,?)",
                ((r["archive_row"], r["observation_id"], r["source_kind"]) for r in rows),
            )
        count = next(self.db.execute("SELECT count(*) FROM raw_requests"))[0]
        if not count:
            with self.db:
                self.checkpoint(key, 0, True)
            return
        source = pq.ParquetFile(directory / "source.parquet")
        maximum = next(self.db.execute("SELECT max(archive_row) FROM raw_requests"))[0]
        if maximum >= source.metadata.num_rows:
            raise IntegrityError("原始元数据行号越界")
        schema = source.schema_arrow.serialize().to_pybytes()
        schema_id = digest(schema)
        with self.db:
            inserted = next(
                self.db.execute(
                    "INSERT OR IGNORE INTO source_schemas VALUES(?,?) RETURNING source_schema_id",
                    (schema_id, schema),
                ),
                None,
            )
            if inserted:
                self.bump("source_schemas", [(schema_id, schema)])
        offset = 0
        for block in source.iter_batches(batch_size=self.chunk_rows):
            end = offset + block.num_rows
            if end <= position:
                offset = end
                continue
            records = list(logical_rows(pa.Table.from_batches([block])))
            rows = []
            for archive_row, oid, kind in self.db.execute(
                "SELECT * FROM raw_requests WHERE archive_row>=? AND archive_row<? ORDER BY archive_row,observation_id",
                (max(position, offset), end),
            ):
                raw, fmt = source_record(records[archive_row - offset], kind)
                rows.append((oid, fmt, schema_id, *encode_raw(raw)))
            with self.db:
                stored = []
                for row in rows:
                    inserted = next(
                        self.db.execute(
                            "INSERT OR IGNORE INTO raw_metadata(observation_id,source_metadata_format,source_schema_id,"
                            "raw_bytes,raw_sha256,raw_zlib) VALUES(?,?,?,?,?,?) RETURNING raw_row",
                            row,
                        ),
                        None,
                    )
                    if inserted:
                        stored.append(row[:-1])
                self.bump("raw_metadata", stored)
                self.checkpoint(key, end)
            self.chunks += 1
            if self.stop:
                self.stop("raw", seq, self.chunks)
            offset = end
        with self.db:
            self.checkpoint(key, offset, True)

    def run(self):
        with closing(
            sqlite3.connect((self.media / "journal.sqlite").as_uri() + "?mode=ro", uri=True)
        ) as journal:
            for seq, batch, manifest_json in journal.execute(
                "SELECT seq,batch_id,manifest_json FROM commits WHERE seq<=? ORDER BY seq",
                (self.build["sequence"],),
            ):
                if self.position(f"archive:{seq}:complete")[1]:
                    continue
                manifest = json.loads(manifest_json)
                if (
                    manifest.get("library_id") != self.build["library_id"]
                    or manifest.get("batch_id") != batch
                ):
                    raise IntegrityError("封存批次身份无效")
                directory = contained(self.media, "segments/" + batch)
                evidence = []
                for name in ("objects.parquet", "observations.parquet", "assets.parquet", "source.parquet"):
                    info = manifest.get("files", {}).get(name)
                    if info is None and name == "source.parquet":
                        continue
                    path = directory / name
                    if (
                        info is None
                        or not path.is_file()
                        or path.stat().st_size != info["bytes"]
                        or file_hash(path) != info["sha256"]
                    ):
                        raise IntegrityError("封存元数据校验失败: " + str(path))
                    signature = path.stat()
                    evidence.append(
                        (f"input:{batch}:{name}", signature.st_mtime_ns, signature.st_size, info["sha256"])
                    )
                image = manifest.get("files", {}).get("images.tar")
                if image:
                    path = directory / "images.tar"
                    if not path.is_file() or path.stat().st_size != image["bytes"]:
                        raise IntegrityError("归档图片包缺失或长度改变")
                    signature = path.stat()
                    evidence.append(
                        (
                            f"image:{batch}:images.tar",
                            signature.st_mtime_ns,
                            signature.st_size,
                            image["sha256"],
                        )
                    )
                with self.db:
                    self.db.executemany("INSERT OR REPLACE INTO build_progress VALUES(?,?,?,?,1)", evidence)
                for table in ("objects", "observations", "assets"):
                    self.import_table(table, directory, seq, batch)
                self.import_raw(directory, seq)
                with self.db:
                    self.checkpoint(f"archive:{seq}:complete", seq, True)
                if seq == 1 or seq % 50 == 0 or seq == self.build["sequence"]:
                    emit(
                        "archive_batch_imported",
                        site=self.build["site"],
                        sequence=seq,
                        through=self.build["sequence"],
                        rows={k: v[0] for k, v in self.summaries.items()},
                    )


def _finish(db, build):
    emit("archive_secondary_indexes", site=build["site"])
    for sql in re.findall(r"CREATE (?:UNIQUE )?INDEX [a-z_]+ ON [^;]+;", schema_sql()):
        db.execute(sql.replace("INDEX ", "INDEX IF NOT EXISTS ", 1))
    seq = build["sequence"]
    if not next(
        db.execute("SELECT 1 FROM build_progress WHERE name='archive:versions' AND complete=1"), None
    ):
        emit("archive_current_versions", site=build["site"])
        with db:
            db.execute("DELETE FROM post_versions; DELETE FROM object_versions")
            db.execute(
                "INSERT INTO post_versions(post_id,valid_from,row_id,asset_id) "
                "SELECT o.post_id,?,o.row_id,CASE WHEN length(o.md5)>0 THEN "
                "(SELECT a.asset_id FROM assets a WHERE a.post_id=o.post_id AND a.source_md5=o.md5 "
                "ORDER BY a.commit_seq DESC,a.asset_id DESC LIMIT 1) ELSE "
                "(SELECT a.asset_id FROM assets a WHERE a.post_id=o.post_id AND a.observation_id=o.observation_id "
                "ORDER BY a.commit_seq DESC,a.asset_id DESC LIMIT 1) END "
                "FROM (SELECT DISTINCT post_id FROM observations WHERE post_id IS NOT NULL) p "
                "JOIN observations o ON o.row_id=(SELECT x.row_id FROM observations x WHERE x.post_id=p.post_id "
                "ORDER BY x.source_priority DESC,x.observed_at DESC,x.ingested_at DESC,x.observation_id DESC LIMIT 1)",
                (seq,),
            )
            db.execute(
                "INSERT INTO object_versions(sha256,valid_from,post_id) SELECT o.sha256,?,"
                "(SELECT min(a.post_id) FROM assets a WHERE a.sha256=o.sha256) FROM objects o",
                (seq,),
            )
            db.execute("INSERT OR REPLACE INTO build_progress VALUES('archive:versions',0,0,'0',1)")
    apply_to_preparation(db, Path(build["media_root"]), build)
    emit("archive_optimize", site=build["site"])
    db.execute("INSERT INTO tag_index(tag_index) VALUES('optimize'); ANALYZE")
    counts = {
        name: next(db.execute("SELECT count(*) FROM " + name))[0]
        for name in ("objects", "observations", "assets")
    }
    with db:
        db.execute(
            "INSERT OR REPLACE INTO publications VALUES(?,?,?,?,?,?)",
            (
                seq,
                build["head_batch"] or "empty-baseline",
                now(),
                counts["objects"],
                counts["observations"],
                counts["assets"],
            ),
        )
        set_state(
            db,
            served_seq=seq,
            object_count=counts["objects"],
            observation_count=counts["observations"],
            asset_count=counts["assets"],
        )
        for name in CHECK_COLUMNS:
            db.execute("INSERT OR IGNORE INTO build_progress VALUES(?,0,0,'0',1)", (name,))
            db.execute("UPDATE build_progress SET complete=1 WHERE name=?", (name,))
    db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    return counts


def build_archive(media, output, site, *, through=None, chunk_rows=1024, stop=None, reference_index=None):
    if read_json(Path(media) / "library.json").get("format_version") == 3:
        from .pinterest.lake.maintenance import build
        if site != "pinterest":
            raise IntegrityError("Archive source differs from the requested site")
        return build(media, output, through=through, chunk_rows=chunk_rows, stop=stop, reference_index=reference_index)
    if read_json(Path(media) / "library.json").get("format_version") == 2:
        from .media_lake.maintenance import build

        if site != "pixiv":
            raise IntegrityError("Archive source differs from the requested site")
        return build(media, output, through=through, chunk_rows=chunk_rows, stop=stop, reference_index=reference_index)
    media, output = checked_output(media, output)
    if site not in {"danbooru", "gelbooru", "yandere"} or not 1 <= chunk_rows <= 4096:
        raise ValueError("站点或重建批次无效")
    library = read_json(media / "library.json")
    if library.get("site") is not None and library["site"] != site:
        raise IntegrityError("站点与归档来源不一致")
    source_plan = media / "source_manifests" / "hf-conversion-plan.json"
    if source_plan.exists() and read_json(source_plan).get("site") != site:
        raise IntegrityError("站点与归档来源不一致")
    output.mkdir(parents=True, exist_ok=True)
    control = output / "ONLINE-BUILD.json"
    with FileLock(output / ".archive-build.lock", timeout=1):
        prior = read_json(control) if control.exists() else None
        if prior and through is not None and through != prior.get("sequence"):
            raise IntegrityError("不能改变已开始的重建水位")
        reference = None
        if reference_index is not None:
            reference_index = str(Path(reference_index).resolve())
            reference, reference_values = reference_state(reference_index)
            if reference["library_id"] != library["library_id"] or reference["site"] != site:
                raise IntegrityError("参考在线库与归档身份不一致")
            if prior and reference_index != prior.get("reference_index"):
                raise IntegrityError("不能改变重建参考位置")
            if through is None and prior is None:
                through = int(reference_values["served_seq"])
        snapshot = journal_snapshot(media, prior["sequence"] if prior else through)
        identity = {
            "library_id": library["library_id"],
            "site": site,
            "source": "canonical-archive-v1",
            "media_root": str(media),
            "normalization_version": NORMALIZATION_VERSION,
            **snapshot,
            "legacy_bindings_sha256": load_bindings(media, library["library_id"], site)[1],
        }
        if prior:
            if any(prior.get(k) != value for k, value in identity.items()):
                raise IntegrityError("归档或重建身份已变化，保留准备库")
            build = prior
            if build["state"] in {"built", "verified"}:
                if not contained(output, build["file"]).is_file():
                    raise IntegrityError("准备库文件缺失；请保留凭据并使用新的重建目录")
                return build
            if build["state"] != "building":
                raise IntegrityError("重建清单阶段无效")
        else:
            if any(p.name != ".archive-build.lock" for p in output.iterdir()):
                raise IntegrityError("新的重建目录必须为空")
            generation = "online-" + uuid.uuid4().hex
            build = {
                **identity,
                "schema_version": VERSION,
                "generation": generation,
                "file": f"online/{generation}.sqlite",
                "state": "building",
                "created_at": now(),
            }
            if reference is not None:
                build.update(reference_index=reference_index, reference_generation=reference["generation"])
            atomic_json(control, build)
        comparison_lease(build)
        (output / "online").mkdir(exist_ok=True)
        path = contained(output, build["file"])
        db = connect(path, building=True)
        try:
            if not next(db.execute("SELECT 1 FROM sqlite_master WHERE name='online_state'"), None):
                with db:
                    db.execute(schema_sql())
                    set_state(
                        db,
                        schema_version=VERSION,
                        library_id=build["library_id"],
                        site=site,
                        generation=build["generation"],
                        archive_seq=build["sequence"],
                        served_seq=0,
                        analysis_seq=0,
                        base_seq=build["sequence"],
                        min_seq=build["sequence"],
                        media_root=str(media),
                        source_index=str(output),
                        source_generation="archive",
                        source="canonical-archive-v1",
                        object_count=0,
                        observation_count=0,
                        asset_count=0,
                    )
            state = settings(db)
            if state["library_id"] != build["library_id"] or state["generation"] != build["generation"]:
                raise IntegrityError("准备库身份与重建清单不一致")
            # Keep identity constraints while delaying only secondary lookup indexes.
            for name in re.findall(r"CREATE INDEX ([a-z_]+) ON [^;]+;", schema_sql()):
                db.execute("DROP INDEX IF EXISTS " + name)
            Importer(db, media, build, chunk_rows, stop).run()
            if journal_snapshot(media, build["sequence"]) != snapshot:
                raise IntegrityError("归档提交前缀发生变化")
            counts = _finish(db, build)
            build.update(state="built", counts=counts, built_at=now())
            atomic_json(control, build)
        finally:
            db.close()
        return build


def verify_archive(output, *, maximum_raw_bytes=256 * 1024**2):
    output = Path(output).resolve()
    build = read_json(output / "ONLINE-BUILD.json")
    if build.get("source") == "canonical-archive-v3":
        from .pinterest.lake.maintenance import verify
        return verify(output, maximum_raw_bytes=maximum_raw_bytes)
    if build.get("source") == "canonical-archive-v2":
        from .media_lake.maintenance import verify

        return verify(output, maximum_raw_bytes=maximum_raw_bytes)
    if build.get("source") != "canonical-archive-v1" or build["state"] not in {"built", "verified"}:
        raise IntegrityError("归档准备库尚未构建完成")
    media = Path(build["media_root"])
    snapshot = journal_snapshot(media, build["sequence"])
    if any(snapshot[k] != build[k] for k in snapshot):
        raise IntegrityError("重建归档前缀发生变化")
    if load_bindings(media, build["library_id"], build["site"])[1] != build.get("legacy_bindings_sha256"):
        raise IntegrityError("历史关联归档在重建后发生变化")
    with FileLock(output / ".archive-build.lock", timeout=1):
        db = connect(contained(output, build["file"]), building=True)
        try:
            check = list(db.execute("PRAGMA quick_check"))
            if check != [("ok",)]:
                raise IntegrityError("在线数据库结构校验失败: " + str(check))
            db.execute("INSERT INTO tag_index(tag_index) VALUES('integrity-check')")
            tables = {}
            raw_statistics = {
                "records": 0,
                "raw_bytes": 0,
                "compressed_bytes": 0,
                "hash_sample_prefix": "00",
                "sample_records": 0,
                "sample_compressed_bytes": 0,
                "sample_duplicate_payload_bytes": 0,
                "sample_unique_payloads": 0,
                "sample_truncated": False,
            }
            sampled_payloads = set()
            for name, columns in CHECK_COLUMNS.items():
                expected = next(db.execute("SELECT rows,digest FROM build_progress WHERE name=?", (name,)))
                count = hashed = 0
                raw = name == "raw_metadata"
                for row in db.execute(
                    "SELECT " + _quote(columns + (["raw_zlib"] if raw else [])) + " FROM " + name
                ):
                    if raw:
                        decode_raw(row[-1], row[3], row[4], maximum_bytes=maximum_raw_bytes)
                        raw_statistics["records"] += 1
                        raw_statistics["raw_bytes"] += max(0, row[3])
                        raw_statistics["compressed_bytes"] += len(row[-1])
                        if row[4].startswith("00"):
                            raw_statistics["sample_records"] += 1
                            raw_statistics["sample_compressed_bytes"] += len(row[-1])
                            if row[4] in sampled_payloads:
                                raw_statistics["sample_duplicate_payload_bytes"] += len(row[-1])
                            elif len(sampled_payloads) < 200000:
                                sampled_payloads.add(row[4])
                            else:
                                raw_statistics["sample_truncated"] = True
                        row = row[:-1]
                    hashed = (hashed + row_digest(row)) % MODULUS
                    count += 1
                    if count % 250000 == 0:
                        emit("archive_verify_progress", site=build["site"], table=name, rows=count)
                actual = count, format(hashed, "x")
                if actual != expected:
                    raise IntegrityError("归档重建全行校验失败: " + name)
                tables[name] = {"rows": count, "digest": actual[1]}
                emit("archive_table_verified", site=build["site"], table=name, rows=count)
            relations = {
                "missing_object": "SELECT count(*) FROM assets a LEFT JOIN objects o ON o.sha256=a.sha256 WHERE a.sha256 IS NOT NULL AND o.sha256 IS NULL",
                "missing_origin": "SELECT count(*) FROM assets a LEFT JOIN observations o ON o.observation_id=a.observation_id WHERE a.observation_id IS NOT NULL AND o.observation_id IS NULL",
                "missing_current": "SELECT count(*) FROM post_versions p LEFT JOIN observations o ON o.row_id=p.row_id WHERE o.row_id IS NULL",
                "missing_raw": "SELECT count(*) FROM observations o LEFT JOIN raw_metadata r ON r.observation_id=o.observation_id WHERE r.observation_id IS NULL",
                "missing_schema": "SELECT count(*) FROM raw_metadata r LEFT JOIN source_schemas s ON s.source_schema_id=r.source_schema_id WHERE s.source_schema_id IS NULL",
            }
            failures = {key: next(db.execute(sql))[0] for key, sql in relations.items()}
            if any(failures.values()):
                raise IntegrityError("归档重建引用校验失败: " + str(failures))
            result = {
                "checked_at": now(),
                "source": build["source"],
                "sequence": build["sequence"],
                "journal_digest": build["journal_digest"],
                "tables": tables,
                "relations": failures,
                "raw_roundtrip_verified": True,
                "image_payloads_rehashed": False,
                "legacy_bindings_sha256": build.get("legacy_bindings_sha256"),
            }
            raw_statistics["sample_unique_payloads"] = len(sampled_payloads)
            result["raw_storage"] = raw_statistics
            db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
            path = contained(output, build["file"])
            wal = Path(str(path) + "-wal")
            if wal.exists() and wal.stat().st_size:
                raise IntegrityError("准备库仍有未合并的 WAL，等待其他连接退出后重新验证")
            signature = path.stat()
            result["file_signature"] = [signature.st_size, signature.st_mtime_ns]
            emit("archive_file_checksum", site=build["site"], bytes=signature.st_size)
            result["file_sha256"] = file_hash(path)
            atomic_json(output / "ONLINE-VERIFY.json", result)
            build.update(state="verified", verified_at=now())
            atomic_json(output / "ONLINE-BUILD.json", build)
            return result
        finally:
            db.close()


def compare_reference(output):
    """Compare stable identities and full metadata at the protected serving sequence."""
    output = Path(output).resolve()
    build = read_json(output / "ONLINE-BUILD.json")
    if build.get("source") == "canonical-archive-v3":
        from .pinterest.lake.maintenance import compare
        return compare(output)
    if build.get("source") == "canonical-archive-v2":
        from .media_lake.maintenance import compare

        return compare(output)
    if build["state"] != "verified" or not build.get("reference_index"):
        raise IntegrityError("对照前需要已验证的归档重建库和参考位置")
    comparison_lease(build)
    reference = Path(build["reference_index"])
    pointer, _ = reference_state(reference)
    sequence = build["sequence"]
    columns = {
        "objects": OBJECT_COLUMNS,
        "observations": [*OBS_SCHEMA.names, "batch_id", "commit_seq"],
        "assets": ASSET_COLUMNS,
        "raw_metadata": RAW_COLUMNS,
        "source_schemas": ["source_schema_id", "schema_ipc"],
        "current_posts": ["post_id", "observation_id", "asset_id"],
        "current_objects": ["sha256", "post_id"],
    }

    schema_ids = [set(), set()]

    def query(table, side):
        selected = _quote(columns[table])
        if table == "objects":
            return f"SELECT {selected} FROM objects WHERE first_seq<=?", (sequence,)
        if table in {"observations", "assets"}:
            return f"SELECT {selected} FROM {table} WHERE commit_seq<=?", (sequence,)
        if table == "raw_metadata":
            return (
                "SELECT r."
                + ",r.".join(columns[table])
                + ",r.raw_zlib FROM raw_metadata r JOIN observations o "
                "ON o.observation_id=r.observation_id WHERE o.commit_seq<=?"
            ), (sequence,)
        if table == "source_schemas":
            return (
                f"SELECT {selected} FROM source_schemas WHERE source_schema_id IN "
                "(SELECT value FROM json_each(?))"
            ), (json.dumps(sorted(schema_ids[side])),)
        if table == "current_posts":
            return (
                "SELECT p.post_id,o.observation_id,p.asset_id FROM post_versions p JOIN observations o "
                "ON o.row_id=p.row_id WHERE p.valid_from<=? AND (p.valid_until IS NULL OR p.valid_until>?)"
            ), (sequence, sequence)
        return (
            "SELECT sha256,post_id FROM object_versions WHERE valid_from<=? "
            "AND (valid_until IS NULL OR valid_until>?)"
        ), (sequence, sequence)

    proofs = {}
    paths = (contained(output, build["file"]), contained(reference, pointer["file"]))
    with (
        closing(sqlite3.connect(paths[0].as_uri() + "?mode=ro", uri=True)) as rebuilt,
        closing(sqlite3.connect(paths[1].as_uri() + "?mode=ro", uri=True)) as existing,
    ):
        for db in (rebuilt, existing):
            db.execute("PRAGMA query_only=ON; ")
            db.execute("PRAGMA cache_size=-131072")
        for table in columns:
            signatures = []
            for side, db in enumerate((rebuilt, existing)):
                sql, args = query(table, side)
                count = hashed = 0
                for row in db.execute(sql, args):
                    if table == "raw_metadata":
                        if row[2] is not None:
                            schema_ids[side].add(row[2])
                            if len(schema_ids[side]) > 100000:
                                raise IntegrityError("来源 schema 种类超过对照内存预算")
                        # The prepared copy has already passed a full round trip.
                        if db is existing:
                            decode_raw(row[-1], row[3], row[4], maximum_bytes=256 * 1024**2)
                        row = row[:-1]
                    hashed = (hashed + row_digest(row)) % MODULUS
                    count += 1
                    if count % 250000 == 0:
                        emit(
                            "archive_compare_progress",
                            site=build["site"],
                            table=table,
                            rows=count,
                            side="rebuilt" if db is rebuilt else "existing",
                        )
                signatures.append((count, format(hashed, "x")))
            proofs[table] = {
                "rebuilt": signatures[0],
                "existing": signatures[1],
                "equal": signatures[0] == signatures[1],
            }
            emit("archive_reference_compared", site=build["site"], table=table, **proofs[table])
    result = {
        "checked_at": now(),
        "sequence": sequence,
        "reference_generation": build["reference_generation"],
        "equal": all(p["equal"] for p in proofs.values()),
        "tables": proofs,
    }
    atomic_json(output / "ONLINE-COMPARE.json", result)
    if not result["equal"]:
        raise IntegrityError("归档重建与现用在线库不一致，保留准备库和快照租约")
    comparison_lease(build, release=True)
    return result


def activate_archive(media, output):
    """Activate a verified new location; never replace an existing library's serving pointer."""
    media, output = Path(media).resolve(), Path(output).resolve()
    if media == output or media in output.parents or output in media.parents:
        raise IntegrityError("重建目录必须独立于主库")
    build = read_json(output / "ONLINE-BUILD.json")
    if build.get("source") not in {"canonical-archive-v1", "canonical-archive-v2", "canonical-archive-v3"} or build["state"] not in {"verified", "active"}:
        raise IntegrityError("启用前必须完整验证归档重建结果")
    if read_json(media / "library.json")["library_id"] != build["library_id"]:
        raise IntegrityError("主库身份不匹配")
    with (
        FileLock(media / ".writer.lock"),
        FileLock(output / ".archive-build.lock"),
        FileLock(output / ".online.lock"),
    ):
        pointer = {key: build[key] for key in ("library_id", "generation", "file", "site", "schema_version")}
        if build.get("source") == "canonical-archive-v3":
            from .pinterest.lake.library import check_info
            check_info(read_json(media / "library.json"))
            pointer.update(schema_set=build["schema_set"], required_features=build["required_features"])
        linked = {"schema_version": build["schema_version"], "library_id": build["library_id"], "index_root": str(output)}
        for path, expected in ((output / "ONLINE.json", pointer), (media / "online-index.json", linked)):
            if path.exists() and read_json(path) != expected:
                raise IntegrityError("现有在线湖保持原代次；恢复切换需另行核对项目与版本引用")
        if build["state"] == "verified":
            path = contained(output, build["file"])
            wal = Path(str(path) + "-wal")
            proof = read_json(output / "ONLINE-VERIFY.json")
            if (wal.exists() and wal.stat().st_size) or file_hash(path) != proof.get("file_sha256"):
                raise IntegrityError("准备库在校验后发生变化，需重新验证")
        atomic_json(output / "cache_owner.json", {"library_id": build["library_id"], "root": str(media)})
        atomic_json(output / "ONLINE.json", pointer)
        atomic_json(media / "online-index.json", linked)
        build["state"] = "active"
        atomic_json(output / "ONLINE-BUILD.json", build)
        return pointer


def cleanup_preparation(output, evidence):
    """Remove only this tool's private database after preserving its small proof files."""
    output, evidence = Path(output).resolve(), Path(evidence).resolve()
    build = read_json(output / "ONLINE-BUILD.json")
    if build.get("source") not in {"canonical-archive-v1", "canonical-archive-v2", "canonical-archive-v3"} or build["state"] not in {
        "built",
        "verified",
        "cleaned",
    }:
        raise IntegrityError("只能回收本工具生成且未启用的准备库")
    checked_output(Path(build["media_root"]), output)
    if evidence == output or output in evidence.parents:
        raise IntegrityError("证据目录必须位于准备目录之外")
    with FileLock(output / ".archive-build.lock", timeout=1):
        receipt_path = evidence / "preparation-cleanup.json"
        if build["state"] == "cleaned":
            receipt = read_json(receipt_path)
            if receipt["generation"] != build["generation"]:
                raise IntegrityError("准备库回收凭据身份不一致")
            return receipt
        comparison_lease(build, release=True)
        evidence.mkdir(parents=True, exist_ok=True)
        for name in (
            "ONLINE-BUILD.json",
            "ONLINE-VERIFY.json",
            "ONLINE-COMPARE.json",
            "ONLINE-VERIFY.before-bindings.json",
            "ONLINE-COMPARE.before-bindings.json",
            "LEGACY-BINDINGS-ADOPTION.json",
        ):
            source = output / name
            if source.exists():
                value = read_json(source)
                target = evidence / name
                if target.exists() and read_json(target) != value:
                    raise IntegrityError("既有证据内容不同，禁止覆盖")
                atomic_json(target, value)
        path = safe_managed_path(output, contained(output, build["file"]))
        candidates = [path, Path(str(path) + "-wal"), Path(str(path) + "-shm")]
        receipt = {
            "generation": build["generation"],
            "output": str(output),
            "deleted_bytes": 0,
            "deleted_files": [],
            "at": now(),
        }
        if receipt_path.exists():
            receipt = read_json(receipt_path)
            if receipt["generation"] != build["generation"]:
                raise IntegrityError("准备库回收凭据身份不一致")
        for candidate in candidates:
            safe_managed_path(output, candidate)
            if candidate.exists():
                size = candidate.stat().st_size
                candidate.unlink()
                receipt["deleted_bytes"] += size
                receipt["deleted_files"].append(candidate.name)
                atomic_json(receipt_path, receipt)
        if path.parent.exists() and not any(path.parent.iterdir()):
            path.parent.rmdir()
        atomic_json(receipt_path, receipt)
        build.update(state="cleaned", cleaned_at=now(), evidence=str(evidence))
        atomic_json(output / "ONLINE-BUILD.json", build)
        return receipt


def main():
    import argparse

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "command",
        choices=("build", "verify", "compare", "release", "activate", "cleanup", "adopt-legacy-bindings"),
    )
    parser.add_argument("--media", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--site", choices=("danbooru", "gelbooru", "yandere", "pixiv", "pinterest"))
    parser.add_argument("--through", type=int)
    parser.add_argument("--chunk-rows", type=int, default=1024)
    parser.add_argument("--reference-index", type=Path)
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    if args.command in {"build", "activate"} and args.media is None:
        parser.error("此命令需要 --media")
    if args.command == "build" and args.site is None:
        parser.error("build 需要 --site")
    if args.command == "build":
        result = build_archive(
            args.media,
            args.output,
            args.site,
            through=args.through,
            chunk_rows=args.chunk_rows,
            reference_index=args.reference_index,
        )
    elif args.command == "verify":
        result = verify_archive(args.output)
    elif args.command == "compare":
        result = compare_reference(args.output)
    elif args.command == "release":
        comparison_lease(read_json(args.output / "ONLINE-BUILD.json"), release=True)
        result = {"released": True}
    elif args.command == "cleanup":
        if args.evidence is None:
            parser.error("cleanup 需要独立的 --evidence 目录")
        result = cleanup_preparation(args.output, args.evidence)
    elif args.command == "adopt-legacy-bindings":
        from .archive_bindings import adopt

        if read_json(args.output / "ONLINE-BUILD.json").get("source") != "canonical-archive-v1":
            raise IntegrityError("Legacy single-post bindings do not apply to canonical media archives")
        result = adopt(args.output)
    else:
        result = activate_archive(args.media, args.output)
    emit("complete", result=result)


if __name__ == "__main__":
    main()
