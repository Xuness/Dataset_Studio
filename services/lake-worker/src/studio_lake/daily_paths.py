"""Stable main-library and SSD locations grouped by the run's Beijing creation date."""

from contextlib import closing
from datetime import datetime
import json
import re
import sqlite3
from zoneinfo import ZoneInfo

from .util import IntegrityError, atomic_json, now, safe_managed_path

DAILY_ZONE = ZoneInfo("Asia/Shanghai")
LAYOUT_DDL = """CREATE TABLE IF NOT EXISTS daily_run_layouts (
    run_id TEXT PRIMARY KEY REFERENCES daily_runs(run_id),
    day TEXT NOT NULL, ordinal INTEGER NOT NULL CHECK(ordinal>0),
    UNIQUE(day,ordinal)
)"""
KINDS = {"new_posts": "new-posts", "refresh": "refresh", "backfill": "backfill"}


def local_day(timestamp=None):
    value = datetime.fromisoformat(timestamp) if timestamp else datetime.now(DAILY_ZONE)
    if value.tzinfo is None:
        raise IntegrityError("日任务创建时间缺少时区")
    return value.astimezone(DAILY_ZONE).date().isoformat()


def checked_day(day):
    if not isinstance(day, str) or not re.fullmatch(r"\d{4}-\d{2}-\d{2}", day):
        raise IntegrityError("无效日期目录")
    datetime.strptime(day, "%Y-%m-%d")
    return day


def layout_rows(db):
    """Read-only preview works on v1 journals; assigned v2 ordinals never change."""
    if not db.execute("SELECT 1 FROM sqlite_master WHERE name='daily_runs' AND type='table'").fetchone():
        return {}
    assigned = {}
    if db.execute("SELECT 1 FROM sqlite_master WHERE name='daily_run_layouts' AND type='table'").fetchone():
        assigned = {
            r[0]: (r[1], r[2]) for r in db.execute("SELECT run_id,day,ordinal FROM daily_run_layouts")
        }
    next_number = {}
    for day, number in assigned.values():
        next_number[day] = max(next_number.get(day, 0), number)
    rows = {}
    for run_id, kind, created in db.execute(
        "SELECT run_id,kind,created_at FROM daily_runs ORDER BY created_at,run_id"
    ):
        if run_id in assigned:
            day, ordinal = assigned[run_id]
        else:
            day = local_day(created)
            ordinal = next_number.get(day, 0) + 1
            next_number[day] = ordinal
        checked_day(day)
        if kind not in KINDS or not isinstance(ordinal, int) or ordinal < 1:
            raise IntegrityError("日任务目录登记无效")
        rows[run_id] = {
            "day": day,
            "ordinal": ordinal,
            "kind": kind,
            "label": f"{ordinal:02d}-{KINDS[kind]}",
            "created_at": created,
        }
    return rows


def assign_layouts(db):
    db.execute(LAYOUT_DDL)
    rows = layout_rows(db)
    db.executemany(
        "INSERT OR IGNORE INTO daily_run_layouts(run_id,day,ordinal) VALUES (?,?,?)",
        [(run_id, row["day"], row["ordinal"]) for run_id, row in rows.items()],
    )
    return rows


def read_layouts(lib, *, refresh=False):
    if refresh or getattr(lib, "_daily_layouts", None) is None:
        with closing(sqlite3.connect((lib.root / "journal.sqlite").as_uri() + "?mode=ro", uri=True)) as db:
            lib._daily_layouts = layout_rows(db)
    return lib._daily_layouts


def day_directory(lib, day=None, *, reports=False):
    root = lib.root if reports else lib.cache
    parent = "ingest_runs" if reports else "daily"
    return safe_managed_path(root, root / parent / checked_day(day or local_day()))


