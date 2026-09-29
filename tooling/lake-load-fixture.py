"""Offline load fixture: full online snapshots, selected media, isolated synthetic updates.

No live lake is registered, written, or locked. This is a serving/load mirror, not
an archive backup: only the selected media packs accompany full online indexes.
"""

import json
import shutil
import sqlite3
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from contextlib import closing
from datetime import datetime, timezone
from pathlib import Path

repo = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(repo / "services/lake-worker/src"))
sys.path.insert(0, str(repo / "services/lake-worker/tests"))
import apsw
from conftest import png
from studio_lake.config import Config
from studio_lake.library import Library
from studio_lake.online_schema import settings
from studio_lake.updates.resources import Resources
from studio_lake.updates.runner import Runner
from studio_lake.updates.state import State
from studio_lake.util import atomic_json, contained
from test_updates import FakeSite, Images, job, post

root = Path(sys.argv[1]).resolve()
if root.parent != (repo / ".local/test-runs").resolve() or not root.name.startswith("astra-load-"):
    raise RuntimeError("Only a dedicated astra-load-* test directory is accepted")
root.mkdir(parents=True, exist_ok=True)


def backup(source, target):
    target.parent.mkdir(parents=True, exist_ok=True)
    with apsw.Connection(str(source), flags=apsw.SQLITE_OPEN_READONLY) as src, apsw.Connection(str(target)) as dst:
        src.set_busy_timeout(5000)
        started = time.monotonic()
        with dst.backup("main", src, "main") as copy:
            while not copy.done:
                copy.step(4096)
                if time.monotonic() - started > 1800:
                    raise RuntimeError("Snapshot exceeded 30 minute bound")
    return round(time.monotonic() - started, 3)


def prepare():
    sources = json.loads(Path(sys.argv[3]).read_text(encoding="utf-8-sig"))
    targets = []
    for source in sources:
        site = source["kind"]
        media, index = root / site / "media", root / site / "index"
        source_media, source_index = Path(source["media_root"]), Path(source["index_root"])
        pointer = json.loads((source_index / "ONLINE.json").read_text())
        media.mkdir(parents=True, exist_ok=True)
        index.mkdir(parents=True, exist_ok=True)
        atomic_json(index / "ONLINE.json", pointer)
        seconds = backup(contained(source_index, pointer["file"]), contained(index, pointer["file"]))
        shutil.copy2(source_media / "library.json", media / "library.json")
        backup(source_media / "journal.sqlite", media / "journal.sqlite")
        for name in ("segments", "staging", "plans", "publish", "releases", "source_manifests"):
            (media / name).mkdir(exist_ok=True)
        if (source_media / "source_manifests").is_dir():
            shutil.copytree(source_media / "source_manifests", media / "source_manifests", dirs_exist_ok=True)
        atomic_json(index / "cache_owner.json", {"library_id": pointer["library_id"], "root": str(media)})
        atomic_json(media / "online-index.json", {"schema_version": 2, "library_id": pointer["library_id"], "index_root": str(index)})
        with apsw.Connection(str(contained(index, pointer["file"]))) as db:
            status = settings(db)
            count = next(db.execute("SELECT count(*) FROM objects"))[0]
            observations = next(db.execute("SELECT count(*) FROM observations"))[0]
            sample = list(db.execute("SELECT sha256,pack_path FROM objects WHERE length>0 AND length<4194304 AND stored_ext IN ('jpg','jpeg','png','webp') ORDER BY object_row LIMIT 64"))
            head = next(db.execute("SELECT coalesce(max(post_id),0) FROM post_versions"))[0]
        # Both backups were read-only and can have different instants. The test
        # journal ends at the projection watermark; new fixture commits follow it.
        with closing(sqlite3.connect(media / "journal.sqlite")) as journal, journal:
            journal.execute("DELETE FROM commits WHERE seq>?", (int(status["served_seq"]),))
            journal.execute("UPDATE sqlite_sequence SET seq=? WHERE name='commits'", (int(status["served_seq"]),))
        for relative in sorted({pack for _, pack in sample}):
            destination = contained(media, relative)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(contained(source_media, relative), destination)
        target = {"library_id": pointer["library_id"], "site": site, "media_root": str(media),
                  "index_root": str(index)}
        State(root / "controller").register(target)
        targets.append({**target, "objects": count, "observations": observations, "served_seq": int(status["served_seq"]),
                        "index_bytes": contained(index, pointer["file"]).stat().st_size,
                        "copy_seconds": seconds, "sample": [sha for sha, _ in sample], "post_head": head})
        atomic_json(root / "targets.json", targets)
        print(json.dumps({"site": site, "objects": count, "observations": observations, "copy_seconds": seconds}), flush=True)


