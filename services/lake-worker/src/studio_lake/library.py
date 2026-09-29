from contextlib import contextmanager
from pathlib import Path
import hashlib
import io
import json
import os
import re
import shutil
import sqlite3
import tarfile
import uuid

import pyarrow as pa
import pyarrow.parquet as pq

from . import __version__
from .metadata import ASSET_SCHEMA, EVENT_SCHEMA, NORMALIZATION_VERSION, OBJECT_SCHEMA, OBS_SCHEMA
from .util import (
    FileLock,
    IntegrityError,
    atomic_json,
    contained,
    digest,
    failpoint,
    file_hash,
    now,
    read_json,
    rows_fingerprint,
    sync_directory,
    sync_file,
    same_directory,
)

JOURNAL_SQL = """
CREATE TABLE IF NOT EXISTS commits (
 seq INTEGER PRIMARY KEY AUTOINCREMENT, batch_id TEXT UNIQUE NOT NULL,
 dedupe_key TEXT UNIQUE NOT NULL, manifest_json TEXT NOT NULL, committed_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS progress (
 source_key TEXT PRIMARY KEY, next_row INTEGER NOT NULL, complete INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS releases (
 release_id TEXT PRIMARY KEY, state TEXT NOT NULL, definition_json TEXT NOT NULL,
 remote_commit TEXT, verification_json TEXT
);
"""


