"""Resumable native-index -> serving SQLite conversion; never reads image bytes."""

from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import json
import re
import time
import uuid

import apsw
import duckdb

from .online_schema import (
    VERSION,
    OBS_COLUMNS,
    ASSET_COLUMNS,
    OBJECT_COLUMNS,
    schema_sql,
    settings,
    set_state,
)
from .util import FileLock, IntegrityError, atomic_json, read_json, now, contained
from .online_storage import MODULUS, connect, scalar, row_digest
from .raw_codec import encode as encode_raw


def pack_raw(row):
    position, oid, raw, fmt, schema = row
    return (
        position + 1,
        oid,
        fmt,
        schema,
        *encode_raw(raw),
    )


def emit(event, **fields):
    print(json.dumps({"time": now(), "event": event, **fields}, ensure_ascii=False), flush=True)


def migrate(media, source_index, output, site, *, chunk_rows=16384, stop_after_chunks=None, activate=False):
    media, source_index, output = map(lambda p: Path(p).resolve(), (media, source_index, output))
    manifest = media / "source_manifests/hf-conversion-plan.json"
    if (
        site not in {"danbooru", "yandere", "gelbooru"}
        or (manifest.exists() and read_json(manifest).get("site") != site)
        or (site != "danbooru" and not manifest.exists())
    ):
        raise IntegrityError("来源站点与湖清单不一致")
    library = read_json(media / "library.json")
    pointer = read_json(source_index / "CURRENT.json")
    if pointer["library_id"] != library["library_id"] or pointer.get("index_version") != 1:
        raise IntegrityError("源目录与主库身份不匹配")
    output.mkdir(parents=True, exist_ok=True)
    online = output / "online"
    online.mkdir(exist_ok=True)
    control = output / "ONLINE-BUILD.json"
    with FileLock(source_index / ".index.lock"), FileLock(output / ".online.lock"):
        pointer = read_json(source_index / "CURRENT.json")
        native = duckdb.connect(
            str(source_index / "indexes" / pointer["generation"] / "analysis.duckdb"), read_only=True
        )
        native.execute("SET threads=1; SET memory_limit='4GB'")
        native.execute("BEGIN TRANSACTION")
        sequence = native.execute("SELECT coalesce(max(seq),0) FROM applied").fetchone()[0]
        if control.exists():
            build = read_json(control)
            if any(
                build.get(k) != v
                for k, v in {
                    "library_id": library["library_id"],
                    "source_generation": pointer["generation"],
                    "sequence": sequence,
                    "site": site,
                }.items()
            ):
                raise IntegrityError("迁移基线已变化；保留已有准备库，不允许错接新的源版本")
        else:
            if (output / "ONLINE.json").exists():
                raise IntegrityError("已有在线湖；请使用 online-sync，而不是覆盖重建")
            build = {
                "schema_version": VERSION,
                "library_id": library["library_id"],
                "source_generation": pointer["generation"],
                "sequence": sequence,
                "site": site,
                "generation": "online-" + uuid.uuid4().hex,
                "state": "building",
            }
            build["file"] = "online/" + build["generation"] + ".sqlite"
            atomic_json(control, build)
        path = output / build["file"]
        db = connect(path, building=True)
        if not list(db.execute("SELECT name FROM sqlite_master WHERE name='online_state'")):
            db.execute(schema_sql())
            with db:
                set_state(
                    db,
                    schema_version=VERSION,
                    library_id=library["library_id"],
                    site=site,
                    generation=build["generation"],
                    archive_seq=sequence,
                    served_seq=0,
                    analysis_seq=sequence,
                    base_seq=sequence,
                    min_seq=sequence,
                    media_root=str(media),
                    source_index=str(source_index),
                    source_generation=pointer["generation"],
                )
        state = settings(db)
        if state["library_id"] != library["library_id"] or state["generation"] != build["generation"]:
            raise IntegrityError("准备库身份与迁移清单不一致")
        indexes = re.findall(r"CREATE (?:UNIQUE )?INDEX ([a-z_]+) ON [^;]+;", schema_sql())
        index_sql = re.findall(r"CREATE (?:UNIQUE )?INDEX [a-z_]+ ON [^;]+;", schema_sql())
        # Private resumable bulk builds do not maintain every secondary B-tree
        # for every inserted row. Uniqueness is validated before publication.
        if build["state"] == "building":
            if (output / "ONLINE.json").exists():
                raise IntegrityError("不能对已启用的在线库延迟索引")
            with db:
                for name in indexes:
                    db.execute("DROP INDEX IF EXISTS " + name)
        chunks = 0
        dictionary = {tag: tid for tid, tag in db.execute("SELECT tag_id,tag FROM tags")}
        raw_columns = [
            "raw_row",
            "observation_id",
            "source_metadata_format",
            "source_schema_id",
            "raw_bytes",
            "raw_sha256",
            "raw_zlib",
        ]
        tasks = [
            ("source_schemas", ["source_schema_id", "schema_ipc"], ["source_schema_id", "schema_ipc"]),
            ("tags", ["tag_id", "tag"], ["tag_id", "tag"]),
            (
                "current_posts",
                ["post_id", "row_id", "asset_id"],
                ["post_id", "valid_from", "row_id", "asset_id"],
            ),
            ("objects", OBJECT_COLUMNS, ["object_row", *OBJECT_COLUMNS, "first_seq"]),
            ("assets", ASSET_COLUMNS, ["asset_row", *ASSET_COLUMNS]),
            ("observations", OBS_COLUMNS, OBS_COLUMNS),
            (
                "raw_metadata",
                ["observation_id", "source_metadata_json", "source_metadata_format", "source_schema_id"],
                raw_columns,
            ),
        ]
        started = time.monotonic()
        try:
            with ThreadPoolExecutor(max_workers=4) as workers:
                for table, columns, target_columns in tasks:
                    target = "post_versions" if table == "current_posts" else table
                    prior = list(
                        db.execute(
                            "SELECT position,rows,digest,complete FROM build_progress WHERE name=?", (table,)
                        )
                    )
                    position, count, digest, complete = prior[0] if prior else (-1, 0, "0", 0)
                    if complete:
                        if table == "tags":
                            dictionary = {tag: tid for tid, tag in db.execute("SELECT tag_id,tag FROM tags")}
                        continue
                    accumulator = int(digest, 16)
                    maximum = native.execute(f"SELECT coalesce(max(rowid),-1) FROM {table}").fetchone()[0]
                    placeholders = ",".join("?" for _ in target_columns)
                    names = ",".join('"' + c + '"' for c in target_columns)
                    insert = f"INSERT INTO {target}({names}) VALUES({placeholders})"
                    while position < maximum:
                        end = min(maximum, position + chunk_rows)
                        selected = native.execute(
                            f"SELECT rowid,{','.join(chr(34) + c + chr(34) for c in columns)} FROM {table} WHERE rowid>? AND rowid<=?",
                            [position, end],
                        ).fetchall()
                        if table == "raw_metadata":
                            values = list(workers.map(pack_raw, selected, chunksize=128))
                        else:
                            values = []
                            for item in selected:
                                values_row = tuple(
                                    scalar(value, name) for name, value in zip(columns, item[1:])
                                )
                                if table == "current_posts":
                                    values_row = (values_row[0], sequence, *values_row[1:])
                                elif table == "objects":
                                    values_row = (item[0] + 1, *values_row, sequence)
                                elif table == "assets":
                                    values_row = (item[0] + 1, *values_row)
                                values.append(values_row)
                        local_digest = sum(row_digest(v) for v in values)
                        with db:
                            db.executemany(insert, values)
                            if table == "observations":
                                tag_column = columns.index("tag_string")
                                encoded = []
                                for value in values:
                                    tokens = set((value[tag_column] or "").split(" ")) - {""}
                                    missing = sorted(t for t in tokens if t not in dictionary)
                                    for tag in missing:
                                        tid = next(db.execute("SELECT coalesce(max(tag_id),0)+1 FROM tags"))[
                                            0
                                        ]
                                        db.execute("INSERT INTO tags VALUES(?,?)", (tid, tag))
                                        dictionary[tag] = tid
                                        previous = next(
                                            db.execute(
                                                "SELECT rows,digest FROM build_progress WHERE name='tags'"
                                            )
                                        )
                                        db.execute(
                                            "UPDATE build_progress SET rows=?,digest=? WHERE name='tags'",
                                            (
                                                previous[0] + 1,
                                                format(
                                                    (int(previous[1], 16) + row_digest((tid, tag))) % MODULUS,
                                                    "x",
                                                ),
                                            ),
                                        )
                                    encoded.append(
                                        (value[0], " ".join("t" + format(dictionary[t], "x") for t in tokens))
                                    )
                                db.executemany("INSERT INTO tag_index(rowid,tokens) VALUES(?,?)", encoded)
                            count += len(values)
                            accumulator = (accumulator + local_digest) % MODULUS
                            position = end
                            db.execute(
                                "INSERT OR REPLACE INTO build_progress VALUES(?,?,?,?,0)",
                                (table, position, count, format(accumulator, "x")),
                            )
                        chunks += 1
                        emit(
                            "migration_chunk",
                            site=site,
                            table=table,
                            rows=count,
                            position=position,
                            maximum=maximum,
                            seconds=round(time.monotonic() - started, 3),
                        )
                        if stop_after_chunks is not None and chunks >= stop_after_chunks:
                            return {"state": "building", "file": str(path), "chunks": chunks}
                    with db:
                        db.execute(
                            "INSERT OR REPLACE INTO build_progress VALUES(?,?,?,?,1)",
                            (table, position, count, format(accumulator, "x")),
                        )
                    if table == "tags":
                        dictionary = {tag: tid for tid, tag in db.execute("SELECT tag_id,tag FROM tags")}
                    emit("migration_table_complete", site=site, table=table, rows=count)
            emit("migration_secondary_indexes", site=site)
            for statement in index_sql:
                db.execute(statement.replace("INDEX ", "INDEX IF NOT EXISTS ", 1))
            if not list(
                db.execute("SELECT 1 FROM build_progress WHERE name='object_versions' AND complete=1")
            ):
                with db:
                    db.execute(
                        "INSERT INTO object_versions(sha256,valid_from,post_id) SELECT o.sha256,?,min(a.post_id) FROM objects o LEFT JOIN assets a ON a.sha256=o.sha256 GROUP BY o.sha256",
                        (sequence,),
                    )
                    db.execute("INSERT INTO build_progress VALUES('object_versions',0,0,'0',1)")
            emit("migration_index_optimize", site=site)
            db.execute("INSERT INTO tag_index(tag_index) VALUES('optimize')")
            db.execute("ANALYZE")
            counts = {
                name: next(db.execute("SELECT count(*) FROM " + name))[0]
                for name in ["objects", "assets", "observations"]
            }
            batch_id = native.execute("SELECT batch_id FROM applied WHERE seq=?", [sequence]).fetchone()
            with db:
                db.execute(
                    "INSERT OR IGNORE INTO publications VALUES(?,?,?,?,?,?)",
                    (
                        sequence,
                        batch_id[0] if batch_id else "empty-baseline",
                        now(),
                        counts["objects"],
                        counts["observations"],
                        counts["assets"],
                    ),
                )
                set_state(
                    db,
                    served_seq=sequence,
                    object_count=counts["objects"],
                    observation_count=counts["observations"],
                    asset_count=counts["assets"],
                )
            db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
            build.update(state="built", counts=counts, sqlite_version=apsw.sqlitelibversion(), built_at=now())
            atomic_json(control, build)
        finally:
            db.close()
            native.close()
    if activate:
        activate_projection(media, output)
    return build


