from contextlib import closing, contextmanager
import json
import sqlite3
import shutil
import stat
import time
import uuid

import duckdb

from .metadata import ASSET_SCHEMA, EVENT_SCHEMA, NORMALIZATION_VERSION, OBS_SCHEMA, TIME_FIELDS
from .tag_projection import (
    initialize_projection,
    refresh_history_view,
    require_projection,
    update_current,
    verify_current,
    version,
)
from .util import FileLock, IntegrityError, atomic_json, failpoint, now, read_json


def sql_type(field):
    return {"string": "VARCHAR", "int64": "BIGINT", "bool": "BOOLEAN"}[str(field.type)]


class Index:
    VERSION = 1

    def __init__(self, library):
        if (library.cache / "PRODUCER-RETIRED.json").exists():
            raise IntegrityError("此湖的生产者索引已退役；请使用 Studio 在线库或从归档重建，禁止自动恢复旧索引")
        self.lib = library
        self._session = None
        self._revision = 0

    def _emit(self, event, **details):
        callback = getattr(self.lib, "index_emit", None)
        if callback:
            callback({"event": event, **details})

    def directory(self):
        pointer = self.lib.cache / "CURRENT.json"
        if not pointer.exists():
            return None
        obj = read_json(pointer)
        if obj.get("library_id") != self.lib.info["library_id"] or obj.get("index_version") != self.VERSION:
            raise IntegrityError("索引版本/主库身份不匹配，请运行 rebuild-index")
        name = obj["generation"]
        if not name.startswith("gen-") or "/" in name or "\\" in name:
            raise IntegrityError("索引目录名无效")
        path = self.lib.cache / "indexes" / name
        if not (path / "catalog.sqlite").exists() or not (path / "analysis.duckdb").exists():
            raise IntegrityError("索引文件不完整，请运行 rebuild-index")
        return path

    def connect_duck(self, path, read_only=False):
        tmp = self.lib.cache / "temp"
        tmp.mkdir(exist_ok=True)
        con = duckdb.connect(str(path / "analysis.duckdb"), read_only=read_only)
        try:
            con.execute("SET threads=?", [self.lib.config.threads])
            con.execute("SET memory_limit=?", [self.lib.config.memory_limit])
            con.execute("SET temp_directory=?", [str(tmp)])
            con.execute("SET checkpoint_threshold=?", [self.lib.config.checkpoint_threshold])
            return con
        except BaseException:
            con.close()
            raise

    @contextmanager
    def _connections(self, path):
        if self._session is not None and self._session[0] == path:
            yield self._session[1:]
            return
        cat = sqlite3.connect(path / "catalog.sqlite", timeout=30)
        cat.row_factory = sqlite3.Row
        try:
            cat.execute("PRAGMA journal_mode=WAL")
            cat.execute("PRAGMA cache_size=-65536")
            cat.execute("PRAGMA temp_store=MEMORY")
            con = self.connect_duck(path)
            try:
                yield con, cat
            finally:
                started = time.monotonic()
                con.close()
                self._emit("index_connection_closed", seconds=time.monotonic() - started)
        finally:
            cat.close()

    @contextmanager
    def session(self, *, defer_tag_index=False):
        """One index writer for a migration, retaining per-batch durable commits."""
        if self._session is not None:
            raise RuntimeError("索引会话不能嵌套")
        with self.lib.cache_lock():
            path = self.directory()
            creating = path is None
            if creating:
                path = self._create()
            with self._connections(path) as (con, cat):
                self._session = path, con, cat
                try:
                    if defer_tag_index and self.lib.config.build_tags:
                        self._defer_tag_index(con, cat)
                    self._revision += self._apply(path, renormalize=creating)
                    if creating:
                        self._point_to(path)
                    yield self
                finally:
                    self._session = None

    @staticmethod
    def _defer_tag_index(con, cat):
        # Persist intent first: an interruption around DROP must remain resumable.
        # Tag rows and all unique/primary-key constraints remain in place.
        with cat:
            cat.execute("INSERT OR REPLACE INTO state VALUES ('tag_index_deferred',1)")
        con.execute("DROP INDEX IF EXISTS postings_tag")

    def _finish_tag_index(self, path):
        with self._connections(path) as (con, cat):
            pending = cat.execute("SELECT value FROM state WHERE key='tag_index_deferred'").fetchone()
            if not pending or not pending[0]:
                return False
            self._tag_index_policy(con, deferred=False)
            with cat:
                cat.execute("UPDATE state SET value=0 WHERE key='tag_index_deferred'")
            return True

    def _tag_index_policy(self, con, deferred=False):
        if self.lib.config.build_tags and self.lib.config.build_tag_index and not deferred:
            con.execute("CREATE INDEX IF NOT EXISTS postings_tag ON current_tag_postings(tag_id)")
        else:
            con.execute("DROP INDEX IF EXISTS postings_tag")

    def finish_deferred_indexes(self):
        if self._session is not None:
            raise RuntimeError("请先关闭批量索引会话，再构建标签加速索引")
        with self.lib.writer_lock(), self.lib.cache_lock():
            path = self._ensure_locked()
            return self._finish_tag_index(path)

    def _point_to(self, path):
        atomic_json(
            self.lib.cache / "CURRENT.json",
            {
                "library_id": self.lib.info["library_id"],
                "index_version": self.VERSION,
                "generation": path.name,
            },
        )

    def _create(self):
        path = self.lib.cache / "indexes" / ("gen-" + uuid.uuid4().hex)
        path.mkdir(parents=True)
        with sqlite3.connect(path / "catalog.sqlite") as db:
            db.executescript("""
              PRAGMA journal_mode=WAL;
              CREATE TABLE objects (sha256 TEXT PRIMARY KEY, pack_path TEXT NOT NULL, offset INTEGER NOT NULL,
                length INTEGER NOT NULL, stored_ext TEXT) WITHOUT ROWID;
              CREATE TABLE state (key TEXT PRIMARY KEY,value INTEGER);
              INSERT INTO state VALUES ('seq',0);
              INSERT INTO state VALUES ('observation_lookup_version',2);
              INSERT INTO state VALUES ('observation_lookup_seq',0);
              CREATE TABLE observation_lookup(
                observation_id TEXT PRIMARY KEY, post_id INTEGER, md5 TEXT, source_id TEXT, source_kind TEXT
              ) WITHOUT ROWID;
              CREATE INDEX lookup_source_post ON observation_lookup(source_id,post_id);
            """)
        db.close()
        con = self.connect_duck(path)
        try:
            con.execute("CREATE SEQUENCE observation_seq START 1")
            cols = ",".join(
                f'"{f.name}" '
                + ("TIMESTAMPTZ" if f.name in TIME_FIELDS + ["observed_at", "ingested_at"] else sql_type(f))
                for f in OBS_SCHEMA
            )
            con.execute(
                f"CREATE TABLE observations(row_id BIGINT DEFAULT nextval('observation_seq'), {cols}, "
                "batch_id VARCHAR, commit_seq BIGINT, PRIMARY KEY(observation_id))"
            )
            for name, schema in [("assets", ASSET_SCHEMA), ("events", EVENT_SCHEMA)]:
                cols = ",".join(f'"{f.name}" {sql_type(f)}' for f in schema)
                con.execute(f"CREATE TABLE {name}({cols}, batch_id VARCHAR, commit_seq BIGINT)")
            con.execute("CREATE UNIQUE INDEX assets_id ON assets(asset_id)")
            con.execute("CREATE INDEX observations_post ON observations(post_id)")
            con.execute("CREATE INDEX observations_row ON observations(row_id)")
            con.execute("CREATE INDEX assets_post ON assets(post_id)")
            con.execute(
                "CREATE TABLE objects(sha256 VARCHAR PRIMARY KEY, pack_path VARCHAR, "
                '"offset" BIGINT, length BIGINT, stored_ext VARCHAR)'
            )
            con.execute("CREATE TABLE applied(seq BIGINT PRIMARY KEY, batch_id VARCHAR)")
            con.execute(
                "CREATE TABLE raw_metadata(observation_id VARCHAR PRIMARY KEY, source_metadata_json VARCHAR, "
                "source_metadata_format VARCHAR, source_schema_id VARCHAR)"
            )
            con.execute("CREATE TABLE source_schemas(source_schema_id VARCHAR PRIMARY KEY, schema_ipc BLOB)")
            con.execute(
                "CREATE TABLE current_posts(post_id BIGINT PRIMARY KEY, row_id BIGINT, asset_id VARCHAR)"
            )
            con.execute("CREATE SEQUENCE tag_seq START 1")
            con.execute(
                "CREATE TABLE tags(tag_id BIGINT DEFAULT nextval('tag_seq'), tag VARCHAR PRIMARY KEY)"
            )
            initialize_projection(con, self.lib.config.build_tags)
            con.execute("""
              CREATE VIEW posts AS
              SELECT p.*, a.asset_id, a.sha256 AS blob_sha256, a.stored_ext, a.stored_bytes,
                     a.storage_profile, a.details_json AS storage_details_json, a.asset_id IS NOT NULL AS has_image
              FROM current_posts c JOIN observations p ON p.row_id=c.row_id
              LEFT JOIN assets a ON a.asset_id=c.asset_id
            """)
        finally:
            con.close()
        return path

    def _apply(self, path, renormalize=False):
        started = time.monotonic()
        with self._connections(path) as (con, cat):
            require_projection(con, self.lib.config.build_tags)
            deferred = cat.execute("SELECT value FROM state WHERE key='tag_index_deferred'").fetchone()
            self._tag_index_policy(con, deferred=bool(deferred and deferred[0]))
            seq = cat.execute("SELECT value FROM state WHERE key='seq'").fetchone()[0]
            duck_seq = con.execute("SELECT coalesce(max(seq),0) FROM applied").fetchone()[0]
            if duck_seq < seq:
                raise IntegrityError("SSD 索引提交序号不一致，请运行 rebuild-index")
            self._ensure_catalog_lookup(con, cat, seq)
            pending = self.lib.commits(seq)
            if pending:
                self._emit("index_sync_started", batches=len(pending), after_seq=seq)
            commit_seconds = 0.0
            tag_changes = 0
            for position, commit in enumerate(pending, 1):
                n, batch = commit["seq"], commit["batch_id"]
                p = self.lib.root / "segments" / batch
                applied = con.execute("SELECT batch_id FROM applied WHERE seq=?", [n]).fetchone()
                if applied and applied[0] != batch:
                    raise IntegrityError("SSD 索引提交身份与主库不同")
                if not con.execute("SELECT 1 FROM applied WHERE seq=?", [n]).fetchone():
                    reparse = renormalize or (
                        json.loads(commit["manifest_json"]).get("normalization_version")
                        != NORMALIZATION_VERSION
                    )
                    con.execute("BEGIN TRANSACTION")
                    try:
                        old_row = con.execute("SELECT coalesce(max(row_id),0) FROM observations").fetchone()[
                            0
                        ]
                        for name, schema in [
                            ("observations", OBS_SCHEMA),
                            ("assets", ASSET_SCHEMA),
                            ("events", EVENT_SCHEMA),
                        ]:
                            names = ",".join('"' + f.name + '"' for f in schema)
                            conflict = " ON CONFLICT DO NOTHING" if name != "events" else ""
                            if name == "observations" and reparse:
                                con.register("reparsed_observations", reparse_observations(p))
                                try:
                                    con.execute(
                                        f"INSERT INTO observations({names},batch_id,commit_seq) "
                                        f"SELECT {names},?,? FROM reparsed_observations ON CONFLICT DO NOTHING",
                                        [batch, n],
                                    )
                                finally:
                                    con.unregister("reparsed_observations")
                                continue
                            con.execute(
                                f"INSERT INTO {name}({names},batch_id,commit_seq) "
                                f"SELECT {names},?,? FROM read_parquet(?)" + conflict,
                                [batch, n, str(p / f"{name}.parquet")],
                            )
                        con.execute(
                            'INSERT INTO objects SELECT sha256,?,"offset",length,stored_ext '
                            "FROM read_parquet(?) ON CONFLICT DO NOTHING",
                            [f"segments/{batch}/images.tar", str(p / "objects.parquet")],
                        )
                        index_raw_metadata(con, p)
                        if self.lib.config.build_tags:
                            con.execute(
                                "CREATE OR REPLACE TEMP TABLE incoming_tags AS "
                                "SELECT DISTINCT row_id,unnest(string_split(tag_string,' ')) tag "
                                "FROM observations WHERE row_id>?",
                                [old_row],
                            )
                            con.execute(
                                "INSERT INTO tags(tag) SELECT DISTINCT tag FROM incoming_tags "
                                "WHERE tag<>'' ON CONFLICT DO NOTHING"
                            )
                            con.execute(
                                "UPDATE tag_projection_meta SET historical_count=historical_count+"
                                "(SELECT count(*) FROM incoming_tags WHERE tag<>'')"
                            )
                        self._refresh_current(con, old_row, n)
                        if self.lib.config.build_tags:
                            tag_result = update_current(con)
                            tag_changes += tag_result["changed_posts"]
                        con.execute("INSERT INTO applied VALUES (?,?)", [n, batch])
                        commit_started = time.monotonic()
                        con.execute("COMMIT")
                        commit_seconds += time.monotonic() - commit_started
                    except BaseException as error:
                        # COMMIT may have succeeded when an interrupt is surfaced
                        # by the driver. Preserve that original error for recovery.
                        try:
                            con.execute("ROLLBACK")
                        except Exception as rollback_error:
                            if "no transaction is active" not in str(rollback_error):
                                error.add_note(f"索引回滚也失败: {rollback_error}")
                        raise
                failpoint("after_duckdb_index")
                import pyarrow.parquet as pq

                rows = pq.read_table(p / "objects.parquet").to_pylist()
                observations = con.execute(
                    "SELECT observation_id,post_id,md5,source_kind FROM observations WHERE commit_seq=?", [n]
                ).fetchall()
                source_id = json.loads(commit["manifest_json"]).get("source", {}).get("source_id")
                with cat:
                    cat.executemany(
                        "INSERT OR IGNORE INTO observation_lookup VALUES (?,?,?,?,?)",
                        ((oid, post_id, md5, source_id, kind) for oid, post_id, md5, kind in observations),
                    )
                    cat.executemany(
                        "INSERT OR IGNORE INTO objects VALUES (?,?,?,?,?)",
                        [
                            (
                                r["sha256"],
                                f"segments/{batch}/images.tar",
                                r["offset"],
                                r["length"],
                                r["stored_ext"],
                            )
                            for r in rows
                        ],
                    )
                    cat.execute("UPDATE state SET value=? WHERE key='seq'", (n,))
                    cat.execute("UPDATE state SET value=? WHERE key='observation_lookup_seq'", (n,))
                failpoint("after_index")
                if position == 1 or position % 25 == 0 or position == len(pending):
                    self._emit(
                        "index_sync_progress",
                        batches=position,
                        total_batches=len(pending),
                        seq=n,
                        changed_tag_posts=tag_changes,
                        commit_seconds=commit_seconds,
                        seconds=time.monotonic() - started,
                    )
            if pending:
                self._emit(
                    "index_sync_finished",
                    batches=len(pending),
                    changed_tag_posts=tag_changes,
                    commit_seconds=commit_seconds,
                    seconds=time.monotonic() - started,
                )
            if (self.lib.root / "online-index.json").exists() and self.directory() == path:
                from .online import Publisher, configured_root

                sequence = cat.execute("SELECT value FROM state WHERE key='seq'").fetchone()[0]
                Publisher(self.lib.root, configured_root(self.lib.root)).mark_analysis(sequence, path.name)
            return len(pending)

    def upgrade_tag_projection(self):
        """Upgrade an isolated SSD generation, validate it, then atomically publish its pointer."""
        with (
            FileLock(self.lib.cache / ".daily-run.lock", timeout=1),
            self.lib.writer_lock(),
            self.lib.cache_lock(),
        ):
            previous = self.directory()
            if previous is None:
                return {"status": "completed", "index_directory": str(self._ensure_locked())}
            marker = self.lib.cache / "indexes" / "tag-projection-upgrade.json"
            with self._connections(previous) as (con, _):
                if version(con) is not None:
                    require_projection(con, self.lib.config.build_tags)
                    refresh_history_view(con)
                    self._tag_index_policy(con)
                    result = {"status": "already_current", "index_directory": str(previous)}
                    if marker.exists():
                        saved = read_json(marker)
                        if saved.get("target_generation") == previous.name:
                            saved.update(phase="completed")
                            atomic_json(marker, saved)
                    return result
                con.execute("CHECKPOINT")
            signature = {
                name: [p.stat().st_size, p.stat().st_mtime_ns]
                for name in ("analysis.duckdb", "catalog.sqlite")
                if (p := previous / name).exists()
            }
            saved = read_json(marker) if marker.exists() else None
            if saved and saved.get("phase") != "completed":
                if (
                    saved.get("library_id") != self.lib.info["library_id"]
                    or saved.get("source_generation") != previous.name
                    or saved.get("source_signature") != signature
                    or saved.get("tags_enabled") != self.lib.config.build_tags
                ):
                    raise IntegrityError("未完成的标签升级与当前来源/配置不一致，保留现场等待检查")
                name = saved.get("target_generation", "")
                if not name.startswith("gen-") or "/" in name or "\\" in name:
                    raise IntegrityError("标签升级目标目录无效")
                target = self.lib.cache / "indexes" / name
                if (
                    target.resolve().parent != (self.lib.cache / "indexes").resolve()
                    or target.resolve() == previous.resolve()
                ):
                    raise IntegrityError("标签升级目标路径越界")
            else:
                target = self.lib.cache / "indexes" / ("gen-" + uuid.uuid4().hex)
                target.mkdir()
                saved = {
                    "library_id": self.lib.info["library_id"],
                    "source_generation": previous.name,
                    "source_signature": signature,
                    "target_generation": target.name,
                    "tags_enabled": self.lib.config.build_tags,
                    "phase": "copying",
                    "at": now(),
                }
                atomic_json(marker, saved)
            for owned in (
                target,
                target / "analysis.duckdb",
                target / "catalog.sqlite",
                target / "catalog-upgrade.sqlite.tmp",
            ):
                if owned.is_symlink() or (
                    owned.exists()
                    and getattr(owned.lstat(), "st_file_attributes", 0)
                    & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0)
                ):
                    raise IntegrityError("标签升级目标包含链接路径，保留现场等待检查")
            started = time.monotonic()
            self._emit("tag_projection_upgrade_started", source=str(previous), destination=str(target))
            if saved["phase"] == "copying":
                shutil.copy2(previous / "analysis.duckdb", target / "analysis.duckdb")
                temporary_catalog = target / "catalog-upgrade.sqlite.tmp"
                temporary_catalog.unlink(missing_ok=True)
                with closing(
                    sqlite3.connect((previous / "catalog.sqlite").as_uri() + "?mode=ro", uri=True)
                ) as source:
                    with closing(sqlite3.connect(temporary_catalog)) as destination:
                        source.backup(destination)
                temporary_catalog.replace(target / "catalog.sqlite")
                saved["phase"] = "initializing"
                atomic_json(marker, saved)
                failpoint("after_tag_projection_copy")
            with self._connections(target) as (con, cat):
                if version(con) is None:
                    con.execute("BEGIN")
                    try:
                        initialize_projection(con, self.lib.config.build_tags, self._emit)
                        con.execute("COMMIT")
                    except BaseException as error:
                        try:
                            con.execute("ROLLBACK")
                        except Exception as rollback_error:
                            if "no transaction is active" not in str(rollback_error):
                                error.add_note(f"标签升级回滚也失败: {rollback_error}")
                        raise
                saved["phase"] = "verifying"
                atomic_json(marker, saved)
                failpoint("after_tag_projection_commit")
                refresh_history_view(con)
                self._session = (target, con, cat)
                try:
                    self._apply(target)
                    self._finish_tag_index(target)
                    self._emit("tag_projection_verification_started")
                    checked = verify_current(con, self.lib.config.build_tags, self._emit)
                    self._emit("tag_projection_verification_finished", **checked)
                finally:
                    self._session = None
            saved["phase"] = "ready"
            atomic_json(marker, saved)
            failpoint("before_tag_projection_pointer")
            self._point_to(target)
            failpoint("after_tag_projection_pointer")
            result = {
                "status": "completed",
                "index_directory": str(target),
                "previous_index_directory": str(previous),
                "verification": checked,
                "seconds": time.monotonic() - started,
            }
            saved.update(phase="completed", result=result)
            atomic_json(marker, saved)
            self._emit("tag_projection_upgrade_finished", **result)
            return result

    def _ensure_catalog_lookup(self, con, cat, seq):
        version = cat.execute("SELECT value FROM state WHERE key='observation_lookup_version'").fetchone()
        covered = 0
        if version is not None:
            if version[0] not in {1, 2}:
                raise IntegrityError("观察记录定位索引版本不兼容，请运行 rebuild-index")
            state = (
                cat.execute("SELECT value FROM state WHERE key='observation_lookup_seq'").fetchone()
                if version[0] == 2
                else None
            )
            if state is not None:
                covered = state[0]
                if covered == seq:
                    return
                if not 0 <= covered < seq:
                    raise IntegrityError("观察记录定位索引提交序号不一致，请运行 rebuild-index")
        # Upgrade only this rebuildable SSD cache. The old archive/plan identities
        # stay unchanged, including recovery where DuckDB is ahead of SQLite.
        with cat:
            if not cat.in_transaction:
                cat.execute("BEGIN")
            if version is not None and version[0] == 1:
                cat.execute("DROP TABLE IF EXISTS observation_lookup")
            cat.execute(
                "CREATE TABLE IF NOT EXISTS observation_lookup("
                "observation_id TEXT PRIMARY KEY,post_id INTEGER,md5 TEXT,source_id TEXT,source_kind TEXT) WITHOUT ROWID"
            )
            cat.execute(
                "CREATE INDEX IF NOT EXISTS lookup_source_post ON observation_lookup(source_id,post_id)"
            )
            if not covered:
                cat.execute("DELETE FROM observation_lookup")
            sources = {
                c["seq"]: json.loads(c["manifest_json"]).get("source", {}).get("source_id")
                for c in self.lib.commits(covered)
                if c["seq"] <= seq
            }
            cursor = con.execute(
                "SELECT observation_id,post_id,md5,source_kind,commit_seq FROM observations "
                "WHERE commit_seq>? AND commit_seq<=?",
                [covered, seq],
            )
            while rows := cursor.fetchmany(8192):
                cat.executemany(
                    "INSERT OR REPLACE INTO observation_lookup VALUES (?,?,?,?,?)",
                    ((oid, post_id, md5, sources[n], kind) for oid, post_id, md5, kind, n in rows),
                )
            cat.execute("INSERT OR REPLACE INTO state VALUES ('observation_lookup_version',2)")
            cat.execute("INSERT OR REPLACE INTO state VALUES ('observation_lookup_seq',?)", (seq,))

    @staticmethod
    def _refresh_current(con, old_row, seq):
        con.execute(
            "CREATE OR REPLACE TEMP TABLE affected AS "
            "SELECT DISTINCT post_id FROM observations WHERE row_id>? AND post_id IS NOT NULL "
            "UNION SELECT post_id FROM assets WHERE commit_seq=? AND post_id IS NOT NULL",
            [old_row, seq],
        )
        # Only new observations and the previous winners compete; no full history window sort.
        con.execute(
            """
          CREATE OR REPLACE TEMP TABLE winners AS
          SELECT * FROM (
            SELECT p.row_id,p.post_id,p.md5,p.source_priority,p.observed_at,p.ingested_at,p.observation_id
                FROM observations p JOIN current_posts c ON p.row_id=c.row_id
                JOIN affected x ON c.post_id=x.post_id
            UNION ALL
            SELECT row_id,post_id,md5,source_priority,observed_at,ingested_at,observation_id
                FROM observations WHERE row_id>? AND post_id IS NOT NULL
          ) QUALIFY row_number() OVER (PARTITION BY post_id ORDER BY
              source_priority DESC,try_cast(observed_at AS TIMESTAMPTZ) DESC NULLS LAST,
              ingested_at DESC,observation_id DESC)=1
        """,
            [old_row],
        )
        # Splitting the two matching rules permits equality hash joins. Combining
        # them with OR makes DuckDB use a blockwise nested loop over all assets.
        con.execute("""
          INSERT OR REPLACE INTO current_posts
          SELECT post_id,row_id,asset_id FROM (
            SELECT p.post_id,p.row_id,a.asset_id,a.commit_seq FROM winners p LEFT JOIN assets a
              ON p.post_id=a.post_id AND a.source_md5=p.md5
              WHERE p.md5 IS NOT NULL AND p.md5<>''
            UNION ALL
            SELECT p.post_id,p.row_id,a.asset_id,a.commit_seq FROM winners p LEFT JOIN assets a
              ON p.post_id=a.post_id AND p.observation_id=a.observation_id WHERE p.md5 IS NULL OR p.md5=''
          ) matched QUALIFY row_number() OVER (PARTITION BY post_id ORDER BY
              commit_seq DESC NULLS LAST,asset_id DESC NULLS LAST)=1
        """)

    def _ensure_locked(self):
        path = self.directory()
        if path is None:
            path = self._create()
            self._revision += self._apply(path, renormalize=True)
            self._point_to(path)
        else:
            self._revision += self._apply(path)
        return path

    def sync(self):
        if self._session is not None:
            path = self._session[0]
            self._revision += self._apply(path)
            return path
        with self.lib.cache_lock():
            return self._ensure_locked()

    def rebuild(self):
        with self.lib.writer_lock(), self.lib.cache_lock():
            path = self._create()
            if self.lib.config.build_tags:
                with self._connections(path) as (con, cat):
                    self._defer_tag_index(con, cat)
            self._apply(path, renormalize=True)
            self._finish_tag_index(path)
            failpoint("before_index_pointer")
            atomic_json(
                self.lib.cache / "CURRENT.json",
                {
                    "library_id": self.lib.info["library_id"],
                    "index_version": self.VERSION,
                    "generation": path.name,
                },
            )
            return str(path)

    @contextmanager
    def read(self):
        if self._session is not None:
            self.sync()
            yield self._session[1]
            return
        with self.lib.cache_lock():
            path = self._ensure_locked()
            con = self.connect_duck(path, read_only=True)
            try:
                yield con
            finally:
                con.close()

    @contextmanager
    def object_lookup(self):
        path = self.sync()
        if self._session is not None:
            yield CatalogLookup(self, self._session[2])
            return
        con = sqlite3.connect((path / "catalog.sqlite").as_uri() + "?mode=ro", uri=True)
        con.row_factory = sqlite3.Row
        try:
            con.execute("PRAGMA temp_store=MEMORY")
            yield CatalogLookup(self, con)
        finally:
            con.close()

    def stats(self):
        with self.read() as con:
            counts = {
                name: con.execute(f"SELECT count(*) FROM {name}").fetchone()[0]
                for name in [
                    "observations",
                    "posts",
                    "assets",
                    "objects",
                    "raw_metadata",
                    "tags",
                    "current_tag_state",
                    "current_tag_postings",
                ]
            }
            counts["tag_postings"] = con.execute(
                "SELECT historical_count FROM tag_projection_meta"
            ).fetchone()[0]
            return counts

    def query(self, sql, limit=100):
        with self.read() as con:
            statements = con.extract_statements(sql)
            if len(statements) != 1 or statements[0].type.name != "SELECT":
                raise ValueError("query 只接受一条只读 SELECT")
            cur = con.execute(f"SELECT * FROM ({sql.rstrip().rstrip(';')}) AS result LIMIT ?", [limit])
            names = [d[0] for d in cur.description]
            return [dict(zip(names, row)) for row in cur.fetchall()]


