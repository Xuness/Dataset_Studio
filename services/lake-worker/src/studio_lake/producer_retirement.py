"""Audited retirement of rebuildable producer files while keeping serving generations intact."""

from contextlib import ExitStack, closing, contextmanager
import hashlib
import json
from pathlib import Path
import sqlite3

import pyarrow.parquet as pq

from .archive_rebuild import journal_snapshot
from .archive_bindings import load as load_bindings
from .online_storage import connect
from .online_schema import set_state
from .util import (
    FileLock,
    IntegrityError,
    atomic_json,
    contained,
    file_hash,
    now,
    read_json,
    safe_managed_path,
)

MARKER = "PRODUCER-RETIRED.json"
NATIVE_FILES = {
    "analysis.duckdb",
    "analysis.duckdb.wal",
    "catalog.sqlite",
    "catalog.sqlite-wal",
    "catalog.sqlite-shm",
}


@contextmanager
def maintenance_locks(media, index, identity):
    with ExitStack() as locks:
        controller = index / "UPDATE-CONTROLLER.json"
        if controller.exists():
            from .updates.state import State
            from .updates.locations import access

            owner = read_json(controller)
            if owner["library_id"] != identity:
                raise IntegrityError("更新控制器身份不匹配")
            state = State(Path(owner["root"]))
            registered = state.library(identity)
            if registered.root.resolve() != media or registered.cache.resolve() != index:
                raise IntegrityError("维护位置与 Studio 登记不一致")
            locks.enter_context(access(state, identity))
        for root, name in (
            (index, ".daily-run.lock"),
            (media, ".writer.lock"),
            (index, ".index.lock"),
            (index, ".online.lock"),
        ):
            locks.enter_context(FileLock(root / name, timeout=1))
        yield


def _identity(media, index):
    media, index = Path(media).resolve(), Path(index).resolve()
    if media == index or media in index.parents or index in media.parents:
        raise IntegrityError("主库与在线目录必须独立")
    info, pointer = read_json(media / "library.json"), read_json(index / "ONLINE.json")
    owner = read_json(index / "cache_owner.json")
    linked = read_json(media / "online-index.json")
    if (
        info["library_id"] != pointer["library_id"]
        or owner["library_id"] != info["library_id"]
        or Path(owner["root"]).resolve() != media
        or Path(linked["index_root"]).resolve() != index
        or linked["library_id"] != info["library_id"]
        or pointer["schema_version"] != 2
    ):
        raise IntegrityError("主库与在线位置身份不一致")
    return media, index, pointer


def _backup_journal(media, directory):
    directory.mkdir(parents=True, exist_ok=True)
    target = directory / "journal-before-handoff.sqlite"
    if not target.exists():
        temporary = directory / "journal-before-handoff.sqlite.tmp"
        if temporary.exists():
            raise IntegrityError("上一次日志备份未完成，保留现场")
        with (
            closing(sqlite3.connect((media / "journal.sqlite").as_uri() + "?mode=ro", uri=True)) as source,
            closing(sqlite3.connect(temporary)) as destination,
        ):
            source.backup(destination)
            if destination.execute("PRAGMA quick_check").fetchone() != ("ok",):
                raise IntegrityError("交接前日志备份校验失败")
        temporary.replace(target)
    return {"file": str(target), "bytes": target.stat().st_size, "sha256": file_hash(target)}


