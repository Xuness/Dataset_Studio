"""Opt-in real-lake/model integration probe. Writes only its own engine/materials.

The registry argument is read-only; registered Booru media is read in bounded
batches through the production engine. No collection, frontend or full-lake run.
"""
import argparse
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import time
import urllib.error
import urllib.request


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", required=True)
    parser.add_argument("--registry", required=True)
    parser.add_argument("--models", required=True)
    parser.add_argument("--python", required=True)
    parser.add_argument("--materials", required=True)
    parser.add_argument("--output-root", required=True)
    parser.add_argument("--limit", type=int, default=128)
    parser.add_argument("--resume-run", help="Resume this explicitly named probe run after a repaired failure")
    args = parser.parse_args()
    root = Path(args.output_root).resolve()
    app = root / "engine"
    root.mkdir(parents=True, exist_ok=True)
    app.mkdir(exist_ok=True)
    source_db = sqlite3.connect(Path(args.registry).resolve().as_uri() + "?mode=ro", uri=True)
    sources = [json.loads(r[0]) for r in source_db.execute("SELECT json FROM source_locations")]
    source_db.close()
    sources = [s for s in sources if s["kind"] in ("danbooru", "yandere", "gelbooru")]
    assert len(sources) == 3, "probe requires exactly three registered Booru lakes"
    log = (root / "engine.log").open("a", encoding="utf-8")
    command = [args.engine, "serve", "--data-dir", str(app)]
    process = subprocess.Popen(command, stdout=log, stderr=log,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    connection = None
    report = {"source_ids": [s["id"] for s in sources], "limit_per_source": args.limit}

    def call(path, body=None, method=None):
        request = urllib.request.Request(connection["endpoint"] + path,
            data=None if body is None else json.dumps(body).encode(),
            headers={"Authorization": "Bearer " + connection["token"], "Content-Type": "application/json"},
            method=method or ("GET" if body is None else "POST"))
        try:
            with urllib.request.urlopen(request, timeout=900) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            raise RuntimeError(error.read().decode()) from error

    def wait_run(identity):
        deadline = time.monotonic() + 1800
        last = None
        while time.monotonic() < deadline:
            run = call(f"/v1/visual/runs/{identity}")
            if run["status"] != last:
                (root / "progress.json").write_text(json.dumps(run, ensure_ascii=False, indent=2), encoding="utf-8")
                last = run["status"]
            if run["status"] in ("paused", "failed", "completed", "completed_with_errors", "cancelled"):
                while call("/v1/visual/runtime")["busy"]:
                    time.sleep(0.1)
                return run
            time.sleep(0.5)
        raise TimeoutError("visual run did not reach a boundary")

    def model_counts(identity):
        with sqlite3.connect(Path(args.materials).resolve().joinpath("visual.sqlite").as_uri() + "?mode=ro", uri=True) as db:
            receipts = [json.loads(r[0]) for r in db.execute("SELECT receipt FROM batches WHERE run=? AND state='published'", (identity,))]
        result = {}
        for receipt in receipts:
            for rid, n in receipt.get("model_images", {}).items():
                result[rid] = result.get(rid, 0) + n
        return result

    try:
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise RuntimeError("probe engine exited; see engine.log")
            try:
                connection = json.loads((app / "engine.json").read_text(encoding="utf-8"))
                if connection["pid"] == process.pid:
                    call("/v1/health")
                    break
            except (OSError, ValueError, RuntimeError):
                pass
            time.sleep(0.1)
        else:
            raise TimeoutError("engine startup")
        # Only the isolated probe registry is edited. Its public source service
        # uses the same identities and roots as the read-only original registry.
        with sqlite3.connect(app / "registry.sqlite") as db:
            for source in sources:
                db.execute("INSERT OR REPLACE INTO source_locations(id,json) VALUES (?,?)", (source["id"], json.dumps(source)))
        runtime = call("/v1/visual/runtime", {"python": str(Path(args.python).resolve()),
            "materials_root": str(Path(args.materials).resolve()), "model_root": str(Path(args.models).resolve()), "device": "cuda"}, "PUT")
        recipes = runtime["description"]["recipes"]
        report["runtime"] = runtime
        definition = {"source_ids": [s["id"] for s in sources], "recipe_ids": [r["id"] for r in recipes],
                      "max_objects_per_source": args.limit, "batch_size": 32}
        created = call(f"/v1/visual/runs/{args.resume_run}/actions", {"action": "resume"}) if args.resume_run else call("/v1/visual/runs", definition)
        call(f"/v1/visual/runs/{created['id']}/actions", {"action": "pause"})
        paused = wait_run(created["id"])
        assert paused["status"] == "paused", paused
        call(f"/v1/visual/runs/{created['id']}/actions", {"action": "resume"})
        started = time.monotonic()
        completed = wait_run(created["id"])
        report["first_run"] = completed
        report["elapsed_seconds"] = round(time.monotonic() - started, 3)
        assert completed["status"] in ("completed", "completed_with_errors"), completed
        report["first_model_images"] = model_counts(created["id"])
        coverage = call("/v1/visual/coverage")["items"]
        report["coverage"] = coverage
        for source in sources:
            for recipe in recipes:
                row = next(c for c in coverage if c["source_id"] == source["id"] and c["recipe_id"] == recipe["id"])
                assert row["succeeded"] > 0, (source["kind"], recipe["model"], row)
        failures = call(f"/v1/visual/runs/{created['id']}/failures")["items"]
        report["failures"] = failures
        allowed = ("unsupported_animation", "pixel_limit", "compressed_byte_limit", "cannot identify image file")
        assert all(any((f["error"] or "").startswith(code) for code in allowed) for f in failures), failures
        report["searches"] = []
        with sqlite3.connect(Path(args.materials).resolve().joinpath("visual.sqlite").as_uri() + "?mode=ro", uri=True) as db:
            for recipe in recipes:
                source_id, sha = db.execute("SELECT s.source_id,o.sha256 FROM outcomes c JOIN objects o ON o.id=c.object JOIN bindings b ON b.object=o.id JOIN sources s ON s.id=b.source WHERE c.recipe=? AND c.status='success' ORDER BY o.id LIMIT 1", (recipe["id"],)).fetchone()
                query = {"recipe_id": recipe["id"], "asset": {"source_id": source_id, "asset_id": sha},
                         "source_ids": [sources[0]["id"]], "limit": 10, "exact": True}
                exact = call("/v1/visual/search", query)
                assert exact["items"] and exact["method"] == "exact"
                assert all(a["source_id"] == sources[0]["id"] for item in exact["items"] for a in item["assets"])
                query["source_ids"] = []
                self_result = call("/v1/visual/search", query)
                assert self_result["items"][0]["distance"] < 1e-5
                count = db.execute("SELECT count(*) FROM outcomes WHERE recipe=? AND status='success'", (recipe["id"],)).fetchone()[0]
                if count >= 256:
                    indexed = call("/v1/visual/indexes", {"recipe_id": recipe["id"], "partitions": 4})
                    ann = call("/v1/visual/search", {**query, "exact": False, "nprobes": 4})
                    assert ann["method"] == "ivf_flat"
                    assert abs(ann["items"][0]["distance"] - self_result["items"][0]["distance"]) < 1e-5
                else:
                    indexed = None
                report["searches"].append({"model": recipe["model"], "dimension": recipe["dimension"], "self_distance": self_result["items"][0]["distance"], "index": indexed})
            phash = call("/v1/visual/duplicates", {"asset": {"source_id": source_id, "asset_id": sha}, "limit": 10, "radius": 7})
            report["phash_query"] = {"kind": phash["kind"], "items": len(phash["items"])}
        repeated = call("/v1/visual/runs", definition)
        repeated = wait_run(repeated["id"])
        assert repeated["status"] in ("completed", "completed_with_errors"), repeated
        report["repeat_run"] = repeated
        report["repeat_model_images"] = model_counts(repeated["id"])
        assert not any(report["repeat_model_images"].values()), report["repeat_model_images"]
        report["pause_resume"] = True
        report["passed"] = True
    finally:
        (root / "result.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
        if process.poll() is None:
            try:
                call("/v1/shutdown", {}, "POST")
                process.wait(timeout=30)
            except Exception:
                process.kill()
                process.wait()
        log.close()
    print(json.dumps({"passed": report.get("passed", False), "result": str(root / "result.json")}, ensure_ascii=False))


if __name__ == "__main__":
    main()
