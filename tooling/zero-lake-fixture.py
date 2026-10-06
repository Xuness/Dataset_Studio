"""Populate UI-created isolated lakes through real runners with recorded source responses."""
# ruff: noqa: E402 -- fixture imports use the repository paths set below.

from pathlib import Path
import json
import sys

repo = Path(__file__).resolve().parents[1]
run = Path(sys.argv[1]).resolve()
if (repo / ".local" / "test-runs").resolve() not in run.parents:
    raise ValueError("The fixture must stay under .local/test-runs")
sys.path[:0] = [str(repo / "services/lake-worker/src"), str(repo / "services/lake-worker/tests")]

import requests
from conftest import png
from test_updates import FakeSite, Images, Resources, Runner, job, post
from test_collections import HTTP, key
from test_collection_continuous import FreshClient
from studio_lake.collections.model import sample_definition
from studio_lake.collections.runner import Runner as CollectionRunner
from studio_lake.collections.service import Service
from studio_lake.updates.state import State


def forbidden(*args, **kwargs):
    raise AssertionError("UI fixture must never contact a real source")


requests.Session.send = forbidden
state = State(run / "controller")
targets = json.loads((run / "created-lakes.json").read_text(encoding="utf-8"))
results = []
for lake in targets:
    for field in ("media", "index_root"):
        if run not in Path(lake[field]).resolve().parents:
            raise ValueError("Fixture lake escaped the owned test run")
    if lake["site"] != "pixiv":
        lib = state.library(lake["id"])
        data = png("red")
        records = [post(lake["site"], pid, data) for pid in (11, 12)]
        records[1]["tag_string" if lake["site"] == "danbooru" else "tags"] = "a c"
        runner = Runner(state, {lake["site"]: FakeSite(lake["site"], records)}, Resources(reserve_bytes=0), Images(data))
        metadata = job(state, lib, dict(kind="ids", ids=[11, 12]))
        assert runner.run(metadata["id"])["state"] == "completed"
        image = job(state, lib, dict(kind="ids", ids=[11]), "original")
        assert runner.run(image["id"])["state"] == "completed"
        results.append(dict(site=lake["site"], metadata_posts=2, stored_posts=1))
    else:
        service = Service(state)
        account = service.accounts.save(dict(request_key=key(), expected_revision=None, account_id=key(), label="offline fixture", mode="anonymous"))
        task = service.create_job(dict(request_key=key(), definition=sample_definition(lake["id"], account["id"], ["10109777"])))
        runner = CollectionRunner(state, resources=Resources(reserve_bytes=0), client_factory=FreshClient, image_http=HTTP())
        outcome = runner.run(task["job"]["id"], time_slice=60)
        assert outcome["state"] == "completed", outcome
        results.append(dict(site="pixiv", works=1, stored_media=2))
(run / "fixture-result.json").write_text(json.dumps(results, indent=2), encoding="utf-8")
print(json.dumps(results))