class CatalogLookup:
    """Bounded request tables keep SQLite point lookups outermost on SSD."""

    def __init__(self, index, connection):
        self.index, self.connection = index, connection
        self.revision = index._revision
        self.cached = {}

    def _refresh(self):
        if self.revision != self.index._revision:
            self.cached.clear()
            self.revision = self.index._revision

    def __call__(self, sha):
        self._refresh()
        if sha in self.cached:
            return self.cached[sha]
        return self.connection.execute("SELECT * FROM objects WHERE sha256=?", (sha,)).fetchone()

    def prefetch(self, hashes):
        self._refresh()
        values = list(dict.fromkeys(h.lower() for h in hashes if isinstance(h, str)))[:4096]
        self.cached = dict.fromkeys(values)
        with self.connection as con:
            con.execute(
                "CREATE TEMP TABLE IF NOT EXISTS requested_objects(sha256 TEXT PRIMARY KEY) WITHOUT ROWID"
            )
            con.execute("DELETE FROM requested_objects")
            con.executemany("INSERT INTO requested_objects VALUES (?)", ((v,) for v in values))
            for row in con.execute(
                "SELECT o.* FROM requested_objects r CROSS JOIN objects o ON o.sha256=r.sha256"
            ):
                self.cached[row["sha256"]] = row

    def observations(self, identities):
        values = list(dict.fromkeys(identities))
        output = {}
        with self.connection as con:
            con.execute(
                "CREATE TEMP TABLE IF NOT EXISTS requested_observations("
                "observation_id TEXT PRIMARY KEY) WITHOUT ROWID"
            )
            for start in range(0, len(values), 4096):
                con.execute("DELETE FROM requested_observations")
                con.executemany(
                    "INSERT INTO requested_observations VALUES (?)",
                    ((v,) for v in values[start : start + 4096]),
                )
                for row in con.execute(
                    "SELECT o.* FROM requested_observations r CROSS JOIN observation_lookup o "
                    "ON o.observation_id=r.observation_id"
                ):
                    output[row["observation_id"]] = dict(row)
        return output

    def observations_for_posts(self, source_id, post_ids):
        """Resolve missing old row locators only within this exact input source."""
        if not source_id:
            raise ValueError("按帖子匹配必须限定来源")
        values = list(dict.fromkeys(pid for pid in post_ids if pid is not None))
        output = {}
        with self.connection as con:
            con.execute("CREATE TEMP TABLE IF NOT EXISTS requested_posts(post_id INTEGER PRIMARY KEY)")
            for start in range(0, len(values), 4096):
                con.execute("DELETE FROM requested_posts")
                con.executemany(
                    "INSERT INTO requested_posts VALUES (?)", ((v,) for v in values[start : start + 4096])
                )
                for row in con.execute(
                    "SELECT o.* FROM requested_posts r CROSS JOIN observation_lookup o INDEXED BY lookup_source_post "
                    "ON o.post_id=r.post_id WHERE o.source_id=? AND o.source_kind='legacy_parquet'",
                    (source_id,),
                ):
                    if row["post_id"] in output:
                        raise IntegrityError(f"同一 V2 来源的帖子元数据匹配不唯一: {row['post_id']}")
                    output[row["post_id"]] = dict(row)
        return output