def handoff_legacy(media, index, state_root, run_id, job_id, *, apply=False):
    media, index, pointer = _identity(media, index)
    state_root = Path(state_root).resolve()
    controller = read_json(index / "UPDATE-CONTROLLER.json")
    if Path(controller["root"]).resolve() != state_root:
        raise IntegrityError("交接任务不属于已登记控制器")
    with maintenance_locks(media, index, pointer["library_id"]):
        with closing(sqlite3.connect((media / "journal.sqlite").as_uri() + "?mode=ro", uri=True)) as journal:
            journal.row_factory = sqlite3.Row
            old = journal.execute("SELECT * FROM daily_runs WHERE run_id=?", (run_id,)).fetchone()
            if old is None:
                raise IntegrityError("旧日任务不存在")
            old = dict(old)
            if old["state"] == "handed_off":
                saved = journal.execute(
                    "SELECT receipt_json FROM legacy_task_handoffs WHERE run_id=?", (run_id,)
                ).fetchone()
                if not saved or json.loads(saved[0])["studio_job_id"] != job_id:
                    raise IntegrityError("旧任务交接身份冲突")
                return json.loads(saved[0])
            if old["state"] != "planning" or not old["api_complete"]:
                raise IntegrityError("仅可交接已结束 API 捕获、尚未规划图片的旧任务")
            expected = set()
            for (batch,) in journal.execute(
                "SELECT batch_id FROM daily_run_batches WHERE run_id=? AND role='api'", (run_id,)
            ):
                path = contained(media, "segments/" + batch + "/observations.parquet")
                expected.update(
                    p
                    for p in pq.read_table(path, columns=["post_id"])["post_id"].to_pylist()
                    if p is not None
                )
        with closing(
            sqlite3.connect((state_root / "updates.sqlite").as_uri() + "?mode=ro", uri=True)
        ) as state:
            state.row_factory = sqlite3.Row
            job = state.execute("SELECT * FROM jobs WHERE id=?", (job_id,)).fetchone()
            if job is None or job["lake_id"] != pointer["library_id"]:
                raise IntegrityError("Studio 接续任务身份不匹配")
            definition = json.loads(job["definition"])
            if (
                definition["range"]["kind"] != "ids"
                or set(definition["range"]["ids"]) != expected
                or definition["media"]["profile"] != json.loads(old["parameters_json"])["profile"]
                or definition["media"]["existing"] != "keep"
            ):
                raise IntegrityError("Studio 接续范围或保存策略与旧任务不一致")
            if job["state"] not in {"completed", "completed_with_exclusions"}:
                raise IntegrityError("Studio 接续任务尚未完成")
            items = [
                dict(r)
                for r in state.execute(
                    "SELECT post_id,state FROM items WHERE job_id=? ORDER BY post_id", (job_id,)
                )
            ]
            if {r["post_id"] for r in items} != expected or any(
                r["state"] not in {"stored", "reused", "unavailable"} for r in items
            ):
                raise IntegrityError("Studio 接续结果未覆盖全部旧任务帖子")
        receipt = {
            "schema_version": 1,
            "kind": "legacy_task_handoff",
            "library_id": pointer["library_id"],
            "legacy_run_id": run_id,
            "original_state": old["state"],
            "studio_job_id": job_id,
            "studio_state": job["state"],
            "post_count": len(expected),
            "post_ids_sha256": hashlib.sha256(json.dumps(sorted(expected)).encode()).hexdigest(),
            "counts": {s: sum(r["state"] == s for r in items) for s in ("stored", "reused", "unavailable")},
            "definition": definition,
            "at": now(),
            "applied": apply,
        }
        if apply:
            directory = safe_managed_path(index, index / "retirement")
            receipt["journal_backup"] = _backup_journal(media, directory)
            with closing(sqlite3.connect(media / "journal.sqlite")) as journal, journal:
                journal.execute(
                    "CREATE TABLE IF NOT EXISTS legacy_task_handoffs("
                    "run_id TEXT PRIMARY KEY,job_id TEXT NOT NULL,receipt_json TEXT NOT NULL)"
                )
                current = journal.execute(
                    "SELECT state,updated_at FROM daily_runs WHERE run_id=?", (run_id,)
                ).fetchone()
                if current != (old["state"], old["updated_at"]):
                    raise IntegrityError("旧任务在交接检查后发生变化")
                journal.execute(
                    "INSERT INTO legacy_task_handoffs VALUES(?,?,?)",
                    (run_id, job_id, json.dumps(receipt, ensure_ascii=False)),
                )
                # A handoff is not a legacy verification. Its old watermark and completion report remain untouched.
                journal.execute(
                    "UPDATE daily_runs SET state='handed_off',updated_at=? WHERE run_id=?", (now(), run_id)
                )
            atomic_json(directory / ("handoff-" + run_id + ".json"), receipt)
        return receipt