def verify_projection(output, *, full=True, memory_mib=2048):
    output = Path(output).resolve()
    build = read_json(output / "ONLINE-BUILD.json")
    if build["state"] not in {"built", "verified", "active"}:
        raise IntegrityError("准备库尚未构建完成")
    db = connect(output / build["file"])
    try:
        if not 256 <= memory_mib <= 16384:
            raise ValueError("校验缓存预算必须为 256–16384 MiB")
        # Mapping the whole 80+ GiB file can fill a Windows process working set.
        # Keep mapped pages within half the explicit verification cache budget.
        db.execute(f"PRAGMA cache_size=-{memory_mib * 1024}; PRAGMA mmap_size={memory_mib * 512 * 1024}")
        emit("migration_verify_phase", site=build["site"], phase="sqlite_quick_check")
        check = list(db.execute("PRAGMA quick_check"))
        if check != [("ok",)]:
            raise IntegrityError(str(check))
        emit("migration_verify_phase", site=build["site"], phase="tag_integrity")
        db.execute("INSERT INTO tag_index(tag_index) VALUES('integrity-check')")
        relations = {
            "missing_object": "SELECT count(*) FROM assets a LEFT JOIN objects o ON o.sha256=a.sha256 WHERE a.sha256 IS NOT NULL AND o.sha256 IS NULL",
            "missing_origin": "SELECT count(*) FROM assets a LEFT JOIN observations o ON o.observation_id=a.observation_id WHERE a.observation_id IS NOT NULL AND o.observation_id IS NULL",
            "missing_current": "SELECT count(*) FROM post_versions p LEFT JOIN observations o ON o.row_id=p.row_id WHERE o.row_id IS NULL",
            "missing_raw": "SELECT count(*) FROM observations o LEFT JOIN raw_metadata r ON r.observation_id=o.observation_id WHERE r.observation_id IS NULL",
        }
        failures = {}
        for key, sql in relations.items():
            emit("migration_verify_phase", site=build["site"], phase=key)
            failures[key] = next(db.execute(sql))[0]
        if any(failures.values()):
            raise IntegrityError("在线引用不完整: " + str(failures))
        verified = {}
        if full:
            tasks = [
                ("source_schemas", "source_schemas", ["source_schema_id", "schema_ipc"]),
                ("tags", "tags", ["tag_id", "tag"]),
                ("current_posts", "post_versions", ["post_id", "valid_from", "row_id", "asset_id"]),
                ("objects", "objects", ["object_row", *OBJECT_COLUMNS, "first_seq"]),
                ("assets", "assets", ["asset_row", *ASSET_COLUMNS]),
                ("observations", "observations", OBS_COLUMNS),
                (
                    "raw_metadata",
                    "raw_metadata",
                    [
                        "raw_row",
                        "observation_id",
                        "source_metadata_format",
                        "source_schema_id",
                        "raw_bytes",
                        "raw_sha256",
                        "raw_zlib",
                    ],
                ),
            ]
            for progress, table, columns in tasks:
                expected = next(
                    db.execute("SELECT rows,digest FROM build_progress WHERE name=?", (progress,))
                )
                count = accumulator = 0
                for row in db.execute(
                    "SELECT " + ",".join('"' + c + '"' for c in columns) + " FROM " + table
                ):
                    accumulator = (accumulator + row_digest(row)) % MODULUS
                    count += 1
                if (count, format(accumulator, "x")) != expected:
                    raise IntegrityError("全行校验失败: " + table)
                verified[table] = {"rows": count, "digest": format(accumulator, "x")}
                emit("migration_table_verified", site=build["site"], table=table, rows=count)
        result = {"checked_at": now(), "full_row_checksums": full, "tables": verified, "relations": failures}
        atomic_json(output / "ONLINE-VERIFY.json", result)
        if full:
            build.update(state="verified", verified_at=now())
            atomic_json(output / "ONLINE-BUILD.json", build)
        return result
    finally:
        db.close()