class Library:
    def __init__(self, config):
        self.config, self.root, self.cache = config, config.root, config.cache
        marker = self.root / "library.json"
        if not marker.exists():
            raise RuntimeError("主库尚未初始化，请先运行 init")
        self.info = read_json(marker)
        if self.info.get("format_version") != 1:
            raise IntegrityError("不支持的主库版本")
        owner = self.cache / "cache_owner.json"
        if owner.exists() and read_json(owner).get("library_id") != self.info["library_id"]:
            raise IntegrityError("SSD 目录已属于另一个主库")
        self.cache.mkdir(parents=True, exist_ok=True)
        if not owner.exists():
            atomic_json(owner, {"library_id": self.info["library_id"], "root": str(self.root)})
        if not (self.root / "journal.sqlite").exists():
            raise IntegrityError("主库提交日志缺失；禁止把已有数据视为一个空库")

    @classmethod
    def initialize(cls, config):
        if (config.cache / "cache_owner.json").exists() and not (config.root / "library.json").exists():
            raise IntegrityError("SSD 目录已属于另一个主库")
        config.root.mkdir(parents=True, exist_ok=True)
        with FileLock(config.root / ".writer.lock"):
            if not (config.root / "library.json").exists():
                allowed = {".writer.lock", "journal.sqlite", "journal.sqlite-wal", "journal.sqlite-shm"}
                if any(p.name not in allowed for p in config.root.iterdir()):
                    raise RuntimeError("初始化目标非空；请使用空目录，避免覆盖已有数据")
                with sqlite3.connect(config.root / "journal.sqlite") as db:
                    db.executescript(JOURNAL_SQL)
                db.close()
                sync_file(config.root / "journal.sqlite")
                atomic_json(
                    config.root / "library.json",
                    {
                        "format_version": 1,
                        "library_id": str(uuid.uuid4()),
                        "created_at": now(),
                        "created_by": __version__,
                        "image_format": "uncompressed-pax-tar",
                        "source_policy": "preserve-all-fields-and-original-api-response-bodies",
                    },
                )
        lib = cls(config)
        for name in ["segments", "staging", "plans", "releases", "publish", "source_manifests"]:
            (lib.root / name).mkdir(exist_ok=True)
        return lib

    @contextmanager
    def writer_lock(self):
        from contextlib import nullcontext

        marker = self.cache / "UPDATE-CONTROLLER.json"
        admission = nullcontext()
        if marker.exists():
            from .updates.io import device_lock

            owner = read_json(marker)
            if owner.get("library_id") != self.info["library_id"]:
                raise IntegrityError("更新控制器身份不匹配")
            admission = device_lock(Path(owner["root"]), self.root)
        with admission, FileLock(self.root / ".writer.lock"):
            self.check_location()
            yield

    def check_location(self):
        if (self.cache / "LAKE-RELOCATION.json").exists():
            raise IntegrityError("数据湖位置迁移尚未完成，禁止写入旧位置")
        owner = self.cache / "cache_owner.json"
        if owner.exists() and not same_directory(read_json(owner).get("root"), self.root):
            raise IntegrityError("数据湖媒体位置已迁移，禁止写入旧位置")

    @contextmanager
    def cache_lock(self):
        with FileLock(self.cache / ".index.lock"):
            self.check_location()
            yield

    @contextmanager
    def journal(self):
        from .sqlite_control import Connection

        db = Connection(self.root / "journal.sqlite", timeout=30)
        db.execute("PRAGMA journal_mode=WAL")
        db.execute("PRAGMA synchronous=FULL")
        db.execute("PRAGMA foreign_keys=ON")
        try:
            yield db
        finally:
            db.close()

    def setting(self, key, default=None):
        with self.journal() as db:
            row = db.execute("SELECT value FROM settings WHERE key=?", (key,)).fetchone()
        return json.loads(row[0]) if row else default

    def set_setting(self, key, value):
        with self.journal() as db, db:
            db.execute("INSERT OR REPLACE INTO settings VALUES (?,?)", (key, json.dumps(value)))

    def commits(self, after=0):
        with self.journal() as db:
            return [dict(r) for r in db.execute("SELECT * FROM commits WHERE seq>? ORDER BY seq", (after,))]

    def committed_key(self, key):
        with self.journal() as db:
            row = db.execute("SELECT seq,batch_id FROM commits WHERE dedupe_key=?", (key,)).fetchone()
        return dict(row) if row else None

    def progress(self, key):
        with self.journal() as db:
            row = db.execute("SELECT next_row,complete FROM progress WHERE source_key=?", (key,)).fetchone()
        return (row[0], bool(row[1])) if row else (0, False)

    def accept_manifest(self, directory: Path, manifest, verify=True):
        if manifest["library_id"] != self.info["library_id"]:
            raise IntegrityError("批次来自不同主库")
        if verify:
            verify_files(directory, manifest)
        final = self.root / "segments" / manifest["batch_id"]
        if directory != final:
            if final.exists():
                raise IntegrityError(f"目标批次目录已存在: {final}")
            directory.rename(final)
            sync_directory(final.parent)
        failpoint("after_rename")
        with self.journal() as db, db:
            old = db.execute("SELECT * FROM commits WHERE dedupe_key=?", (manifest["dedupe_key"],)).fetchone()
            if old:
                if old["batch_id"] != manifest["batch_id"]:
                    raise IntegrityError("重复批次键对应不同文件，保留文件等待核查")
                self.sync_online()
                return old["seq"]
            db.execute(
                "INSERT INTO commits(batch_id,dedupe_key,manifest_json,committed_at) VALUES (?,?,?,?)",
                (manifest["batch_id"], manifest["dedupe_key"], json.dumps(manifest), now()),
            )
            seq = db.execute("SELECT last_insert_rowid()").fetchone()[0]
            p = manifest.get("progress")
            if p:
                prev = db.execute(
                    "SELECT next_row FROM progress WHERE source_key=?", (p["source_key"],)
                ).fetchone()
                if (prev[0] if prev else 0) != p["start_row"]:
                    raise IntegrityError("源文件恢复位置不连续，拒绝提交")
                db.execute(
                    "INSERT OR REPLACE INTO progress VALUES (?,?,?)",
                    (p["source_key"], p["next_row"], int(p["complete"])),
                )
            for key, value in manifest.get("settings", {}).items():
                db.execute("INSERT OR REPLACE INTO settings VALUES (?,?)", (key, json.dumps(value)))
            if manifest.get("source", {}).get("ingest_run_id"):
                from .daily_state import record_ingest_commit

                record_ingest_commit(db, manifest, seq)
            if manifest.get("source", {}).get("update_job_id"):
                from .updates.archive import record_update_commit

                record_update_commit(db, manifest, seq)
        failpoint("after_commit")
        self.sync_online()
        return seq

    def sync_online(self):
        if (self.root / "online-index.json").exists():
            from .online import sync_library

            return sync_library(self)
        return None

    def recover(self, deep=False):
        with self.writer_lock():
            committed = {r["batch_id"] for r in self.commits()}
            output = []
            candidates = list((self.root / "segments").iterdir()) + list((self.root / "staging").iterdir())
            for p in sorted(candidates):
                if not p.is_dir() or p.name in committed:
                    continue
                if not (p / "manifest.json").exists():
                    intent = read_json(p / "intent.json") if (p / "intent.json").exists() else {}
                    if intent.get("source", {}).get("kind") in {"api", "update_api"} and (p / "response_body.bin").exists():
                        if file_hash(p / "response_body.bin") != intent["source"].get("response_sha256"):
                            output.append(
                                {"batch": p.name, "status": "incomplete-response-retained", "path": str(p)}
                            )
                            continue
                        if intent["source"]["kind"] == "update_api":
                            from .updates.archive import resume_response

                            resume_response(self, p)
                        else:
                            from .ingest import finish_saved_response

                            finish_saved_response(self, p)
                    else:
                        output.append({"batch": p.name, "status": "incomplete-retained", "path": str(p)})
                        continue
                m = read_json(p / "manifest.json")
                seq = self.accept_manifest(p, m)
                output.append({"batch": p.name, "status": "committed", "seq": seq})
            if deep:
                for c in self.commits():
                    verify_files(self.root / "segments" / c["batch_id"], json.loads(c["manifest_json"]))
        return output

    def verify(self, deep=False):
        result = {"batches": 0, "objects": 0, "observations": 0, "bytes": 0}
        for c in self.commits():
            m = json.loads(c["manifest_json"])
            p = self.root / "segments" / c["batch_id"]
            verify_files(p, m, hash_contents=deep)
            result["batches"] += 1
            result["observations"] += m["counts"]["observations"]
            for row in pq.read_table(p / "objects.parquet").to_pylist():
                result["objects"] += 1
                result["bytes"] += row["length"]
                if deep:
                    data = read_object(
                        self.root, f"segments/{c['batch_id']}/images.tar", row["offset"], row["length"]
                    )
                    if digest(data) != row["sha256"]:
                        raise IntegrityError(f"图片偏移或内容校验失败: {row['sha256']}")
        result["mode"] = "content-hashes-and-object-offsets" if deep else "file-presence-and-size"
        return result


