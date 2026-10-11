"""Opt-in visual index lifecycle/capacity probe in a fresh, isolated test root.

Uses synthetic full-dimensional vectors and real engine/worker/index processes.
No lake is opened and no model inference is performed. Installed local models
are used only to obtain a supported immutable recipe description.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import time
import urllib.error
import urllib.request

import numpy as np

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "services/visual-worker/src"))
from studio_visual.models import normalized
from studio_visual.vectors import Vectors


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", required=True)
    parser.add_argument("--python", required=True)
    parser.add_argument("--models", required=True)
    parser.add_argument("--output-root", required=True)
    parser.add_argument("--rows", type=int, default=20000)
    parser.add_argument("--dimension", type=int, choices=(256, 768, 1152), default=1152)
    parser.add_argument("--capacity-only", action="store_true")
    args = parser.parse_args()
    if not 1024 <= args.rows <= 2_000_000:
        raise ValueError("capacity fixture requires 1024-2000000 rows")
    root = Path(args.output_root).resolve()
    if not root.is_relative_to(ROOT / ".local/test-runs"):
        raise ValueError("output must be a fresh .local/test-runs directory")
    root.mkdir(parents=True, exist_ok=False)
    app, materials = root / "engine", root / "features"
    app.mkdir()
    materials.mkdir()
    binary = root / Path(args.engine).name
    shutil.copyfile(Path(args.engine).resolve(), binary)
    cache = ROOT.parent.parent / "Visual Features/model-digests.json"
    if cache.is_file():
        shutil.copyfile(cache, materials / "model-digests.json")
    log = (root / "engine.log").open("a", encoding="utf-8")
    with binary.open("rb") as stream:
        fingerprint = hashlib.file_digest(stream, "sha256").hexdigest()
    report = {"synthetic": True, "rows": args.rows, "dimension": args.dimension,
              "engine_sha256": fingerprint}
    process = None
    connection = None

    def call(path, body=None, method=None, timeout=120):
        req = urllib.request.Request(connection["endpoint"] + path,
            data=None if body is None else json.dumps(body).encode(),
            headers={"Authorization": "Bearer " + connection["token"], "Content-Type": "application/json"},
            method=method or ("GET" if body is None else "POST"))
        try:
            with urllib.request.urlopen(req, timeout=timeout) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            raise RuntimeError(error.read().decode()) from error

    def start():
        nonlocal process, connection
        process = subprocess.Popen([str(binary), "serve", "--data-dir", str(app)],
            cwd=ROOT, stdout=log, stderr=log, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise RuntimeError("isolated engine exited; see engine.log")
            try:
                candidate = json.loads((app / "engine.json").read_text(encoding="utf-8"))
                if candidate["pid"] == process.pid:
                    connection = candidate
                    call("/v1/health")
                    return
            except (OSError, ValueError, RuntimeError):
                pass
            time.sleep(.05)
        raise TimeoutError("isolated engine startup")

    def stop():
        if process and process.poll() is None:
            call("/v1/shutdown", {})
            process.wait(timeout=40)

    def index_state(identity):
        return call("/v1/visual/indexes/" + identity)

    def observe(identity, terminal=False):
        deadline = time.monotonic() + (3600 if terminal else 180)
        phases = []
        while time.monotonic() < deadline:
            job = index_state(identity)
            phase = job["progress"]["phase"]
            if not phases or phases[-1] != phase:
                phases.append(phase)
            (root / "progress.json").write_text(json.dumps(job, indent=2), encoding="utf-8")
            if job["status"] in ("completed", "failed", "cancelled", "paused"):
                while call("/v1/visual/runtime")["busy"]:
                    time.sleep(.05)
                return job, phases
            if not terminal and job["worker_running"] and phase not in ("queued", "compacting", "scalar_indexes"):
                return job, phases
            time.sleep(.03 if not terminal else .25)
        raise TimeoutError("index observation timed out; task is not restarted automatically")

    def submit(recipe):
        return call("/v1/visual/indexes", {"recipe_id": recipe["id"], "max_iterations": 30, "sample_rate": 128})

    try:
        start()
        configured = call("/v1/visual/runtime", {"python": str(Path(args.python).resolve()),
            "materials_root": str(materials), "model_root": str(Path(args.models).resolve()), "device": "cpu"}, "PUT")
        recipe = next(r for r in configured["description"]["recipes"] if r["dimension"] == args.dimension)
        report["recipe_id"] = recipe["id"]
        stop()
        assert process.poll() is not None, "fixture may only be written with its engine stopped"
        vectors = Vectors(materials, cache_bytes=64 << 20)
        rng = np.random.default_rng(109)
        centers = normalized(rng.standard_normal((128, args.dimension)), args.dimension)
        query_vectors = []
        with sqlite3.connect(materials / "visual.sqlite") as db:
            for n in range(1, 4):
                db.execute("INSERT INTO sources(id,source_id) VALUES (?,?)", (n, f"synthetic-{n}"))
            for start_row in range(0, args.rows, 8192):
                ids = np.arange(start_row + 1, min(args.rows, start_row + 8192) + 1)
                values = normalized(centers[ids % len(centers)] + rng.standard_normal((len(ids), args.dimension)).astype(np.float32) * .012, args.dimension)
                if not query_vectors:
                    query_vectors = values[:16].tolist()
                rows = [{"object_id": int(n), "source_ids": [1 + int(n) % 3]} for n in ids]
                vectors.append(recipe, f"fixture-{start_row}", rows, values)
                db.executemany("INSERT INTO objects(id,sha256,verified) VALUES (?,?,1)", ((int(n), f"{int(n):064x}") for n in ids))
                db.executemany("INSERT INTO bindings(source,object) VALUES (?,?)", ((1 + int(n) % 3, int(n)) for n in ids))
                db.executemany("INSERT INTO outcomes VALUES (?,?,'success','fixture',NULL)", ((int(n), recipe["id"]) for n in ids))
                db.commit()
            version = vectors.project_sources(recipe, [{"object_id": 1, "source_ids": [2]}])
            db.execute("INSERT INTO versions VALUES (?,?)", (recipe["id"], version))
            db.execute("UPDATE sources SET discovered=(SELECT count(*) FROM bindings b WHERE b.source=sources.id)")
            db.execute("INSERT INTO coverage_counts SELECT id,?,discovered,0 FROM sources", (recipe["id"],))
        del vectors
        start()
        baseline = call("/v1/visual/snapshots", {"name": "Synthetic baseline", "recipe_ids": [recipe["id"]]})
        query = {"recipe_id": recipe["id"], "vector": query_vectors[0], "limit": 20, "exact": True}
        exact = call("/v1/visual/search", query)
        assert len(exact["items"]) == 20 and exact["items"][0]["distance"] < 1e-5
        index = submit(recipe)
        if not args.capacity_only:
            live, phases = observe(index["id"])
            assert live["status"] == "running" and live["worker_running"], live
            during = call("/v1/visual/search", {**query, "snapshot_id": baseline["id"]})
            assert during["version"] == version
            call(f"/v1/visual/indexes/{index['id']}/actions", {"action": "pause"})
            paused, _ = observe(index["id"], terminal=True)
            assert paused["status"] == "paused" and not paused["worker_running"]
            report["pause"] = {"phase": live["progress"], "phases": phases, "paused": paused}
            call(f"/v1/visual/indexes/{index['id']}/actions", {"action": "resume"})
        completed, phases = observe(index["id"], terminal=True)
        assert completed["status"] == "completed", completed
        report["completed"] = completed
        report["index_phases"] = phases
        result = completed["result"]
        assert result["indexed_rows"] == args.rows and result["unindexed_rows"] == 0
        all_parts = call("/v1/visual/search", {**query, "exact": False, "nprobes": result["partitions"]})
        assert {i["object_id"] for i in all_parts["items"]} == {i["object_id"] for i in exact["items"]}
        if not args.capacity_only:
            cancelled_job = submit(recipe)
            live, _ = observe(cancelled_job["id"])
            assert live["status"] == "running" and live["worker_running"]
            call(f"/v1/visual/indexes/{cancelled_job['id']}/actions", {"action": "cancel"})
            cancelled, _ = observe(cancelled_job["id"], terminal=True)
            assert cancelled["status"] == "cancelled" and not cancelled["worker_running"]
            assert call("/v1/visual/search", query)["version"] == result["version"]
            report["cancel"] = cancelled
            restart_job = submit(recipe)
            live, _ = observe(restart_job["id"])
            assert live["status"] == "running" and live["worker_running"]
            stop()
            start()
            restored = index_state(restart_job["id"])
            assert restored["status"] == "paused" and not restored["worker_running"]
            call(f"/v1/visual/indexes/{restart_job['id']}/actions", {"action": "resume"})
            restored, _ = observe(restart_job["id"], terminal=True)
            assert restored["status"] == "completed", restored
            report["restart"] = restored
        timings, recalls = [], []
        for vector in query_vectors:
            expected = call("/v1/visual/search", {**query, "vector": vector})
            started = time.perf_counter()
            actual = call("/v1/visual/search", {**query, "vector": vector, "exact": False, "nprobes": 32})
            timings.append((time.perf_counter() - started) * 1000)
            recalls.append(len({i["object_id"] for i in expected["items"]} & {i["object_id"] for i in actual["items"]}) / 20)
        report["query"] = {"count": len(timings), "nprobes": 32, "latency_ms": timings,
            "p50_ms": float(np.percentile(timings, 50)), "p95_ms": float(np.percentile(timings, 95)),
            "mean_recall_at_20": float(np.mean(recalls)), "recall_at_20": recalls}
        report["retention_preview"] = call("/v1/visual/retention/cleanup", {})
        report["retention_apply"] = call("/v1/visual/retention/cleanup", {"apply": True})
        old = call("/v1/visual/search", {**query, "snapshot_id": baseline["id"]})
        assert old["version"] == baseline["versions"][recipe["id"]]
        report["passed"] = True
    finally:
        (root / "result.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
        if process and process.poll() is None:
            try:
                stop()
            except Exception:
                process.kill()
                process.wait()
        log.close()
    print(json.dumps({"passed": report.get("passed", False), "rows": args.rows, "result": str(root / "result.json")}))


if __name__ == "__main__":
    main()