def _check_handoffs(media):
    with closing(sqlite3.connect((media / "journal.sqlite").as_uri() + "?mode=ro", uri=True)) as db:
        if not db.execute("SELECT 1 FROM sqlite_master WHERE name='daily_runs'").fetchone():
            return
        pending = db.execute(
            "SELECT run_id,state FROM daily_runs WHERE state NOT IN "
            "('verified','verified_with_exclusions','handed_off')"
        ).fetchall()
        if pending:
            raise IntegrityError("仍有未交接的旧日任务: " + str(pending))
        for (run,) in db.execute("SELECT run_id FROM daily_runs WHERE state='handed_off'"):
            row = db.execute(
                "SELECT receipt_json FROM legacy_task_handoffs WHERE run_id=?", (run,)
            ).fetchone()
            if not row or not json.loads(row[0]).get("applied"):
                raise IntegrityError("旧任务缺少持久交接凭据")


def _check_rebuild(media, index, pointer, proof_root):
    proof_root = Path(proof_root).resolve()
    build, proof, compared = (
        read_json(proof_root / name)
        for name in ("ONLINE-BUILD.json", "ONLINE-VERIFY.json", "ONLINE-COMPARE.json")
    )
    if (
        build.get("source") != "canonical-archive-v1"
        or build["state"] != "verified"
        or build["library_id"] != pointer["library_id"]
        or build["site"] != pointer["site"]
        or Path(build["media_root"]).resolve() != media
        or Path(build["reference_index"]).resolve() != index
        or compared.get("equal") is not True
        or compared["reference_generation"] != pointer["generation"]
        or compared["sequence"] != build["sequence"]
        or proof["sequence"] != build["sequence"]
        or proof.get("raw_roundtrip_verified") is not True
        or proof.get("legacy_bindings_sha256") != build.get("legacy_bindings_sha256")
    ):
        raise IntegrityError("缺少与当前湖匹配的独立归档恢复及在线对照证据")
    if load_bindings(media, build["library_id"], build["site"])[1] != build.get("legacy_bindings_sha256"):
        raise IntegrityError("历史关联归档在验证后发生变化")
    snapshot = journal_snapshot(media, build["sequence"])
    if any(snapshot[key] != build[key] for key in snapshot):
        raise IntegrityError("验证过的归档前缀发生变化")
    path = contained(proof_root, build["file"])
    wal = Path(str(path) + "-wal")
    if (wal.exists() and wal.stat().st_size) or file_hash(path) != proof.get("file_sha256"):
        raise IntegrityError("归档重建准备库在验证后发生变化")
    # Check the captured source-file identities without rereading terabytes of immutable image bytes.
    with closing(sqlite3.connect(path.as_uri() + "?mode=ro", uri=True)) as db:
        for name, mtime, size, sha in db.execute(
            "SELECT name,position,rows,digest FROM build_progress "
            "WHERE name LIKE 'input:%' OR name LIKE 'image:%'"
        ):
            kind, batch, filename = name.split(":", 2)
            source = contained(media, "segments/" + batch + "/" + filename)
            if not source.is_file() or source.stat().st_size != size:
                raise IntegrityError("验证后的归档文件缺失或长度变化: " + str(source))
            if source.stat().st_mtime_ns != mtime:
                if kind == "image" or file_hash(source) != sha:
                    raise IntegrityError("验证后的归档文件发生变化: " + str(source))
    return build, proof, compared


