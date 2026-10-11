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

from studio_visual import PROTOCOL_VERSION


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True)
    parser.add_argument("--device", choices=("cuda", "cpu"), default="cuda")
    args = parser.parse_args()
    # Reserve stdout exclusively for the protocol, including during third-party
    # imports and inference. Their ordinary print output belongs in the log.
    protocol_out = sys.stdout
    sys.stdout = sys.stderr
    from studio_visual.execution import Execution
    from studio_visual.models import describe
    from studio_visual.vectors import Vectors

    execution = Execution(args.root, args.device)
    write_lock = threading.Lock()

    def respond(request):
        try:
            op, value = request["op"], request.get("args", {})
            if op == "describe":
                result = describe(value["model_root"], args.device, args.root)
            elif op == "prepare":
                result = execution.prepare(value)
            elif op == "finish":
                result = execution.finish(value["batch_id"])
            elif op == "search":
                result = Vectors(args.root).search(**value)
            elif op == "index":
                result = execution.vectors.index(**value)
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
            (cpu if request.get("op") in ("search", "ping") else gpu).submit(respond, request)
        # Parent death closes stdin. Never keep an orphan writing while a new
        # engine owns the root. Lance commits and receipt replay handle interruption.
        os._exit(0)


if __name__ == "__main__":
    main()
