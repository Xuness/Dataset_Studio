import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor

import pytest

from conftest import png
from test_updates import FakeSite, post, setup, job
from test_pipeline import configure
from studio_lake.updates.archive import online
from studio_lake.updates.runner import Runner
from studio_lake.updates.state import State
from studio_lake.util import FileLock


def wait_for(check, timeout=20):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        result = check()
        if result:
            return result
        time.sleep(0.03)
    raise AssertionError("Timed out waiting for recovery fixture")


def test_concurrent_daemon_and_rpc_initialization_is_safe(tmp_path):
    for attempt in range(25):
        gate = threading.Barrier(4)

        def initialize(_):
            gate.wait(timeout=10)
            state = State(tmp_path / str(attempt))
            with state.db() as db:
                version = db.execute("PRAGMA user_version").fetchone()[0]
            return version, state.credential_status()

        with ThreadPoolExecutor(max_workers=4) as pool:
            assert list(pool.map(initialize, range(4))) == [(8, [])] * 4


def test_failed_lock_file_initialization_releases_os_ownership(tmp_path, monkeypatch):
    path = tmp_path / "failure.lock"
    original_open = Path.open

    class FailingWrite:
        def __init__(self, handle):
            self.handle = handle

        def __getattr__(self, name):
            return getattr(self.handle, name)

        def write(self, _data):
            raise OSError("fixture initialization write failure")

    def failing_open(target, *args, **kw):
        handle = original_open(target, *args, **kw)
        return FailingWrite(handle) if target == path else handle

    with monkeypatch.context() as patch:
        patch.setattr(Path, "open", failing_open)
        with pytest.raises(OSError, match="initialization"):
            with FileLock(path, timeout=0):
                pytest.fail("initialization must fail")
    with FileLock(path, timeout=0):
        assert path.stat().st_size == 1


