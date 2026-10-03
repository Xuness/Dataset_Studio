"""Offline canonical archive packing/rebuild benchmark, not remote collector throughput.

Uses the production batch writer and the collector's receipt-count bound. Each
synthetic single-page work has detail, manifest and original acquisition receipts.
All occurrences share a tiny PNG; this deliberately measures metadata/file-count
amplification, not network bandwidth or large-image decoding.
"""

# ruff: noqa: E402

import argparse
import io
import json
from pathlib import Path
import sys
import time
import uuid

REPO = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO / "services/lake-worker/src"))

from PIL import Image

from studio_lake.collections.batches import MAX_RECEIPTS
from studio_lake.collectors.pixiv import normalize
from studio_lake.config import Config
from studio_lake.media_lake.library import MediaBatch, MediaLibrary
from studio_lake.media_lake import maintenance
from studio_lake.media_lake.schema import canonical, utc
from studio_lake.util import atomic_json, digest, safe_managed_path, stable_id


def uid():
    return str(uuid.uuid4())


def run(output, count):
    output.mkdir()
    lib = MediaLibrary.initialize(Config(output / "archive", output / "online"))
    definition = dict(benchmark="pixiv-canonical-packing-v1", works=count)
    definition_sha, job = digest(canonical(definition).encode()), uid()
    at = utc()
    context = normalize.visibility("synthetic-benchmark-public", observed_at=at)
    buffer = io.BytesIO()
    Image.new("RGB", (12, 9), "blue").save(buffer, format="PNG")
    image = buffer.getvalue()
    started = time.perf_counter()
    write_seconds = publish_seconds = 0
    max_publish_lag = peak_staging = archive_files = archive_bytes = 0
    with lib.writer_lock():
        intent = MediaBatch(lib, job, definition_sha, [uid()], definition=definition)
        intent.commit()
    lib.sync_online()
    chunk = MAX_RECEIPTS // 3
    progress = dict(works=count, processed=0, state="writing", started_at=at)
    milestones = {0, count // 4, count // 2, count * 3 // 4, count}
    reached = set()
    for offset in range(0, count, chunk):
        n = min(chunk, count - offset)
        receipt_ids = [uid() for _ in range(n * 3)]
        batch = MediaBatch(lib, job, definition_sha, receipt_ids, intent_batch_id=intent.id)
        batch.add("visibility_contexts", [context])
        for i in range(n):
            work_id = str(1_000_000 + offset + i)
            body = dict(id=work_id, userId="10109777", title="Synthetic work " + work_id, description="archive benchmark",
                        illustType=0, pageCount=1, width=12, height=9, uploadDate=at, xRestrict=0, aiType=1,
                        tags=dict(tags=[dict(tag="synthetic fixture")]))
            detail = normalize.capture(lib.info["library_id"], context, receipt_ids[i * 3], "fixture/detail", "work", work_id,
                                       canonical(dict(error=False, body=body)).encode(), observed_at=at)
            normalized = normalize.work(detail)
            source = [dict(width=12, height=9, urls=dict(original="https://i.pximg.net/fixture/" + work_id + ".png"))]
            media = normalize.capture(lib.info["library_id"], context, receipt_ids[i * 3 + 1], "fixture/pages", "media", work_id,
                                      canonical(dict(error=False, body=source)).encode(), observed_at=at)
            manifest = normalize.manifest(media, normalized["work_observations"][0])
            for name, rows in {"captures": [detail, media], **normalized, **manifest}.items():
                batch.add(name, rows)
            sha = batch.add_blob(image, "png", content_type="image/png", width=12, height=9)
            media_id, receipt = manifest["media_entries"][0]["media_id"], receipt_ids[i * 3 + 2]
            batch.add("assets", [dict(asset_id=stable_id("asset-record-v2", media_id, "original", "original", receipt, sha),
                                      media_id=media_id, sha256=sha, representation="original", recipe_id="original",
                                      acquisition_receipt_id=receipt, source_sha256=sha, source_bytes=len(image), acquired_at=at,
                                      last_verified_at=at, evidence="downloaded", derived_from_asset_id=None, details_json="{}")])
        stamp = time.perf_counter()
        with lib.writer_lock():
            sealed = batch.seal()
            peak_staging = max(peak_staging, sum(p.stat().st_size for p in batch.path.iterdir() if p.is_file()) +
                               (batch.pack_staging_path.stat().st_size if batch.pack_staging_path else 0))
            lib.accept_manifest(batch.path, sealed)
        accepted = time.perf_counter()
        write_seconds += accepted - stamp
        lib.sync_online()
        published = time.perf_counter()
        publish_seconds += published - accepted
        max_publish_lag = max(max_publish_lag, published - accepted)
        if batch.pack_staging_path:
            batch.pack_staging_path.unlink()
        files = [p for p in (lib.root / "segments" / batch.id).iterdir() if p.is_file()]
        archive_files += len(files)
        archive_bytes += sum(p.stat().st_size for p in files)
        processed = offset + n
        newly_reached = {mark for mark in milestones if mark <= processed} - reached
        if newly_reached:
            reached.update(newly_reached)
            progress.update(processed=processed, elapsed_seconds=published - started, physical_batches=1 + (offset // chunk + 1))
            atomic_json(output / "progress.json", progress)
            print(json.dumps(progress), flush=True)
    acquisition_seconds = time.perf_counter() - started
    rebuild_start = time.perf_counter()
    rebuilt = output / "rebuilt"
    maintenance.build(lib.root, rebuilt, reference_index=lib.cache)
    rebuild_seconds = time.perf_counter() - rebuild_start
    verify_start = time.perf_counter()
    verification = maintenance.verify(rebuilt)
    compared = maintenance.compare(rebuilt)
    verification_seconds = time.perf_counter() - verify_start
    proof = lib.verify(deep=True)
    assert compared["equal"] and proof["objects"] == 1 and proof["captures"] == count * 2
    result = dict(works=count, logical_receipts=count * 3 + 1, physical_batches=proof["batches"],
                  archive_files_excluding_intent=archive_files, archive_bytes_excluding_intent=archive_bytes,
                  packing_and_publication_seconds=acquisition_seconds, writer_seconds=write_seconds,
                  publication_seconds=publish_seconds, max_publication_lag_seconds=max_publish_lag,
                  peak_batch_staging_bytes=peak_staging, rebuild_seconds=rebuild_seconds,
                  verification_seconds=verification_seconds, rebuilt_equal=compared["equal"], archive_verified=proof,
                  raw_roundtrip_verified=verification["raw_roundtrip_verified"], control_database_lock_wait_seconds=None,
                  download_staging_peak_bytes=None, finished_at=utc(),
                  boundary="Synthetic archive writer/publisher/rebuild only; no HTTP, decoder load, control scheduler or large media throughput.")
    atomic_json(output / "result.json", result)
    print(json.dumps(result), flush=True)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--works", nargs="+", type=int, default=[10_000, 100_000])
    args = parser.parse_args()
    base = (REPO / ".local/test-runs").resolve()
    output = safe_managed_path(base, args.output.resolve())
    if output == base or output.exists() or any(n < 1 or n > 100_000 for n in args.works) or len(set(args.works)) != len(args.works):
        parser.error("Use a new child of .local/test-runs and distinct sizes in 1..100000")
    output.mkdir(parents=True)
    results = [run(output / str(size), size) for size in args.works]
    atomic_json(output / "summary.json", dict(results=results))


if __name__ == "__main__":
    main()
