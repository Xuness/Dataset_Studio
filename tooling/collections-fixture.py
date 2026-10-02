"""Offline end-to-end Pixiv fixture; no requests leave this process."""

from dataclasses import replace
import json
from pathlib import Path
import sys
import uuid

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "services" / "lake-worker" / "src"))
sys.path.insert(0, str(ROOT / "services" / "lake-worker" / "tests"))

from test_collections import Client, HTTP, setup
from studio_lake.collections.model import sample_definition
from studio_lake.media_lake.reader import Reader
from studio_lake.util import atomic_json


class MultiClient(Client):
    def request(self, kind, payload):
        response = super().request(kind, payload)
        data = json.loads(response.body)
        if kind == "author_directory":
            data["body"]["illusts"]["24680"] = None
        if kind == "work_detail":
            data["body"]["bookmarkCount"] = 99 if payload["work_id"] == "24680" else 1
            data["body"]["tags"]["tags"] = [{"tag": "red hair" if payload["work_id"] == "24680" else "blue hair"}]
        return replace(response, body=json.dumps(data).encode())


class NextClient(Client):
    def request(self, kind, payload):
        response = super().request(kind, payload)
        return replace(response, observed_at="2026-10-03T04:00:00.000000Z")


def main():
    root = Path(sys.argv[1]).resolve()
    root.mkdir(parents=True, exist_ok=True)
    service, runner, job, _ = setup(root)
    runner.client_factory = MultiClient
    result = runner.run(job["id"], time_slice=60)
    assert result["state"] == "completed", result
    lake = service.lake(job["library_id"])
    with Reader(lake["media_root"], lake["index_root"]) as reader:
        first_version = reader.version
        sha = reader.objects()["items"][0]["sha256"]
    spec = sample_definition(job["library_id"], job["account_id"], ["12345"], kind="works", metadata_only=True)
    next_job = service.create_job(dict(request_key=str(uuid.uuid4()), definition=spec))["job"]
    runner.client_factory = NextClient
    assert runner.run(next_job["id"], time_slice=60)["state"] == "completed"
    with Reader(lake["media_root"], lake["index_root"]) as reader:
        latest_version = reader.version
    atomic_json(root / "collections.json", dict(lake=lake, job=service.job(job["id"]),
                account_id=job["account_id"], first_version=first_version, latest_version=latest_version, sha256=sha))


if __name__ == "__main__":
    main()