def retire_producer(media, index, proof_root, *, apply=False):
    media, index, pointer = _identity(media, index)
    marker = index / MARKER
    with maintenance_locks(media, index, pointer["library_id"]):
        previous = read_json(marker) if marker.exists() else None
        if previous and (
            previous.get("library_id") != pointer["library_id"]
            or previous.get("online_generation") != pointer["generation"]
        ):
            raise IntegrityError("退役凭据与现用在线库不一致")
        if previous and previous["phase"] == "complete":
            if any((index / entry["relative"]).exists() for entry in previous["files"]):
                raise IntegrityError("已退役的生产者文件重新出现，请先检查旧入口")
            return previous
        _check_handoffs(media)
        build, proof, compared = _check_rebuild(media, index, pointer, proof_root)
        if previous:
            receipt = previous
        else:
            current = read_json(index / "CURRENT.json")
            if current["library_id"] != pointer["library_id"] or current.get("index_version") != 1:
                raise IntegrityError("生产者指针身份或格式无效")
            generation = current["generation"]
            if not generation.startswith("gen-") or "/" in generation or "\\" in generation:
                raise IntegrityError("生产者代次路径无效")
            directory = safe_managed_path(index, index / "indexes" / generation)
            with closing(
                sqlite3.connect((directory / "catalog.sqlite").as_uri() + "?mode=ro", uri=True)
            ) as db:
                sequence = db.execute("SELECT value FROM state WHERE key='seq'").fetchone()[0]
            if sequence > build["sequence"]:
                raise IntegrityError("归档重建验证未覆盖当前生产者水位")
            entries = []
            for path in sorted(directory.iterdir()):
                safe_managed_path(index, path)
                if not path.is_file() or path.name not in NATIVE_FILES or path.stat().st_nlink != 1:
                    raise IntegrityError("生产者目录存在未知文件、链接或共享文件，保留: " + str(path))
                st = path.stat()
                entries.append(
                    {
                        "relative": path.relative_to(index).as_posix(),
                        "bytes": st.st_size,
                        "mtime_ns": st.st_mtime_ns,
                        "deleted": False,
                    }
                )
            receipt = {
                "schema_version": 1,
                "kind": "producer_index_retirement",
                "phase": "prepared",
                "library_id": pointer["library_id"],
                "online_generation": pointer["generation"],
                "legacy_current": current,
                "legacy_sequence": sequence,
                "verified_sequence": build["sequence"],
                "journal_digest": build["journal_digest"],
                "prepared_at": now(),
                "files": entries,
                "candidate_bytes": sum(r["bytes"] for r in entries),
                "deleted_bytes": 0,
                "deleted_files": 0,
            }
        if not apply:
            return {**receipt, "apply": False}
        audit = safe_managed_path(index, index / "retirement")
        audit.mkdir(exist_ok=True)
        for filename, contents in (
            ("archive-build.json", build),
            ("archive-verify.json", proof),
            ("archive-compare.json", compared),
            ("CURRENT.before.json", receipt["legacy_current"]),
        ):
            if not (audit / filename).exists():
                atomic_json(audit / filename, contents)
        # This durable marker is installed before the first unlink. Old owned commands fail closed.
        atomic_json(marker, receipt)
        atomic_json(
            index / "CURRENT.json",
            {
                "library_id": pointer["library_id"],
                "index_version": 0,
                "generation": receipt["legacy_current"]["generation"],
                "retired": True,
                "replacement": "ONLINE.json",
            },
        )
        try:
            for entry in receipt["files"]:
                if entry["deleted"]:
                    continue
                path = safe_managed_path(index, index / entry["relative"])
                if not path.exists():
                    entry["absent_at_resume"] = True
                    atomic_json(marker, receipt)
                    continue
                current = path.stat()
                if current.st_size != entry["bytes"] or current.st_mtime_ns != entry["mtime_ns"]:
                    raise IntegrityError("生产者文件在清单生成后变化: " + str(path))
                path.unlink()
                entry["deleted"] = True
                receipt["deleted_bytes"] += entry["bytes"]
                receipt["deleted_files"] += 1
                atomic_json(marker, receipt)
            directory = safe_managed_path(index, index / "indexes" / receipt["legacy_current"]["generation"])
            if directory.exists():
                directory.rmdir()
            db = connect(contained(index, pointer["file"]))
            try:
                with db:
                    set_state(db, producer_index_retired=1, producer_retired_at=now())
            finally:
                db.close()
            receipt.update(phase="complete", completed_at=now())
            receipt.pop("error", None)
            atomic_json(audit / "retirement-receipt.json", receipt)
            atomic_json(marker, receipt)
        except Exception as error:
            receipt["error"] = str(error)
            atomic_json(marker, receipt)
            raise
        return receipt


def main():
    import argparse

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("handoff", "retire"))
    parser.add_argument("--media", type=Path, required=True)
    parser.add_argument("--index", type=Path, required=True)
    parser.add_argument("--proof", type=Path)
    parser.add_argument("--state-root", type=Path)
    parser.add_argument("--run")
    parser.add_argument("--job")
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()
    if args.command == "handoff":
        if args.state_root is None or args.run is None or args.job is None:
            parser.error("handoff 需要 --state-root、--run、--job")
        result = handoff_legacy(args.media, args.index, args.state_root, args.run, args.job, apply=args.apply)
    else:
        if args.proof is None:
            parser.error("retire 需要 --proof")
        result = retire_producer(args.media, args.index, args.proof, apply=args.apply)
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
