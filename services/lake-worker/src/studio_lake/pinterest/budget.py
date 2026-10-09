"""Per-round counters and bounded admission, independent of unrelated source budgets."""

import json

from . import model, planner


def usage(db, job):
    result = {k: job[k] for k in ("api_requests", "detail_requests", "download_bytes", "elapsed_seconds")}
    result.update({"admitted_" + ("pins" if r[0] == "pin" else "boards"): r[1]
        for r in db.execute("SELECT kind,n FROM pinterest_admitted_counts WHERE job_id=? AND kind IN ('pin','board')", (job["id"],))})
    result.update({r[0]: r[1] for r in db.execute("SELECT name,value FROM pinterest_metrics WHERE job_id=? AND name LIKE '%:requests'", (job["id"],))})
    return result


def reason(db, job, task, spec):
    baseline = json.loads(job["budget_baseline_json"])
    values = usage(db, job)
    limits = spec["run_budget"]
    def spent(name):
        return values.get(name, 0) - baseline.get(name, 0)
    kind, entry = task["kind"], json.loads(task["input_json"])
    if kind in model.NETWORK_KINDS:
        if spent("api_requests") >= limits["api_requests"]:
            return "api_requests"
        if kind in ("pin_detail", "pin_enrichment") and spent("detail_requests") >= limits["detail_requests"]:
            return "detail_requests"
        maximum = spec.get("discovery", {}).get("entry_requests", {}).get(kind, 100)
        if kind in model.PAGE_KINDS and spent(kind + ":requests") >= maximum:
            return "entry_requests:" + kind
    if kind == "media_download" and spent("download_bytes") >= limits["download_bytes"]:
        return "download_bytes"
    if kind in ("pin_detail", "pin_admit") and not planner.admitted(db, job["id"], "pin", task["pin_id"]):
        if spent("admitted_pins") >= limits["admitted_pins"]:
            return "admitted_pins"
    if kind == "board_admit" and entry["source_kind"] == "board" and not planner.admitted(db, job["id"], "board", task["pin_id"]):
        if spent("admitted_boards") >= limits["admitted_boards"]:
            return "admitted_boards"
    if kind in model.PAGE_KINDS:
        pending = db.execute("SELECT coalesce(sum(n),0) FROM pinterest_counts WHERE job_id=? AND kind IN ('media_download','pin_admit') AND state IN ('queued','running','waiting_retry','waiting_budget')", (job["id"],)).fetchone()[0]
        if pending >= spec.get("discovery", {}).get("max_pending_downloads", 128):
            return "discovery_backlog"
    return None


def request(db, job_id, kind, scan_id=None):
    db.execute("UPDATE pinterest_jobs SET api_requests=api_requests+1,detail_requests=detail_requests+? WHERE id=?",
               (int(kind in ("pin_detail", "pin_enrichment")), job_id))
    db.execute("INSERT INTO pinterest_metrics VALUES(?,?,1) ON CONFLICT(job_id,name) DO UPDATE SET value=value+1",
               (job_id, kind + ":requests"))
    if kind in ("pin_detail", "pin_enrichment") and scan_id:
        row = db.execute("SELECT entrypoint FROM pinterest_streams WHERE scan_id=? AND job_id=?", (scan_id, job_id)).fetchone()
        if row:
            db.execute("INSERT INTO pinterest_metrics VALUES(?,?,1) ON CONFLICT(job_id,name) DO UPDATE SET value=value+1",
                       (job_id, row[0] + ":detail_requests"))
