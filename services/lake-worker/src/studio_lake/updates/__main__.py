"""Versioned stdin/stdout control transport. No credentials in argv or logs."""

import argparse
import json
from pathlib import Path
import signal
import sys
import threading
import time

from . import PROTOCOL_VERSION
from .protocol import definition, validate_site
from .runner import Runner
from .sites import Site, UpdateError
from .state import State
from .read_model import activity


def dispatch(state, command, args):
    if not isinstance(args, dict):
        raise UpdateError("INVALID_INPUT", "Command arguments must be an object")
    if command == "status":
        heartbeat = state.root / "heartbeat.json"
        info = json.loads(heartbeat.read_text(encoding="utf-8")) if heartbeat.exists() else None
        return {
            "protocol_version": PROTOCOL_VERSION,
            "worker_recent": bool(info and time.time() - heartbeat.stat().st_mtime < 10),
            "heartbeat": info,
            "credentials": state.credential_status(),
            "activity": activity(state),
        }
    if command == "capabilities":
        return {"items": [Site(name, state.credentials(name)).capabilities() for name in Site.URLS]}
    if command == "register":
        return state.register(args)
    if command == "input_create":
        return state.create_input(**args)
    if command == "input":
        return state.input(args["id"])
    if command == "input_append":
        return state.append_input(args["id"], args.get("post_ids"), args.get("object_sha256s"))
    if command == "input_append_batch":
        objects = args.get("object_sha256s")
        if not isinstance(objects, list) or not 1 <= len(objects) <= 4096:
            raise UpdateError("INVALID_INPUT", "Fixed member preparation accepts 1–4096 object hashes")
        result = None
        for offset in range(0, len(objects), 128):
            result = state.append_input(args["id"], object_sha256s=objects[offset:offset + 128])
        return result
    if command == "input_seal":
        return state.seal_input(args["id"])
    if command == "lakes":
        with state.db() as db:
            return {"items": [dict(r) for r in db.execute("SELECT * FROM lakes ORDER BY id LIMIT 1000")]}
    if command == "credential_set":
        return state.set_credentials(args["site"], args["value"])
    if command == "credential_delete":
        if args["site"] not in Site.URLS:
            raise UpdateError("INVALID_INPUT", "Unknown site")
        with state.db() as db:
            db.execute("DELETE FROM credentials WHERE site=?", (args["site"],))
        return {"site": args["site"], "credential_set": False}
    if command == "probe":
        site = Site(args["site"], state.credentials(args["site"]), rate_root=state.root)
        try:
            params = {"limit": 1}
            if site.name == "gelbooru":
                params.update(page="dapi", s="post", q="index", json=1)
            response = site.request(params)
            rows = site.parse(response)
            if rows:
                expected = rows[0][1]["id"]
                control = site.parse(site.request(site.params(expected, expected + 1, 1)))
                if [r[1]["id"] for r in control] != [expected]:
                    raise UpdateError("UPDATE_PAGE_INVALID", "Positive-control ID range probe failed")
            return {
                "site": site.name,
                "status": response.status,
                "records": len(rows),
                "range_verified": bool(rows),
                "fields": sorted(set().union(*(r[1].keys() for r in rows))) if rows else [],
            }
        finally:
            site.close()
    if command == "preview":
        spec = definition(args["definition"])
        lake = state.lake(spec["library_id"])
        validate_site(lake["site"], spec)
        kind = spec["range"]["kind"]
        known = len(spec["range"]["ids"]) if kind == "ids" else None
        if kind == "new" and spec["range"].get("after_id") is None:
            lib = state.library(spec["library_id"])
            baseline = lib.setting("update_new_metadata_cursor")
            if baseline is None and lake["site"] == "danbooru":
                baseline = lib.setting("api_watermark")
            if baseline is None:
                raise UpdateError("UPDATE_BASELINE_REQUIRED", "首次补充新帖需要填写起点 ID，已有 HF 最大 ID 不能作为完整覆盖依据")
        if kind == "input":
            frozen = state.input(spec["range"]["input_id"])
            if frozen["lake_id"] != spec["library_id"] or frozen["state"] != "sealed":
                raise UpdateError("UPDATE_CONFLICT", "固定输入尚未封存或属于其他数据湖")
            known = frozen["count"]
        return {
            "definition": spec,
            "known_candidates": known,
            "scan_strategy": "id_filtered_scan" if kind in {"created", "updated"} else "keyset",
            "note": "Creation dates use a bounded ID scan; narrow ID bounds to reduce remote requests"
            if kind == "created"
            else None,
        }
    if command == "create":
        return state.create(args["definition"], args["request_key"])
    if command == "jobs":
        return state.jobs(args.get("after", ""), args.get("limit", 50), args.get("lake_id"), args.get("status"))
    if command == "job":
        return state.job(args["id"])
    if command == "items":
        return state.items(args["id"], args.get("after", 0), args.get("limit", 100), args.get("status"), args.get("reason"))
    if command == "action":
        return state.action(args["id"], args["action"])
    if command == "coverage":
        job = state.job(args["id"])
        with state.db() as db:
            row = db.execute("SELECT * FROM coverage WHERE job_id=?", (args["id"],)).fetchone()
        return {"job_id": job["id"], "state": job["state"], "coverage": dict(row) if row else None}
    if command == "schedule_set":
        return state.set_schedule(**args)
    if command == "schedules":
        return state.schedules()
    if command == "schedule_delete":
        with state.db() as db:
            changed = db.execute(
                "DELETE FROM schedules WHERE id=? AND revision=?", (args["id"], args["revision"])
            ).rowcount
        if not changed:
            raise UpdateError("REVISION_CONFLICT", "Schedule missing or changed")
        return {"deleted": True}
    raise UpdateError("INVALID_INPUT", "Unknown update command")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--mode", choices=["rpc", "serve", "run", "import-credentials"], default="rpc")
    parser.add_argument("--job")
    parser.add_argument("--file", type=Path)
    parser.add_argument("--watch-stdin", action="store_true")
    args = parser.parse_args()
    try:
        state = State(args.root)
        if args.mode == "serve":
            runner = Runner(state)
            signal.signal(signal.SIGINT, lambda *_: runner.stop.set())
            signal.signal(signal.SIGTERM, lambda *_: runner.stop.set())
            if args.watch_stdin:

                def watch():
                    sys.stdin.buffer.read(1)
                    runner.stop.set()

                threading.Thread(target=watch, daemon=True).start()
            runner.serve()
            return
        if args.mode == "import-credentials":
            if args.file is None or args.file.stat().st_size > 65536:
                raise UpdateError("INVALID_INPUT", "A small local credential file is required")
            values = json.loads(args.file.read_text(encoding="utf-8-sig"))
            result = {
                "items": [
                    state.set_credentials(site, value)
                    for site, value in values.items()
                    if value.get("api_key")
                ]
            }
        elif args.mode == "run":
            result = Runner(state).run(args.job)
        else:
            raw = sys.stdin.buffer.read(2 * 1024**2 + 1)
            if len(raw) > 2 * 1024**2:
                raise UpdateError("INVALID_INPUT", "Control request exceeds 2 MiB")
            request = json.loads(raw)
            if request.get("protocol_version") != PROTOCOL_VERSION:
                raise UpdateError("UPDATE_PROTOCOL", "Update protocol version mismatch")
            result = dispatch(state, request["command"], request.get("arguments", {}))
        reply = {"protocol_version": PROTOCOL_VERSION, "ok": True, "result": result}
    except UpdateError as error:
        reply = {
            "protocol_version": PROTOCOL_VERSION,
            "ok": False,
            "error": {"code": error.code, "message": str(error)},
        }
    except Exception:
        reply = {
            "protocol_version": PROTOCOL_VERSION,
            "ok": False,
            "error": {"code": "UPDATE_INTERNAL", "message": "Update operation failed; secret values omitted"},
        }
    print(json.dumps(reply, ensure_ascii=False, separators=(",", ":")))
    if not reply["ok"]:
        sys.exit(1)


if __name__ == "__main__":
    main()