def original_record(lib, observation, cache=None):
    """Return a complete source row/object. Small bounded cache is owned by the caller."""
    import pyarrow.parquet as pq

    batch = observation["batch_id"]
    if cache is not None and batch in cache:
        table = cache[batch]
    else:
        table = pq.ParquetFile(lib.root / "segments" / batch / "source.parquet").read()
        if cache is not None:
            if len(cache) >= 2:
                cache.clear()
            cache[batch] = table
    from .util import arrow_row

    row = arrow_row(table, observation["archive_row"])
    if observation["source_kind"] in {"api_json", "api_yandere_v1", "api_gelbooru_v1"}:
        return json.loads(row["post_json"]), row["post_json"], "api-json-original-object/v1"
    from .util import json_text, typed_value

    return row, json_text(typed_value(row)), "arrow-row-typed-json/v1"


def reparse_observations(directory):
    """Rebuild derived business fields from the preserved source, retaining envelope identities/times."""
    import pyarrow as pa
    import pyarrow.parquet as pq
    from .metadata import normalize, normalization_columns, normalization_rows
    from .hf_metadata import HF_KINDS, normalize_hf
    from .api_metadata import API_KINDS, normalize_api
    from decimal import Decimal

    envelopes = pq.ParquetFile(directory / "observations.parquet").read().to_pylist()
    if not envelopes:
        return pa.Table.from_pylist([], schema=OBS_SCHEMA)
    source_file = pq.ParquetFile(directory / "source.parquet")
    kind = envelopes[0]["source_kind"]
    columns = (
        None
        if kind in {*API_KINDS, "orphan_index", *HF_KINDS}
        else normalization_columns(source_file.schema_arrow)
    )
    source_table = source_file.read(columns=columns)
    source = (
        source_table.to_pylist()
        if kind in {*API_KINDS, "orphan_index", *HF_KINDS}
        else normalization_rows(source_table)
    )
    output = []
    for envelope in envelopes:
        raw = source[envelope["archive_row"]]
        kind = envelope["source_kind"]
        context = {}
        if kind in API_KINDS:
            context["tag_types"] = json.loads(raw.get("tag_types_json") or "null")
            raw = json.loads(raw["post_json"], parse_float=Decimal)
        elif kind == "orphan_index":
            raw = json.loads(raw["legacy_index_json"])
        normalizer = normalize_hf if kind in HF_KINDS else normalize_api if kind in API_KINDS else normalize
        record = normalizer(
            raw,
            envelope["source_key"],
            envelope["source_row"],
            kind,
            envelope["archive_row"],
            envelope["observed_at"],
            envelope["time_quality"],
            **context,
        )
        for name in [
            "observation_id",
            "ingested_at",
            "publication_kind",
            "publication_group",
            "source_priority",
        ]:
            if name in envelope:
                record[name] = envelope[name]
        output.append(record)
    return pa.Table.from_pylist(output, schema=OBS_SCHEMA)