def activate_projection(media, output):
    media, output = Path(media).resolve(), Path(output).resolve()
    build = read_json(output / "ONLINE-BUILD.json")
    if build["state"] not in {"verified", "active"}:
        raise IntegrityError("正式启用前必须完成全行校验")
    if read_json(media / "library.json")["library_id"] != build["library_id"]:
        raise IntegrityError("主库身份不匹配")
    pointer = {k: build[k] for k in ["library_id", "generation", "file", "site", "schema_version"]}
    atomic_json(output / "ONLINE.json", pointer)
    # Producer discovery is independent of the old analytical cache location.
    atomic_json(
        media / "online-index.json",
        {"schema_version": VERSION, "library_id": build["library_id"], "index_root": str(output)},
    )
    build["state"] = "active"
    atomic_json(output / "ONLINE-BUILD.json", build)
    return pointer


def main():
    import argparse

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["build", "verify", "activate", "sync", "gc", "status"])
    parser.add_argument("--media", type=Path)
    parser.add_argument("--source-index", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--site", choices=["danbooru", "yandere", "gelbooru"])
    parser.add_argument("--memory-mib", type=int, default=2048)
    args = parser.parse_args()
    if args.command in {"build", "activate", "sync", "gc"} and args.media is None:
        parser.error("此命令需要 --media")
    if args.command == "build" and (args.source_index is None or args.site is None):
        parser.error("build 需要 --source-index 和 --site")
    if args.command == "build":
        result = migrate(args.media, args.source_index, args.output, args.site)
    elif args.command == "verify":
        result = verify_projection(args.output, memory_mib=args.memory_mib)
    elif args.command == "activate":
        result = activate_projection(args.media, args.output)
    elif args.command == "status":
        pointer = read_json(args.output / "ONLINE.json")
        db = apsw.Connection(str(contained(args.output, pointer["file"])), flags=apsw.SQLITE_OPEN_READONLY)
        try:
            result = settings(db)
            result["leases"] = next(db.execute("SELECT count(*) FROM leases"))[0]
        finally:
            db.close()
    else:
        from .online import Publisher

        publisher = Publisher(args.media, args.output)
        result = publisher.sync() if args.command == "sync" else {"min_seq": publisher.collect_versions()}
    emit("complete", result=result)


if __name__ == "__main__":
    main()
