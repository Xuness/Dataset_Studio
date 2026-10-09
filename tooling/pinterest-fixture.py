"""Offline fixture through the production Pinterest runner; never calls the source site."""
# ruff: noqa: E402
import hashlib
import json
import sys
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "services/lake-worker/src"), str(ROOT / "services/lake-worker/tests")]

from pinterest_fixtures import Fixture
from test_pinterest_manifest import pin
from studio_lake.canonical import utc
from studio_lake.pinterest.http import Response, request_parameters
from studio_lake.online_storage import connect
from studio_lake.util import atomic_json, read_json


def main():
    root = Path(sys.argv[1]).resolve()
    if not root.is_relative_to(ROOT / ".local/test-runs"):
        raise ValueError("Fixture must stay inside .local/test-runs")
    root.mkdir(parents=True, exist_ok=True)

    def details(pin):
        pin.update(title="红色测试图", description=None, is_ai_generated=False, repin_count=0,
                   board={"id": "1001", "name": "颜色画板"}, pinner={"id": "2001", "full_name": "保存账号样本"})
    fixture = Fixture(root, ("858146903966145189", "1089097122423035272"), pin_transform=details)
    result = fixture.run()
    assert result["state"] == "completed", result
    pointer = read_json(root / "index/ONLINE.json")
    first_version = f'online-v4:{pointer["generation"]}:{result["served_seq"]}'
    db = connect(root / "index/online.sqlite")
    try:
        records = [dict(zip(("asset_id", "pin_id", "observation_id"), row)) for row in db.execute(
            "SELECT a.asset_id,m.pin_id,f.observation_id FROM assets a JOIN media_entries m USING(media_id) JOIN media_manifests f USING(manifest_id) ORDER BY a.asset_id")]
    finally:
        db.close()
    # Leave a known exhausted budget for API action tests. It cannot send a request after the daemon starts.
    spec = {**fixture.spec, "seeds": [{"kind": "pin", "id": "3001"}, {"kind": "pin", "id": "3002"}],
            "metadata": {"detail_enrichment": "none"},
            "run_budget": {"api_requests": 1, "detail_requests": 1, "admitted_pins": 1}}
    fixture.job = fixture.service.create({"request_key": str(uuid.uuid4()), "definition": spec})
    budget_job = fixture.run()
    assert budget_job["state"] == "waiting_budget", budget_job
    old_factory = fixture.factory
    def factory(root, context, *, cancelled):
        client = old_factory(root, context, cancelled=cancelled)
        def request(kind, entry):
            value = pin("858146903966145189" if entry["cursor"] is None else "1089097122423035272")
            value.update(type="pin")
            details(value)
            body = dict(resource_response=dict(status="success", data=[value]),
                        resource=dict(options=dict(bookmarks=["next"] if entry["cursor"] is None else ["-end-"])))
            endpoint, options, source = request_parameters(kind, entry)
            return Response(json.dumps(body).encode(), endpoint, dict(options=options, source_url=source), 200, utc(), context)
        client.request = request
        return client
    fixture.factory = factory
    fixture.job = fixture.service.create(dict(request_key=str(uuid.uuid4()), definition={**fixture.spec,
        "seeds": [dict(kind="board", id="1001")], "metadata": dict(detail_enrichment="none"),
        "discovery": dict(include_sections=False)}))
    discovery_job = fixture.run()
    assert discovery_job["state"] == "completed", discovery_job
    atomic_json(root / "pinterest.json", {"lake": fixture.lake, "job": result, "budget_job": budget_job,
        "discovery_job": discovery_job,
        "records": records, "first_version": first_version, "sha256": hashlib.sha256(fixture.data).hexdigest(),
        "md5": hashlib.md5(fixture.data).hexdigest(), "bytes": len(fixture.data), "original_hex": fixture.data.hex()})


if __name__ == "__main__":
    main()
