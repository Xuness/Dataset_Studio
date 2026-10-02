"""Standalone access to the same collection service, with JSON input from a file or stdin."""

import argparse
import json
from pathlib import Path
import sys

from .runner import Runner
from .service import Service
from ..media_lake.online import rebuild
from ..media_lake.reader import Reader
from ..updates.sites import UpdateError
from ..updates.state import State
from ..util import FileLock


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", help="Control command (lake_create/accounts/preview/create/job/action/...), run, work, media, objects, raw, or rebuild")
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--input", type=Path, help="UTF-8 JSON arguments; credentials are never accepted in command arguments")
    args = parser.parse_args()
    try:
        if args.input:
            if args.input.stat().st_size > 2 * 1024**2:
                raise UpdateError("COLLECTION_LIMIT", "CLI request exceeds 2 MiB")
            raw = args.input.read_text(encoding="utf-8-sig")
        else:
            raw = sys.stdin.read(2 * 1024**2 + 1)
            if len(raw.encode()) > 2 * 1024**2:
                raise UpdateError("COLLECTION_LIMIT", "CLI request exceeds 2 MiB")
        values = json.loads(raw or "{}")
        state = State(args.root)
        service = Service(state)
        if args.command == "run":
            with FileLock(state.root / "runner.lock", timeout=0):
                result = Runner(state).run(values["id"], time_slice=values.get("time_slice", 30))
        elif args.command in {"work", "media", "objects", "raw", "rebuild"}:
            lib = state.library(values["library_id"])
            if args.command == "rebuild":
                result = rebuild(lib, values["output"], through=values.get("through"))
            else:
                with Reader(lib.root, lib.cache, version=values.get("version")) as reader:
                    if args.command == "work":
                        result = reader.work(values["work_id"])
                    elif args.command == "media":
                        result = reader.media(values["work_id"], **{k: values[k] for k in ("cursor", "limit", "manifest_id", "recipe_id") if k in values})
                    elif args.command == "raw":
                        result = reader.raw(values["capture_id"])
                    else:
                        result = reader.objects(**{k: values[k] for k in ("cursor", "limit") if k in values})
        else:
            result = service.dispatch(args.command.removeprefix("collection_"), values)
        output = dict(protocol_version=1, ok=True, result=result)
    except UpdateError as error:
        output = dict(protocol_version=1, ok=False, error=dict(code=error.code, message=str(error)))
    except Exception:
        output = dict(protocol_version=1, ok=False, error=dict(code="COLLECTION_INTERNAL", message="Collection operation failed; secret values omitted"))
    print(json.dumps(output, ensure_ascii=False))
    if not output["ok"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
