"""Durable ingest runs in the main journal; disposable execution queues live on SSD."""

from pathlib import Path
import json
import re
import sqlite3
import uuid

from .util import IntegrityError, atomic_json, now, read_json, safe_managed_path
from .daily_paths import assign_layouts, ensure_run_directory, run_directory, write_day_summary

SCHEMA_VERSION = 2
VERIFIED = {"verified", "verified_with_exclusions"}
DDL = [
    """CREATE TABLE IF NOT EXISTS daily_runs (
      run_id TEXT PRIMARY KEY, kind TEXT NOT NULL, state TEXT NOT NULL,
      baseline_id INTEGER, high_id INTEGER, api_complete INTEGER NOT NULL DEFAULT 0,
      api_state_json TEXT NOT NULL DEFAULT '{}', parameters_json TEXT NOT NULL,
      plan_sha256 TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
      completed_at TEXT, error TEXT, result_json TEXT
    )""",
    """CREATE TABLE IF NOT EXISTS daily_run_batches (
      run_id TEXT NOT NULL REFERENCES daily_runs(run_id),
      batch_id TEXT NOT NULL REFERENCES commits(batch_id),
      seq INTEGER NOT NULL, role TEXT NOT NULL,
      PRIMARY KEY(run_id,batch_id)
    )""",
    "CREATE INDEX IF NOT EXISTS daily_batches_run_seq ON daily_run_batches(run_id,seq)",
    """CREATE TABLE IF NOT EXISTS batch_verifications (
      batch_id TEXT PRIMARY KEY REFERENCES commits(batch_id),
      manifest_sha256 TEXT NOT NULL, verifier_version INTEGER NOT NULL,
      verified_at TEXT NOT NULL, result_json TEXT NOT NULL
    )""",
]


def checked_run_id(value):
    if not re.fullmatch(r"[a-f0-9]{32}", value):
        raise ValueError("无效日任务 ID")
    return value


def record_ingest_commit(db, manifest, seq):
    """Called inside the SAME SQLite transaction that accepts a durable batch."""
    source = manifest.get("source", {})
    run_id = source.get("ingest_run_id")
    if not run_id:
        return
    checked_run_id(run_id)
    run = db.execute("SELECT * FROM daily_runs WHERE run_id=?", (run_id,)).fetchone()
    if run is None or run["state"] in VERIFIED or run["state"] == "handed_off":
        raise IntegrityError("批次所属日任务不存在或已经封闭")
    role = source.get("ingest_role", "images")
    stats, request = source.get("response_stats", {}), source.get("request", {})
    if role == "api" and (
        request.get("status", 200) != 200 or stats.get("valid") is False or stats.get("pagination_error")
    ):
        role = "api_error"
    if role == "api" and run["api_complete"]:
        raise IntegrityError("日任务的 API 范围已封闭，拒绝追加新页面")
    db.execute(
        "INSERT INTO daily_run_batches VALUES (?,?,?,?)",
        (run_id, manifest["batch_id"], seq, role),
    )
    if role == "api":
        if request.get("mode") == "refresh":
            state = {"next_chunk": request["chunk"] + 1}
            complete = state["next_chunk"] == request["chunks"]
            db.execute(
                "UPDATE daily_runs SET api_state_json=?,api_complete=?,state=?,updated_at=? WHERE run_id=?",
                (json.dumps(state), int(complete), "planning" if complete else "fetching", now(), run_id),
            )
        else:
            settings = manifest.get("settings", {})
            if "active_api_run" in settings:
                active = settings["active_api_run"]
                high = active["high_id"] if active else settings["api_watermark"]
                db.execute(
                    "UPDATE daily_runs SET high_id=?,api_complete=?,api_state_json=?,state=?,updated_at=? "
                    "WHERE run_id=?",
                    (
                        high,
                        int(active is None),
                        json.dumps(active or {}),
                        "planning" if active is None else "fetching",
                        now(),
                        run_id,
                    ),
                )
    else:
        db.execute("UPDATE daily_runs SET updated_at=? WHERE run_id=?", (now(), run_id))
        if role == "api_error" and request.get("retry_after_at"):
            api_state = json.loads(run["api_state_json"])
            api_state["next_request_after"] = request["retry_after_at"]
            db.execute(
                "UPDATE daily_runs SET api_state_json=? WHERE run_id=?", (json.dumps(api_state), run_id)
            )