def updates():
    targets = json.loads((root / "targets.json").read_text())
    state = State(root / "controller")
    data = png("red")

    class SlowSite(FakeSite):
        def capabilities(self):
            return {**super().capabilities(), "page_size": 10}

        def request(self, params, cancelled=lambda: False, resource="posts"):
            if resource == "posts":
                time.sleep(0.1)
            return super().request(params, cancelled, resource)

    resources = Resources(reserve_bytes=0)
    sequence = int(sys.argv[3]) if len(sys.argv) > 3 else 1
    records = {}
    tasks = []
    for target in targets:
        index = Path(target["index_root"])
        pointer = json.loads((index / "ONLINE.json").read_text())
        with apsw.Connection(str(contained(index, pointer["file"])), flags=apsw.SQLITE_OPEN_READONLY) as source:
            source.set_busy_timeout(5000)
            first = next(source.execute("SELECT coalesce(max(post_id),0) FROM post_versions"))[0] + 10000
        records[target["site"]] = SlowSite(target["site"], [post(target["site"], n, data) for n in range(first, first + 400)])
        lib = Library(Config(Path(target["media_root"]), Path(target["index_root"])))
        tasks.append(job(state, lib, {"kind": "id_range", "start": first, "end": first + 400}, "original"))
    runner = Runner(state, records, resources, Images(data))
    atomic_json(root / f"update-tasks-{sequence}.json", [{"id": t["id"], "lake_id": t["lake_id"]} for t in tasks])
    started = time.monotonic()
    samples = []
    def memory():
        import os
        if os.name != "nt":
            return None
        import ctypes
        from ctypes import wintypes
        class Counters(ctypes.Structure):
            _fields_ = [("cb", wintypes.DWORD), ("faults", wintypes.DWORD)] + [
                (name, ctypes.c_size_t) for name in ("peak_working", "working", "peak_paged", "paged",
                                                  "peak_nonpaged", "nonpaged", "pagefile", "peak_pagefile", "private")]
        value = Counters()
        value.cb = ctypes.sizeof(value)
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.GetCurrentProcess.restype = wintypes.HANDLE
        psapi = ctypes.WinDLL("psapi", use_last_error=True)
        psapi.GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.c_void_p, wintypes.DWORD]
        if psapi.GetProcessMemoryInfo(kernel.GetCurrentProcess(), ctypes.byref(value), value.cb):
            return {"working_set_bytes": value.working, "private_bytes": value.private}
        return None
    # Exercise the production coordinator's per-lake dispatch and accounting.
    with ThreadPoolExecutor(max_workers=1) as pool:
        future = pool.submit(runner.serve)
        try:
            while time.monotonic() - started < 600:
                rows = [state.job(t["id"]) for t in tasks]
                with state.db() as db:
                    transaction_at = time.perf_counter()
                    db.execute("BEGIN IMMEDIATE")
                    transaction_wait_ms = (time.perf_counter() - transaction_at) * 1000
                with resources.condition:
                    actual_started = time.perf_counter()
                    sizes = resources._actual()
                    scan_ms = (time.perf_counter() - actual_started) * 1000
                    actual, reserved = sum(sizes.values()), sum(resources.reservations.values())
                watermarks = {}
                for target in targets:
                    index = Path(target["index_root"])
                    pointer = json.loads((index / "ONLINE.json").read_text())
                    with apsw.Connection(str(contained(index, pointer["file"])), flags=apsw.SQLITE_OPEN_READONLY) as db:
                        db.set_busy_timeout(500)
                        try:
                            values = settings(db)
                            watermarks[target["site"]] = {k: int(values[k]) for k in ("archive_seq", "served_seq")}
                        except apsw.BusyError:
                            # A diagnostic sampler cannot cancel the workload it observes.
                            watermarks[target["site"]] = {"sample_busy": True}
                waits = [(datetime.now(timezone.utc) - datetime.fromisoformat(r["created_at"])).total_seconds()
                         for r in rows if r["state"] == "queued"]
                samples.append({"at_ms": int(time.time()*1000), "seconds": time.monotonic()-started,
                                "states": [r["state"] for r in rows], "memory": memory(),
                                "oldest_queued_seconds": max(waits, default=0),
                                "actual_bytes": actual, "reserved_bytes": reserved, "scan_ms": scan_ms,
                                "control_transaction_wait_ms": transaction_wait_ms,
                                "watermarks": watermarks})
                if all(r["state"] in {"completed", "completed_with_exclusions", "needs_review"} for r in rows):
                    break
                time.sleep(0.5)
        finally:
            runner.stop.set()
            future.result(timeout=90)
    rows = [state.job(t["id"]) for t in tasks]
    atomic_json(root / f"updates-result-{sequence}.json", {"seconds": time.monotonic()-started, "jobs": rows, "samples": samples})
    if any(r["state"] != "completed" for r in rows):
        raise RuntimeError("Load update failed; inspect updates-result.json")


def spool():
    samples = {}
    resources = Resources(spool_bytes=1024**3, reserve_bytes=0)
    base = root / "spool-probe"
    for count in (100, 1000, 10000):
        parent = base / str(count)
        directory = parent / ("a" * 32)
        directory.mkdir(parents=True, exist_ok=True)
        for number in range(count):
            (directory / (f"{number:064x}" + ".partial")).write_bytes(b"x" * 32)
        resources.roots = {parent}
        elapsed = []
        for _ in range(10):
            with resources.condition:
                at = time.perf_counter()
                resources._actual()
                elapsed.append((time.perf_counter()-at)*1000)
        samples[str(count)] = elapsed
    atomic_json(root / "spool-scan.json", samples)


{"prepare": prepare, "updates": updates, "spool": spool}[sys.argv[2]]()