def verify_files(directory, manifest, hash_contents=True):
    for name, info in manifest["files"].items():
        p = contained(directory, name)
        if not p.is_file() or p.stat().st_size != info["bytes"]:
            raise IntegrityError(f"批次文件缺失或长度错误: {p}")
        if hash_contents and file_hash(p) != info["sha256"]:
            raise IntegrityError(f"批次文件内容校验失败: {p}")


def read_object(root, relative_path, offset, length):
    if offset < 0 or length < 0:
        raise IntegrityError("无效图片范围")
    with contained(root, relative_path).open("rb") as f:
        f.seek(offset)
        data = f.read(length)
    if len(data) != length:
        raise IntegrityError("图片读取长度不符")
    return data


class HashingWriter:
    def __init__(self, handle):
        self.handle, self.hash = handle, hashlib.sha256()

    def write(self, data):
        written = self.handle.write(data)
        if written != len(data):
            raise OSError("short write")
        self.hash.update(data)
        return written

    def __getattr__(self, key):
        return getattr(self.handle, key)


class Batch:
    """Caller holds the library writer lock. No source files are moved or removed."""

    def __init__(self, lib, key, source=None):
        if lib.committed_key(key):
            raise RuntimeError("此批次已提交")
        self.lib, self.key = lib, key
        self.id = uuid.uuid4().hex
        self.path = lib.root / "staging" / self.id
        self.path.mkdir(parents=True)
        self.source = source or {}
        self.objects, self.observations, self.assets, self.events = [], [], [], []
        self.new_hashes = set()
        self.payload_bytes = 0
        self.tar, self.pack_writer = None, None
        self.pack_staging_path = None
        self.metadata_staging_path = contained(lib.cache / "metadata_staging", self.id)
        self.source_tables = []
        self.source_json = []
        self.source_json_column = None
        self.source_rows = 0
        self.schemas = {}
        atomic_json(
            self.path / "intent.json",
            {
                "library_id": lib.info["library_id"],
                "dedupe_key": key,
                "source": self.source,
                "batch_id": self.id,
                "created_at": now(),
            },
        )

    def add_source(self, table):
        self._flush_source_json()
        if self.source_tables and not table.schema.equals(self.source_tables[0].schema, check_metadata=True):
            raise IntegrityError("一个来源分片内的 schema 不一致")
        self.source_tables.append(table)
        self.source_rows += table.num_rows

    def add_source_json(self, text, column="legacy_index_json"):
        """Buffer source evidence while reserving its stable archive row now."""
        if self.source_json_column not in {None, column}:
            raise IntegrityError("一个来源分片内的 schema 不一致")
        if self.source_json_column is None:
            schema = pa.schema([(column, pa.string())])
            if self.source_tables and not self.source_tables[0].schema.equals(schema, check_metadata=True):
                raise IntegrityError("一个来源分片内的 schema 不一致")
            self.source_json_column = column
        row = self.source_rows
        self.source_json.append(text)
        self.source_rows += 1
        if len(self.source_json) >= 4096:
            self._flush_source_json()
        return row

    def _flush_source_json(self):
        if self.source_json:
            self.source_tables.append(
                pa.Table.from_arrays(
                    [pa.array(self.source_json, type=pa.string())], names=[self.source_json_column]
                )
            )
            self.source_json.clear()

    def write_bytes(self, name, data):
        """Persist recovery evidence on the main disk immediately."""
        p = contained(self.path, name)
        p.parent.mkdir(parents=True, exist_ok=True)
        with p.open("xb") as f:
            f.write(data)
            f.flush()
            os.fsync(f.fileno())

    def metadata_path(self, name):
        """Place regenerable batch files on SSD until the grouped publication phase."""
        p = contained(self.metadata_staging_path, name)
        p.parent.mkdir(parents=True, exist_ok=True)
        return p

    def stage_bytes(self, name, data):
        with self.metadata_path(name).open("xb") as f:
            f.write(data)

    def add_blob(self, data, ext, lookup, expected_hash=None):
        sha = digest(data)
        if expected_hash and sha != expected_hash.lower():
            raise IntegrityError(f"来源图片 SHA-256 不符: expected={expected_hash}, actual={sha}")
        ext = str(ext or "bin").lower().lstrip(".")
        if not re.fullmatch(r"[a-z0-9]{1,12}", ext):
            ext = "bin"
        if sha in self.new_hashes:
            return sha, ext
        old = lookup(sha)
        if old:
            existing = read_object(self.lib.root, old["pack_path"], old["offset"], old["length"])
            if len(existing) != len(data) or digest(existing) != sha:
                raise IntegrityError("已有重复对象损坏，拒绝丢弃本次有效来源")
            return sha, ext
        if self.tar is None:
            scratch = self.lib.cache / "pack_staging"
            scratch.mkdir(exist_ok=True)
            self.pack_staging_path = scratch / (self.id + ".tar.tmp")
            self.pack_writer = HashingWriter(self.pack_staging_path.open("xb", buffering=8 * 1024**2))
            self.tar = tarfile.open(
                fileobj=self.pack_writer, mode="w", format=tarfile.PAX_FORMAT, copybufsize=1024**2
            )
        info = tarfile.TarInfo(f"{sha}.{ext}")
        info.size, info.mtime, info.mode = len(data), 0, 0o644
        offset = self.pack_writer.tell() + len(info.tobuf(format=tarfile.PAX_FORMAT))
        reader = pa.BufferReader(data) if isinstance(data, pa.Buffer) else io.BytesIO(data)
        with reader:
            self.tar.addfile(info, reader)
        self.objects.append(
            dict(sha256=sha, member_name=info.name, offset=offset, length=len(data), stored_ext=ext)
        )
        self.new_hashes.add(sha)
        self.payload_bytes += len(data)
        return sha, ext

    def seal(self, progress=None, settings=None, *, move_staged=False, verify_objects=False):
        self._flush_source_json()
        staged = {}
        if self.tar:
            self.tar.close()
            self.pack_writer.flush()
            self.pack_writer.close()
            staged["images.tar"] = (
                self.pack_staging_path,
                {"bytes": self.pack_staging_path.stat().st_size, "sha256": self.pack_writer.hash.hexdigest()},
            )
        if self.source_tables:
            table = pa.concat_tables(self.source_tables).combine_chunks()
            expected = rows_fingerprint(table)
            source_path = self.metadata_path("source.parquet")
            pq.write_table(
                table,
                source_path,
                compression="zstd",
                version="2.6",
                store_schema=True,
                row_group_size=65536,
            )
            restored = pq.ParquetFile(source_path).read()
            if rows_fingerprint(restored) != expected:
                raise IntegrityError("原始元数据写后核对失败")
        for name, rows, schema in [
            ("observations", self.observations, OBS_SCHEMA),
            ("assets", self.assets, ASSET_SCHEMA),
            ("objects", self.objects, OBJECT_SCHEMA),
            ("events", self.events, EVENT_SCHEMA),
        ]:
            pq.write_table(
                pa.Table.from_pylist(rows, schema=schema),
                self.metadata_path(f"{name}.parquet"),
                compression="zstd",
                version="2.6",
            )
        for p in sorted(self.metadata_staging_path.rglob("*")):
            if p.is_file():
                staged[p.relative_to(self.metadata_staging_path).as_posix()] = (
                    p,
                    {"bytes": p.stat().st_size, "sha256": file_hash(p)},
                )
        failpoint("after_metadata_prepared")

        # Finish every HDD write before starting destination content verification.
        # API recovery may overwrite derived files left by an interrupted copy.
        for name, (source_path, _) in staged.items():
            target = contained(self.path, name)
            target.parent.mkdir(parents=True, exist_ok=True)
            if move_staged and source_path.stat().st_dev == target.parent.stat().st_dev:
                # Opt-in for a prepared library on the same SSD. Flush before rename;
                # the normal destination hash and durable manifest gates still apply.
                sync_file(source_path)
                os.replace(source_path, target)
                sync_directory(target.parent)
            else:
                with source_path.open("rb") as src, target.open("wb") as dst:
                    shutil.copyfileobj(src, dst, 8 * 1024**2)
                    dst.flush()
                    os.fsync(dst.fileno())
        paths = sorted(p for p in self.path.rglob("*") if p.is_file() and p.name != "manifest.json")
        for p in paths:
            if p.relative_to(self.path).as_posix() not in staged:
                sync_file(p)
        failpoint("after_batch_files_copied")
        files = {}
        for p in paths:
            name = p.relative_to(self.path).as_posix()
            if verify_objects and name == "images.tar":
                from .sequential_io import verify_pack

                info = staged[name][1]
                verify_pack(p, self.objects, info)
            else:
                info = {"bytes": p.stat().st_size, "sha256": file_hash(p)}
            if name in staged and info != staged[name][1]:
                raise IntegrityError(f"批次文件写后校验失败: {p}")
            files[name] = info
        m = {
            "format_version": 1,
            "library_id": self.lib.info["library_id"],
            "batch_id": self.id,
            "dedupe_key": self.key,
            "created_at": now(),
            "writer_version": __version__,
            "normalization_version": NORMALIZATION_VERSION,
            "source": self.source,
            "progress": progress,
            "settings": settings or {},
            "files": files,
            "counts": {
                "observations": len(self.observations),
                "objects": len(self.objects),
                "assets": len(self.assets),
                "events": len(self.events),
                "source_rows": self.source_rows,
            },
        }
        atomic_json(self.path / "manifest.json", m)
        sync_directory(self.path)
        self._clear_staging()
        failpoint("after_seal")
        return m

    def _clear_staging(self):
        # A durable, verified HDD manifest must exist before either SSD buffer is removed.
        metadata = self.metadata_staging_path.resolve()
        expected_parent = (self.lib.cache / "metadata_staging").resolve()
        if metadata.parent != expected_parent or metadata.name != self.id:
            raise IntegrityError("元数据暂存清理路径越界")
        if metadata.exists():
            shutil.rmtree(metadata)
        if self.pack_staging_path is not None:
            pack = self.pack_staging_path.resolve()
            if (
                pack.parent != (self.lib.cache / "pack_staging").resolve()
                or pack.name != self.id + ".tar.tmp"
            ):
                raise IntegrityError("图片包暂存清理路径越界")
            pack.unlink(missing_ok=True)

    def commit(self, **kwargs):
        return self.lib.accept_manifest(self.path, self.seal(**kwargs), verify=False)

    @classmethod
    def resume_metadata(cls, lib, path):
        intent = read_json(path / "intent.json")
        if intent["library_id"] != lib.info["library_id"] or intent["batch_id"] != path.name:
            raise IntegrityError("暂存批次身份不符")
        self = cls.__new__(cls)
        self.lib, self.key, self.id, self.path = lib, intent["dedupe_key"], path.name, path
        self.source = intent["source"]
        self.objects, self.observations, self.assets, self.events = [], [], [], []
        self.new_hashes, self.source_tables, self.schemas = set(), [], {}
        self.source_json, self.source_json_column = [], None
        self.payload_bytes = self.source_rows = 0
        self.tar = self.pack_writer = None
        self.pack_staging_path = None
        self.metadata_staging_path = contained(lib.cache / "metadata_staging", self.id)
        return self