def test_killed_download_process_is_automatically_resumed_once_without_touching_paused_jobs(tmp_path):
    # Uncompressed pixels produce a large, structurally valid PNG for range recovery.
    data = png("red", size=(2048, 1024), compress_level=0)
    (tmp_path / "original.png").write_bytes(data)
    lib, state = setup(tmp_path, "yandere")
    configure(state, max_download_mib=8, spool_mib=64)
    task = job(state, lib, {"kind": "ids", "ids": [11]}, "original")
    paused = job(state, lib, {"kind": "ids", "ids": [12]}, "original")
    state.action(paused["id"], "pause")
    release, requested = threading.Event(), threading.Event()
    calls = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_GET(self):
            header = self.headers.get("Range")
            start = int(header.removeprefix("bytes=").removesuffix("-")) if header else 0
            calls.append({"range": header, "if_range": self.headers.get("If-Range"), "start": start})
            self.send_response(206 if header else 200)
            self.send_header("Content-Length", str(len(data) - start))
            self.send_header("ETag", '"fixture-v1"')
            if header:
                self.send_header("Content-Range", f"bytes {start}-{len(data) - 1}/{len(data)}")
            self.end_headers()
            try:
                cut = 5 * 1024**2
                if not header:
                    self.wfile.write(data[:cut])
                    self.wfile.flush()
                    requested.set()
                    release.wait(30)
                    self.wfile.write(data[cut:])
                else:
                    self.wfile.write(data[start:])
            except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    endpoint = f"http://127.0.0.1:{server.server_port}/image"
    server_thread = threading.Thread(target=server.serve_forever, daemon=True)
    server_thread.start()
    child = None
    try:
        # Run the real Python interpreter directly, avoiding Windows venv launcher's extra child process.
        env = {
            **os.environ,
            "PYTHONPATH": os.pathsep.join(str(p) for p in sys.path if p),
            "PYTHONDONTWRITEBYTECODE": "1",
        }
        with (tmp_path / "worker.log").open("w", encoding="utf-8") as log:
            child = subprocess.Popen(
                [
                    sys._base_executable,
                    str(Path(__file__).parent / "helpers/resume_worker.py"),
                    str(tmp_path),
                    endpoint,
                ],
                env=env,
                stdout=log,
                stderr=log,
            )
            assert requested.wait(20), "child failed: " + (tmp_path / "worker.log").read_text()
            directory = lib.cache / "updates" / task["id"]

            def saved():
                paths = list(directory.glob("*.partial.json"))
                if not paths:
                    return None
                checkpoint = json.loads(paths[0].read_text())
                return checkpoint if checkpoint["bytes"] >= 4 * 1024**2 else None

            checkpoint = wait_for(saved)
            child.kill()
            child.wait(timeout=10)
        release.set()
        orphan = state.job(task["id"])
        assert orphan["state"] == "running" and not orphan["execution_active"]
        assert orphan["telemetry"]["phase"] == "waiting_worker"
        assert orphan["telemetry"]["files"] == [] and orphan["telemetry"]["download_rate_bps"] == 0

        from helpers.resume_worker import LocalImages

        runner = Runner(
            state,
            {"yandere": FakeSite("yandere", [post("yandere", 11, data)])},
            image_http=LocalImages(endpoint),
        )
        with ThreadPoolExecutor(max_workers=1) as pool:
            future = pool.submit(runner.serve)
            try:
                wait_for(lambda: state.job(task["id"])["state"] == "completed", 30)
            finally:
                runner.stop.set()
            future.result(timeout=15)
        done = state.job(task["id"])
        assert done["counts"] == {"stored": 1}
        assert done["telemetry"]["recovery_count"] == 1
        assert done["telemetry"]["resumed_requests"] == 1
        assert state.job(paused["id"])["state"] == "paused"
        assert len(calls) == 2 and calls[1]["start"] == checkpoint["bytes"]
        assert calls[1]["if_range"] == '"fixture-v1"'
        assert not directory.exists() or not list(directory.iterdir())
        with online(lib) as (db, _):
            assert db.execute("SELECT count(*) FROM assets").fetchone()[0] == 1
            assert db.execute("SELECT count(*) FROM objects").fetchone()[0] == 1
            assert db.execute("SELECT sha256 FROM assets").fetchone()[0] == hashlib.sha256(data).hexdigest()
        assert state.items(task["id"])["items"][0]["attempts"] == 1
        (tmp_path / "recovery-proof.json").write_text(
            json.dumps(
                {
                    "passed": True,
                    "checkpoint_bytes": checkpoint["bytes"],
                    "source_bytes": len(data),
                    "requests": calls,
                    "final_counts": done["counts"],
                    "recovery_count": done["telemetry"]["recovery_count"],
                    "resumed_requests": done["telemetry"]["resumed_requests"],
                    "paused_task_state": state.job(paused["id"])["state"],
                    "single_asset_sha256": hashlib.sha256(data).hexdigest(),
                },
                indent=2,
            ),
            encoding="utf-8",
        )
    finally:
        release.set()
        if child is not None and child.poll() is None:
            child.kill()
            child.wait(timeout=10)
        server.shutdown()
        server.server_close()


def test_orderly_service_shutdown_queues_same_job_for_automatic_resumption(tmp_path):
    from test_updates import ImageResponse, Images
    from studio_lake.updates.sites import UpdateError

    data = png("blue")
    lib, state = setup(tmp_path, "yandere")
    configure(state)
    task = job(state, lib, {"kind": "ids", "ids": [11]}, "original")
    entered = threading.Event()
    remote = FakeSite("yandere", [post("yandere", 11, data)])

    class Held(ImageResponse):
        def iter_content(self, _):
            yield self.data[:20]
            entered.set()
            assert runner.stop.wait(10)
            raise UpdateError("CANCELLED", "service stopped")

    class ImageHTTP(Images):
        def get(self, *_args, **_kw):
            return Held(data)

    runner = Runner(state, {"yandere": remote}, image_http=ImageHTTP(data))
    with ThreadPoolExecutor(max_workers=1) as pool:
        future = pool.submit(runner.run, task["id"])
        assert entered.wait(10)
        runner.stop.set()
        stopped = future.result(timeout=15)
    assert stopped["state"] == "queued" and stopped["retry_at"] == 0
    assert stopped["error_code"] == "UPDATE_INTERRUPTED"
    assert not stopped["execution_active"]
    directory = lib.cache / "updates" / task["id"]
    assert next(directory.glob("*.partial")).read_bytes() == data[:20]
    # A normal server may ignore Range; its full response must safely replace the partial.
    done = Runner(state, {"yandere": remote}, image_http=Images(data)).run(task["id"])
    assert done["state"] == "completed" and done["counts"] == {"stored": 1}
    assert done["telemetry"]["recovery_count"] == 1
    assert done["telemetry"]["last_recovery_reason"] == "service_restarted"
