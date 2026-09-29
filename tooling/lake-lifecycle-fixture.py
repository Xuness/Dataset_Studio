"""Seed only an integration-owned controller while its engine is stopped."""

import json
import sys
from pathlib import Path

root = Path(sys.argv[1]).resolve()
repository = Path(__file__).resolve().parents[1]
if root.parent != repository / ".local/test-runs" or not root.name.startswith("lake-updates-"):
    raise RuntimeError("Lifecycle fixture requires its isolated integration directory")
sys.path.insert(0, str(repository / "services/lake-worker/src"))
from studio_lake.updates import spool
from studio_lake.updates.state import State
from studio_lake.util import atomic_json

state = State(root / "controller")
targets = json.loads((root / "targets.json").read_text(encoding="utf-8"))
tasks = []
for index, role in enumerate(("cancel_via_api", "cancel_before_restart", "paused")):
    lake_id = targets[index]["library_id"]
    task = state.create({"library_id": lake_id, "range": {"kind": "local", "start_id": 1, "end_id": 10}}, role)
    state.action(task["id"], "pause")
    directory = spool.directory(state.library(lake_id), task["id"])
    directory.mkdir(parents=True)
    partial = directory / ("a" * 64 + ".partial")
    partial.write_bytes(b"x" * (1024 * 1024))
    if role == "cancel_before_restart":
        state.action(task["id"], "cancel")
    tasks.append({"id": task["id"], "role": role, "directory": str(directory), "partial": str(partial)})
atomic_json(root / "lifecycle.json", tasks)
