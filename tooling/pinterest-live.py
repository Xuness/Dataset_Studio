"""Explicit, small real-source verification through the production Pinterest Service and Runner."""

import argparse
import hashlib
import io
import json
from pathlib import Path
import sys
import uuid

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "services/lake-worker/src"))

from studio_lake.library import read_object  # noqa: E402
from studio_lake.online_storage import connect  # noqa: E402
from studio_lake.pinterest.runner import Runner  # noqa: E402
from studio_lake.pinterest.service import Service  # noqa: E402
from studio_lake.updates.resources import Resources  # noqa: E402
from studio_lake.updates.state import State  # noqa: E402


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--allow-network", action="store_true", required=True)
    parser.add_argument("--kind", choices=("pin", "board", "section", "search_pins", "search_boards", "topic"), required=True)
    parser.add_argument("--seed", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--requests", type=int, default=3, choices=range(1, 11))
    args = parser.parse_args()
    output = args.output.resolve()
    if not output.is_relative_to(ROOT / ".local/test-runs") or output.exists():
        parser.error("Use a new directory inside this checkout's .local/test-runs")
    state = State(output / "controller")
    service = Service(state)
    lake = service.create_lake(dict(request_key=str(uuid.uuid4()), site="pinterest",
        media_root=str(output / "media"), index_root=str(output / "index")))
    spec = dict(version=1, collector="pinterest_web_v1", library_id=lake["library_id"],
        seeds=[dict(kind=args.kind, id=args.seed)], metadata=dict(detail_enrichment="sample", sample_size=1),
        discovery=dict(include_sections=False, max_pending_downloads=2),
        run_budget=dict(api_requests=args.requests, detail_requests=2, admitted_pins=2, admitted_boards=1,
                        download_bytes=8 * 1024**2, wall_seconds=120))
    job = service.create(dict(request_key=str(uuid.uuid4()), definition=spec))
    result = Runner(state, resources=Resources(max_download_bytes=4 * 1024**2, reserve_bytes=0)).run(job["id"], time_slice=120)
    lib = service.library(lake["library_id"])
    db = connect(output / "index/online.sqlite")
    try:
        objects = []
        for sha, pack, offset, length, width, height in db.execute("SELECT sha256,pack_path,offset,length,stored_width,stored_height FROM objects ORDER BY object_row LIMIT 10"):
            data = read_object(lib.root, pack, offset, length)
            assert hashlib.sha256(data).hexdigest() == sha
            with Image.open(io.BytesIO(data)) as image:
                image.load()
                assert image.size == (width, height)
            objects.append(dict(sha256=sha, bytes=length, width=width, height=height))
        captures = [dict(endpoint=r[0], status=r[1], source_error=r[2]) for r in db.execute("SELECT endpoint,http_status,source_error FROM captures ORDER BY observed_at")]
    finally:
        db.close()
    report = dict(lake=lake, job=result, streams=service.page(dict(job_id=job["id"]), "streams"),
        objects=objects, captures=captures, archive=lib.verify(deep=True))
    (output / "summary.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(dict(output=str(output), state=result["state"], api_requests=result["api_requests"],
        download_bytes=result["download_bytes"], objects=objects, captures=captures), ensure_ascii=False))


if __name__ == "__main__":
    main()
