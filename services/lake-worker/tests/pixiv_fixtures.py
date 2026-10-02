"""Small synthetic canonical-media fixtures, also used by Rust/HTTP integration."""

import io
import json
import uuid

from PIL import Image

from studio_lake.collectors.pixiv import normalize as n
from studio_lake.config import Config
from studio_lake.media_lake.library import MediaBatch, MediaLibrary
from studio_lake.media_lake.schema import canonical
from studio_lake.util import digest, stable_id


def sample(root):
    lib = MediaLibrary.initialize(Config(root / "archive", root / "online"))
    job = str(uuid.uuid4())
    definition = {"fixture": "two-identical-pages", "job": job}
    definition_sha = digest(canonical(definition).encode())
    context = n.visibility("fixture-public", observed_at="2026-10-03T00:00:00.000000Z")
    with lib.writer_lock():
        intent = MediaBatch(lib, job, definition_sha, [job], definition=definition)
        intent.commit()
    return lib, dict(job=job, definition_sha=definition_sha, intent=intent.id, context=context)


def new_batch(lib, state):
    return MediaBatch(lib, state["job"], state["definition_sha"], [str(uuid.uuid4())], intent_batch_id=state["intent"])


def capture(lib, state, kind, identity, payload, *, at="2026-10-03T01:00:00.000000Z"):
    return n.capture(lib.info["library_id"], state["context"], str(uuid.uuid4()), "fixture/" + kind,
                     kind, identity, json.dumps(dict(error=False, body=payload), ensure_ascii=False).encode(), observed_at=at)


def add_work(lib, state, *, at="2026-10-03T01:00:00.000000Z", pages=2, declared=None, work_id="12345", tags=None, width=12):
    detail = capture(lib, state, "work", work_id, dict(id=work_id, userId="10109777", title="synthetic 紺屋 fixture",
                     description="fixture", illustType=1, pageCount=pages if declared is None else declared,
                     xRestrict=0, aiType=1, bookmarkCount=7, tags={"tags": [{"tag": t} for t in (tags or ["blue hair", "青髪"])]}), at=at)
    work = n.work(detail)
    media = capture(lib, state, "media", work_id,
                    [dict(width=width, height=9, urls={"original": f"https://i.pximg.net/img-original/{work_id}_p{i}.png"}) for i in range(pages)], at=at)
    records = n.manifest(media, work["work_observations"][0])
    batch = new_batch(lib, state)
    batch.add("visibility_contexts", [state["context"]])
    batch.add("captures", [detail, media])
    for name, rows in {**work, **records}.items():
        batch.add(name, rows)
    with lib.writer_lock():
        seq = batch.commit()
    return records, seq, detail


def add_images(lib, state, records, *, color="blue"):
    output = io.BytesIO()
    Image.new("RGB", (12, 9), color).save(output, format="PNG")
    data = output.getvalue()
    batch = new_batch(lib, state)
    assets = []
    for entry in records["media_entries"]:
        sha = batch.add_blob(data, "png", content_type="image/png", width=12, height=9)
        receipt = str(uuid.uuid4())
        assets.append(dict(asset_id=stable_id("asset-record-v2", entry["media_id"], "original", "original", receipt, sha),
                           media_id=entry["media_id"], sha256=sha, representation="original", recipe_id="original",
                           acquisition_receipt_id=receipt, source_sha256=sha, source_bytes=len(data),
                           acquired_at="2026-10-03T02:00:00.000000Z", last_verified_at="2026-10-03T02:00:00.000000Z",
                           evidence="downloaded", derived_from_asset_id=None, details_json="{}"))
    batch.add("assets", assets)
    with lib.writer_lock():
        seq = batch.commit()
    return sha, data, seq