class DailyStore:
    def __init__(self, lib):
        self.lib = lib

    def initialize(self):
        # Run only from the new ingest entrypoints, never while merely opening a library.
        with self.lib.writer_lock(), self.lib.journal() as db, db:
            row = db.execute("SELECT value FROM settings WHERE key='daily_schema_version'").fetchone()
            if row and json.loads(row[0]) not in {1, SCHEMA_VERSION}:
                raise IntegrityError("不支持的日任务日志版本")
            for statement in DDL:
                db.execute(statement)
            layouts = assign_layouts(db)
            db.execute(
                "INSERT OR REPLACE INTO settings VALUES ('daily_schema_version',?)",
                (json.dumps(SCHEMA_VERSION),),
            )
        self.lib._daily_layouts = layouts

    def directory(self, run_id):
        return run_directory(self.lib, checked_run_id(run_id), "reports")

    def ensure_directory(self, run_id):
        # The journal registration precedes filesystem writes: an interrupted mkdir/owner
        # write must resume the same run, not strand an unregistered calendar ordinal.
        run = self.get(run_id)
        with self.lib.writer_lock():
            directory = ensure_run_directory(self.lib, run_id, "reports")
            marker = safe_managed_path(self.lib.root, directory / "run.json")
            record = {
                "run_id": run_id,
                "kind": run["kind"],
                "baseline_id": run["baseline_id"],
                "parameters": run["parameters"],
                "created_at": run["created_at"],
                "library_id": self.lib.info["library_id"],
            }
            if marker.exists():
                if read_json(marker) != record:
                    raise IntegrityError("主库任务记录与日志登记不一致")
            else:
                atomic_json(marker, record)
        return directory

    def work_directory(self, run_id, *, create=False):
        locate = ensure_run_directory if create else run_directory
        return locate(self.lib, checked_run_id(run_id), "work")

    def diagnostics_directory(self, run_id):
        return run_directory(self.lib, checked_run_id(run_id), "diagnostics")

    @staticmethod
    def decode(row):
        result = dict(row)
        for column, name in [
            ("api_state_json", "api_state"),
            ("parameters_json", "parameters"),
            ("result_json", "result"),
        ]:
            value = result.pop(column)
            result[name] = json.loads(value) if value else None
        result["api_complete"] = bool(result["api_complete"])
        return result

    def get(self, run_id):
        checked_run_id(run_id)
        with self.lib.journal() as db:
            row = db.execute("SELECT * FROM daily_runs WHERE run_id=?", (run_id,)).fetchone()
        if row is None:
            raise ValueError("日任务不存在")
        return self.decode(row)

    def runs(self, *, unfinished=False, limit=None):
        query = "SELECT * FROM daily_runs"
        if unfinished:
            query += " WHERE state NOT IN ('verified','verified_with_exclusions','handed_off')"
        query += " ORDER BY created_at,run_id"
        args = ()
        if limit is not None:
            query += " LIMIT ?"
            args = (limit,)
        with self.lib.journal() as db:
            return [self.decode(row) for row in db.execute(query, args)]

    def create(self, kind, parameters, baseline=None):
        if kind not in {"new_posts", "refresh", "backfill"}:
            raise ValueError("未知日任务类型")
        run_id = uuid.uuid4().hex
        timestamp = now()
        with self.lib.writer_lock(), self.lib.journal() as db, db:
            if kind == "new_posts":
                if baseline is None or baseline < 0:
                    raise ValueError("首次日入库必须指定已验证的 --after-id")
                current = db.execute("SELECT value FROM settings WHERE key='api_watermark'").fetchone()
                if current and int(json.loads(current[0])) != baseline:
                    raise ValueError("起始 ID 与当前 API 游标不一致；旧帖应使用刷新任务")
                verified = db.execute(
                    "SELECT value FROM settings WHERE key='verified_ingest_watermark'"
                ).fetchone()
                if verified is None:
                    if not parameters.get("explicit_baseline"):
                        raise ValueError("首次接管需显式给出经过旧流程验证的 --after-id")
                    db.execute(
                        "INSERT INTO settings VALUES ('verified_ingest_watermark',?)", (json.dumps(baseline),)
                    )
                    db.execute(
                        "INSERT OR REPLACE INTO settings VALUES ('daily_baseline',?)",
                        (json.dumps({"id": baseline, "at": timestamp, "source": "explicit_after_id"}),),
                    )
            db.execute(
                "INSERT INTO daily_runs(run_id,kind,state,baseline_id,parameters_json,created_at,updated_at) "
                "VALUES (?,?,'fetching',?,?,?,?)",
                (run_id, kind, baseline, json.dumps(parameters, ensure_ascii=False), timestamp, timestamp),
            )
            layouts = assign_layouts(db)
        self.lib._daily_layouts = layouts
        self.ensure_directory(run_id)
        write_day_summary(self.lib, layouts[run_id]["day"])
        return self.get(run_id)

    def update(self, run_id, **changes):
        allowed = {"state", "api_complete", "plan_sha256", "error", "result_json"}
        if not changes or not set(changes) <= allowed:
            raise ValueError("无效的日任务更新字段")
        if changes.get("state") in VERIFIED:
            raise ValueError("验收完成必须使用 finish")
        changes["updated_at"] = now()
        query = "UPDATE daily_runs SET " + ",".join(f"{key}=?" for key in changes) + " WHERE run_id=?"
        with self.lib.writer_lock(), self.lib.journal() as db, db:
            db.execute(query, (*changes.values(), checked_run_id(run_id)))

    def batches(self, run_id):
        with self.lib.journal() as db:
            return [
                {"seq": r["seq"], "role": r["role"], "manifest": json.loads(r["manifest_json"])}
                for r in db.execute(
                    "SELECT b.seq,b.role,c.manifest_json FROM daily_run_batches b "
                    "JOIN commits c ON c.batch_id=b.batch_id WHERE b.run_id=? ORDER BY b.seq",
                    (checked_run_id(run_id),),
                )
            ]

    def record_verification(self, batch_id, manifest_sha, version, result):
        with self.lib.writer_lock(), self.lib.journal() as db, db:
            db.execute(
                "INSERT OR REPLACE INTO batch_verifications VALUES (?,?,?,?,?)",
                (batch_id, manifest_sha, version, now(), json.dumps(result, ensure_ascii=False)),
            )

    def finish(self, run_id, report):
        if report["pending"] or report["failed"] or not report["verification_passed"]:
            raise IntegrityError("尚有未解决项或验证未完成，拒绝推进验收水位")
        state = "verified_with_exclusions" if report["unavailable"] else "verified"
        with self.lib.writer_lock(), self.lib.journal() as db, db:
            run = db.execute("SELECT * FROM daily_runs WHERE run_id=?", (run_id,)).fetchone()
            if not run or not run["api_complete"] or not run["plan_sha256"]:
                raise IntegrityError("采集或冻结计划未完成")
            missing = db.execute(
                "SELECT count(*) FROM daily_run_batches b LEFT JOIN batch_verifications v "
                "ON b.batch_id=v.batch_id WHERE b.run_id=? AND v.batch_id IS NULL",
                (run_id,),
            ).fetchone()[0]
            if missing:
                raise IntegrityError("尚有本轮批次未经核验")
            db.execute(
                "UPDATE daily_runs SET state=?,completed_at=?,updated_at=?,error=NULL,result_json=? "
                "WHERE run_id=?",
                (state, now(), now(), json.dumps(report, ensure_ascii=False), run_id),
            )
            row = db.execute("SELECT value FROM settings WHERE key='verified_ingest_watermark'").fetchone()
            watermark = int(json.loads(row[0])) if row else None
            if watermark is not None:
                while True:
                    next_row = db.execute(
                        "SELECT max(high_id) FROM daily_runs WHERE kind='new_posts' AND api_complete=1 "
                        "AND state IN ('verified','verified_with_exclusions') AND baseline_id<=? AND high_id>?",
                        (watermark, watermark),
                    ).fetchone()[0]
                    if next_row is None:
                        break
                    watermark = next_row
                db.execute(
                    "INSERT OR REPLACE INTO settings VALUES ('verified_ingest_watermark',?)",
                    (json.dumps(watermark),),
                )
            api = db.execute("SELECT value FROM settings WHERE key='api_watermark'").fetchone()
            report.update(
                status=state,
                api_watermark=json.loads(api[0]) if api else None,
                verified_ingest_watermark=watermark,
            )
            # A report write failure must not commit the completed state or watermark.
            atomic_json(self.directory(run_id) / "report.json", report)
            db.execute(
                "UPDATE daily_runs SET result_json=? WHERE run_id=?",
                (json.dumps(report, ensure_ascii=False), run_id),
            )
        return self.get(run_id)


