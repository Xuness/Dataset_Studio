"""Line protocol entrypoint, embedded and launched by studio-engine."""

import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import sys
import threading
import traceback

sys.path.insert(0, str(Path(__file__).resolve().parent / "src"))
os.environ.setdefault("HF_HUB_OFFLINE", "1")
os.environ.setdefault("TRANSFORMERS_OFFLINE", "1")
os.environ.setdefault("HF_HUB_DISABLE_PROGRESS_BARS", "1")
os.environ.setdefault("TOKENIZERS_PARALLELISM", "false")
os.environ.setdefault("RAYON_NUM_THREADS", "4")
os.environ["LANCE_CPU_THREADS"] = "4"
os.environ["LANCE_SHUFFLE_BATCH_BYTES"] = str(64 << 20)
os.environ["LANCE_INCLUDE_VECTOR_CENTROIDS"] = "false"

from studio_visual import PROTOCOL_VERSION


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True)
    parser.add_argument("--device", choices=("cuda", "cpu"), default="cuda")
    parser.add_argument("--purpose", choices=("models", "index"), default="models")
    args = parser.parse_args()
    # Reserve stdout exclusively for the protocol, including during third-party
    # imports and inference. Their ordinary print output belongs in the log.
    protocol_out = sys.stdout
    sys.stdout = sys.stderr
    from studio_visual.execution import Execution
    from studio_visual.models import describe
    from studio_visual.vectors import Vectors

    execution = Execution(args.root, args.device) if args.purpose == "models" else None
    # Share one bounded native cache across recipe readers. A fresh connection
    # for every request discards the partition cache and penalizes large indexes.
    readers = execution.vectors if execution else Vectors(args.root, cache_bytes=256 << 20)
    write_lock = threading.Lock()

    def respond(request):
        try:
            op, value = request["op"], request.get("args", {})
            if args.purpose == "index" and op not in ("ping", "index_job"):
                raise ValueError("operation is not permitted in an index process")
            if op == "describe":
                result = describe(value["model_root"], args.device, args.root)
            elif op == "prepare":
                result = execution.prepare(value)
            elif op == "finish":
                result = execution.finish(value["batch_id"])
            elif op == "search":
                result = readers.search(**value)
            elif op in ("entries", "export_page", "snapshot_page", "index_status"):
                result = getattr(readers, op)(**value)
            elif op == "pin":
                result = execution.vectors.pin(**value)
            elif op == "index":
                result = execution.vectors.index(**value)
            elif op == "index_job":
                from studio_visual.indexing import run_index
                result = run_index(readers, **value)
            elif op == "release_index_input":
                from studio_visual.indexing import release_input
                result = release_input(readers, **value)
            elif op == "retention":
                from studio_visual.retention import cleanup
                result = cleanup(readers, **value)
            elif op == "ping":
                result = {"protocol_version": PROTOCOL_VERSION, "pid": os.getpid(), "parent_pid": os.getppid()}
            else:
                raise ValueError("unknown visual operation")
            reply = {"id": request["id"], "result": result}
        except Exception as error:
            traceback.print_exc(file=sys.stderr)
            reply = {"id": request.get("id", ""), "error": str(error)[:2000]}
        with write_lock:
            protocol_out.write(json.dumps(reply, ensure_ascii=False, separators=(",", ":")) + "\n")
            protocol_out.flush()

    # All model and write operations share one queue. CPU searches use separate
    # readers at an explicitly published version and can run during inference.
    with ThreadPoolExecutor(max_workers=1) as gpu, ThreadPoolExecutor(max_workers=2) as cpu:
        while line := sys.stdin.buffer.readline(2 << 20):
            if not line.endswith(b"\n"):
                raise ValueError("oversized protocol message")
            request = json.loads(line)
            (cpu if request.get("op") in ("search", "ping", "entries", "export_page", "snapshot_page", "index_status") else gpu).submit(respond, request)
        # Parent death closes stdin. Never keep an orphan writing while a new
        # engine owns the root. Lance commits and receipt replay handle interruption.
        os._exit(0)


if __name__ == "__main__":
    main()
