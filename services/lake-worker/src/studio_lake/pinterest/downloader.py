"""URL-and-access-scoped acquisition, preserving bytes and validating every media binding."""

import hashlib
import json
import re
from types import SimpleNamespace

from PIL import Image, UnidentifiedImageError

from .http import MediaSessions
from . import reuse
from ..canonical import canonical, utc
from ..png_compat import inspect_path, PngCompatibilityError
from ..updates import transfer
from ..updates.sites import UpdateError
from ..updates.staging import StagingPlan, INITIAL_DOWNLOAD
from ..util import file_hash, read_json, stable_id


def acquire(state, lib, job, task, entry, resources, cancelled, *, http=None):
    key = stable_id("pinterest-url-acquisition-v1", job["id"], entry["normalized_url"], entry["context_id"], task["download_generation"])
    directory = lib.cache / "pinterest_downloads" / job["id"]
    directory.mkdir(parents=True, exist_ok=True)
    with state.db() as db:
        previous = db.execute("SELECT acquisition_json FROM pinterest_downloads WHERE job_id=? AND download_key=?", (job["id"], key)).fetchone()
    if previous:
        saved = json.loads(previous[0])
        path = directory / (key + ".downloaded")
        if (saved["width"], saved["height"]) != (entry["width"], entry["height"]):
            return dict(state="needs_review", reason="shared_url_dimensions_mismatch"), None
        if not path.is_file():
            if reuse.object_available(lib, saved, resources, cancelled):
                return {**saved, "state": "downloaded", "reused": True, "archive_reuse": True}, None
            raise UpdateError("PINTEREST_ACQUISITION_LOST", "A staged acquisition is missing or changed; explicit retry is required")
        if file_hash(path) != saved["download_sha256"]:
            raise UpdateError("PINTEREST_ACQUISITION_LOST", "A staged acquisition is missing or changed; explicit retry is required")
        return {**saved, "state": "downloaded", "reused": True}, path
    plan = StagingPlan(min(resources.max_download_bytes, INITIAL_DOWNLOAD), None)
    lease = resources.try_reserve(directory, key, plan.peak())
    if lease is None:
        raise UpdateError("UPDATE_SPACE", "Waiting for Pinterest download staging space")
    slot = resources.try_download("pinterest")
    if slot is None:
        lease.release()
        raise UpdateError("UPDATE_SPACE", "Waiting for a shared Pinterest download slot")
    resources.plan(directory, key, plan)
    sessions = MediaSessions(http)
    with state.db() as db:
        origin = db.execute("SELECT entrypoint FROM pinterest_streams WHERE scan_id=? AND job_id=?", (entry.get("scan_id"), job["id"])).fetchone()
    try:
        path, receipt = directory / (key + ".downloaded"), directory / (key + ".download.json")
        reuse_key = reuse.key(lib, entry)
        if not path.exists() and not (directory / (key + ".partial")).exists():
            prior, response = reuse.previous(state, lib, job, task, entry, reuse_key, key, sessions, resources, cancelled)
            if prior:
                if prior.get("state") == "needs_review":
                    return prior, None
                with state.db() as db:
                    db.execute("INSERT INTO pinterest_downloads VALUES(?,?,?)", (job["id"], key, canonical(prior)))
                return {**prior, "state": "downloaded", "reused": True}, None
            if response is not None:
                sessions.session = reuse.Prefetched(sessions.session, response)
        saved = read_json(receipt) if receipt.exists() and path.exists() else None
        if not saved or saved.get("sha256") != file_hash(path) or not saved.get("md5"):
            def progress(**values):
                delta = values.get("downloaded_bytes_delta", 0)
                if delta:
                    with state.db() as db:
                        db.execute("UPDATE pinterest_jobs SET download_bytes=download_bytes+? WHERE id=?", (delta, job["id"]))
                        if origin:
                            db.execute("INSERT INTO pinterest_metrics VALUES(?,?,?) ON CONFLICT(job_id,name) DO UPDATE SET value=value+excluded.value",
                                       (job["id"], origin[0] + ":download_bytes", delta))

            saved = transfer.fetch(directory, key, entry["normalized_url"], "original", {"post_id": entry["pin_id"]},
                SimpleNamespace(name="pinterest", rate_root=state.root), resources, cancelled, sessions, progress)
        if saved["state"] != "downloaded":
            return saved, None
        etag = saved.get("etag")
        if etag and re.fullmatch(r'"[0-9a-fA-F]{32}"', etag) and etag[1:-1].lower() != saved["md5"]:
            return dict(state="needs_review", reason="cdn_etag_md5_mismatch"), None
        try:
            inspect_path(path)
            with Image.open(path) as image:
                width, height = image.size
                frames = getattr(image, "n_frames", 1)
                fmt = image.format
                if frames != 1:
                    return dict(state="needs_review", reason="animation_not_supported"), None
                if (width, height) != (entry["width"], entry["height"]):
                    return dict(state="needs_review", reason="original_dimensions_mismatch"), None
                if width * height > resources.config["max_image_pixels"]:
                    raise UpdateError("UPDATE_RESOURCE_LIMIT", "Pinterest image exceeds the pixel budget")
                image.verify()
            with resources.encoding(width * height * 16 + path.stat().st_size, cancelled):
                with Image.open(path) as image:
                    image.load()
                    image.convert("RGBA").load()
            if fmt not in ("JPEG", "PNG", "WEBP", "GIF", "AVIF"):
                return dict(state="needs_review", reason="image_format_not_supported"), None
        except (UnidentifiedImageError, OSError, ValueError, PngCompatibilityError):
            return dict(state="needs_review", reason="image_decode_failed"), None
        actual_md5 = hashlib.md5()
        with path.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024**2), b""):
                actual_md5.update(chunk)
        if actual_md5.hexdigest() != saved["md5"]:
            raise UpdateError("PINTEREST_ACQUISITION_CHANGED", "Downloaded Pinterest bytes changed before validation")
        result = dict(acquisition_id=stable_id("pinterest-acquisition-v1", job["id"], key), context_id=entry["context_id"],
            source_url=entry["source_url"], normalized_url=entry["normalized_url"], download_sha256=saved["sha256"],
            download_md5=saved["md5"], download_bytes=saved["download_bytes"], cdn_etag=etag, acquired_at=utc(),
            evidence="downloaded", details_json=canonical(dict(frames=1, source_format=fmt, original_field=entry["field_path"])),
            width=width, height=height, ext={"JPEG": "jpg"}.get(fmt, fmt.lower()), content_type=Image.MIME.get(fmt, "image/" + fmt.lower()),
            download_key=key, reuse_key=reuse_key, source_checked_at=utc())
        with state.db() as db:
            db.execute("INSERT INTO pinterest_downloads VALUES(?,?,?)", (job["id"], key, canonical(result)))
        return {**result, "state": "downloaded", "reused": False}, path
    finally:
        sessions.close()
        slot.release()
        lease.release()