class WorkQueue:
    """High-frequency state. Plan and committed outcomes can reconstruct every task."""

    def __init__(self, path: Path):
        path.parent.mkdir(parents=True, exist_ok=True)
        self.db = sqlite3.connect(path)
        self.db.row_factory = sqlite3.Row
        self.db.execute("PRAGMA journal_mode=WAL")
        self.db.execute("PRAGMA synchronous=FULL")
        self.db.executescript("""
          CREATE TABLE IF NOT EXISTS tasks(
            task_id TEXT PRIMARY KEY, item_json TEXT NOT NULL, state TEXT NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0, next_attempt_at REAL NOT NULL DEFAULT 0,
            result_json TEXT, error TEXT
          );
          CREATE TABLE IF NOT EXISTS attempts(
            task_id TEXT NOT NULL, attempt INTEGER NOT NULL, at TEXT NOT NULL,
            result_json TEXT NOT NULL, PRIMARY KEY(task_id,attempt)
          );
        """)

    def close(self):
        self.db.close()

    def restore(self, items, commits):
        with self.db:
            for item in items:
                self.db.execute(
                    "INSERT OR IGNORE INTO tasks(task_id,item_json,state) VALUES (?,?,'pending')",
                    (item["task_id"], json.dumps(item, ensure_ascii=False)),
                )
            # Leases belong to the previous process; ready spools are checked when claimed again.
            self.db.execute("UPDATE tasks SET state='pending' WHERE state IN ('working','ready')")
            for commit in commits:
                for result in commit["manifest"]["source"].get("ingest_results", []):
                    self.db.execute(
                        "UPDATE tasks SET state=?,result_json=?,error=?,next_attempt_at=? WHERE task_id=?",
                        (
                            result["status"],
                            json.dumps(result, ensure_ascii=False),
                            result.get("reason"),
                            result.get("next_attempt_at", 0),
                            result["task_id"],
                        ),
                    )
                    self.db.execute(
                        "UPDATE tasks SET attempts=max(attempts,?) WHERE task_id=?",
                        (result.get("claim_number", 0), result["task_id"]),
                    )

    def rows(self):
        return [dict(r) for r in self.db.execute("SELECT * FROM tasks ORDER BY rowid")]

    def claim(self, task_id):
        with self.db:
            self.db.execute(
                "UPDATE tasks SET state='working',attempts=attempts+1 WHERE task_id=?", (task_id,)
            )
        return self.db.execute("SELECT attempts FROM tasks WHERE task_id=?", (task_id,)).fetchone()[0]

    def attempted(self, task_id, attempt, result):
        with self.db:
            self.db.execute(
                "INSERT OR REPLACE INTO attempts VALUES (?,?,?,?)",
                (task_id, attempt, now(), json.dumps(result, ensure_ascii=False)),
            )
            self.db.execute(
                "UPDATE tasks SET state='ready',result_json=? WHERE task_id=?",
                (json.dumps(result, ensure_ascii=False), task_id),
            )

    def committed(self, results):
        with self.db:
            for result in results:
                self.db.execute(
                    "UPDATE tasks SET state=?,result_json=?,error=?,next_attempt_at=? WHERE task_id=?",
                    (
                        result["status"],
                        json.dumps(result, ensure_ascii=False),
                        result.get("reason"),
                        result.get("next_attempt_at", 0),
                        result["task_id"],
                    ),
                )