def index_raw_metadata(con, directory):
    import pyarrow as pa
    import pyarrow.parquet as pq
    from .util import digest, logical_rows
    from .api_metadata import API_KINDS

    env = pq.ParquetFile(directory / "observations.parquet").read(
        columns=["observation_id", "archive_row", "source_kind"]
    )
    if not env.num_rows:
        return
    source = pq.ParquetFile(directory / "source.parquet").read()
    schema_bytes = source.schema.serialize().to_pybytes()
    schema_id = digest(schema_bytes)
    con.execute("INSERT INTO source_schemas VALUES (?,?) ON CONFLICT DO NOTHING", [schema_id, schema_bytes])
    rows = []
    if env["archive_row"].null_count:
        raise IntegrityError("原始元数据行号为空")
    selected = source.take(env["archive_row"])
    for e, record in zip(env.to_pylist(), logical_rows(selected)):
        if e["source_kind"] in API_KINDS:
            raw, fmt = record["post_json"], "api-json-original-object/v1"
        else:
            raw = json.dumps(record, ensure_ascii=False, allow_nan=False, separators=(",", ":"))
            fmt = "arrow-row-typed-json/v1"
        rows.append(
            {
                "observation_id": e["observation_id"],
                "source_metadata_json": raw,
                "source_metadata_format": fmt,
                "source_schema_id": schema_id,
            }
        )
    con.register("raw_metadata_input", pa.Table.from_pylist(rows))
    try:
        con.execute("INSERT INTO raw_metadata SELECT * FROM raw_metadata_input ON CONFLICT DO NOTHING")
    finally:
        con.unregister("raw_metadata_input")