def run_directory(lib, run_id, area, *, legacy=True):
    if not re.fullmatch(r"[a-f0-9]{32}", run_id) or area not in {"work", "diagnostics", "reports"}:
        raise ValueError("无效日任务目录参数")
    rows = read_layouts(lib)
    if run_id not in rows:
        rows = read_layouts(lib, refresh=True)
    if run_id not in rows:
        raise ValueError("日任务不存在")
    row = rows[run_id]
    root = lib.root if area == "reports" else lib.cache
    if area == "reports":
        target = safe_managed_path(root, day_directory(lib, row["day"], reports=True) / row["label"])
        previous = safe_managed_path(root, root / "ingest_runs" / run_id)
    else:
        target = safe_managed_path(root, day_directory(lib, row["day"]) / area / row["label"])
        previous = safe_managed_path(root, root / ("daily_" + area) / run_id)
    if legacy and previous.exists():
        if target.exists():
            raise IntegrityError("新旧日任务目录同时存在，请先检查目录整理报告")
        return previous
    return target


def check_run_owner(lib, path, run_id, area):
    marker = safe_managed_path(lib.root if area == "reports" else lib.cache, path / "owner.json")
    expected = {"layout_version": 1, "library_id": lib.info["library_id"], "run_id": run_id, "area": area}
    if not marker.is_file() or json.loads(marker.read_text(encoding="utf-8")) != expected:
        raise IntegrityError("日期工作目录缺少匹配的任务归属记录")


def write_run_owner(lib, path, run_id, area):
    marker = safe_managed_path(lib.root if area == "reports" else lib.cache, path / "owner.json")
    if marker.exists():
        check_run_owner(lib, path, run_id, area)
    else:
        atomic_json(
            marker,
            {"layout_version": 1, "library_id": lib.info["library_id"], "run_id": run_id, "area": area},
        )


def ensure_run_directory(lib, run_id, area):
    target = run_directory(lib, run_id, area)
    previous_parent = lib.root / "ingest_runs" if area == "reports" else lib.cache / ("daily_" + area)
    if target.parent == previous_parent:
        return target
    if not target.exists():
        target.mkdir(parents=True)
    existing = list(target.iterdir())
    if not existing or all(re.fullmatch(r"owner\.json\.[a-f0-9]{32}\.tmp", p.name) for p in existing):
        # mkdir may have completed just before an interrupted owner-marker write.
        write_run_owner(lib, target, run_id, area)
    else:
        check_run_owner(lib, target, run_id, area)
    return target


def write_day_summary(lib, day):
    rows = read_layouts(lib, refresh=True)
    selected = {key: value for key, value in rows.items() if value["day"] == day}
    if not selected:
        return
    with closing(sqlite3.connect((lib.root / "journal.sqlite").as_uri() + "?mode=ro", uri=True)) as db:
        tasks = []
        for run_id, layout in selected.items():
            state, result, error = db.execute(
                "SELECT state,result_json,error FROM daily_runs WHERE run_id=?", (run_id,)
            ).fetchone()
            outcome = json.loads(result or "{}")
            tasks.append(
                {
                    "task": layout["label"],
                    "run_id": run_id,
                    "kind": layout["kind"],
                    "created_at": layout["created_at"],
                    "state": state,
                    "error": error,
                    "counts": {
                        k: outcome[k]
                        for k in ("planned", "stored", "reused", "unavailable", "failed", "pending")
                        if k in outcome
                    },
                    "work_directory": str(run_directory(lib, run_id, "work")),
                    "diagnostics_directory": str(run_directory(lib, run_id, "diagnostics")),
                    "report_directory": str(run_directory(lib, run_id, "reports")),
                }
            )
    summary = {
        "layout_version": 1,
        "library_id": lib.info["library_id"],
        "day": day,
        "timezone": "Asia/Shanghai",
        "updated_at": now(),
        "tasks": tasks,
    }
    for reports in (False, True):
        atomic_json(day_directory(lib, day, reports=reports) / "summary.json", summary)
