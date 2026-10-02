"""Media acquisition reuses shared range transfers, resource admission and image recipes."""

from dataclasses import dataclass
from datetime import datetime, timezone
import io
import json
import os
from pathlib import Path
import time
import zipfile

from PIL import Image

from ..collectors.pixiv.normalize import media_url
from ..image_policy import prepare_image, profile_id
from ..library import read_object
from ..media_lake.records import one
from ..media_lake.schema import canonical, utc
from ..online_storage import connect
from ..png_compat import inspect_path, open_image
from ..updates.sites import UpdateError
from ..updates.staging import estimate, OVERHEAD
from ..updates.transfer import fetch
from ..util import contained, digest, file_hash, read_json


@dataclass(frozen=True)
class Plan:
    base: object
    retain: bool

    def peak(self, size=None):
        source = self.base.download_bytes if size is None else size
        return self.base.peak(size) + (2 * source if self.retain else 0)


def stored_file(directory, key, data, *, role, recipe, ext, width=None, height=None, category="image", details=None, evidence="downloaded", verified_at=None, source_sha=None, source_bytes=None):
    path = directory / (key + "." + role + ".ready")
    temporary = path.with_suffix(".tmp")
    with temporary.open("wb") as handle:
        handle.write(data)
        handle.flush()
        os.fsync(handle.fileno())
    temporary.replace(path)
    return dict(path=path.name, sha256=digest(data), bytes=len(data), representation=role, recipe_id=recipe,
                stored_ext=ext, content_type="application/zip" if category == "archive" else Image.MIME.get({"jpg": "JPEG"}.get(ext, ext.upper()), "application/octet-stream"),
                media_category=category, stored_width=width, stored_height=height, details_json=canonical(details or {}),
                evidence=evidence, last_verified_at=verified_at, source_sha256=source_sha, source_bytes=source_bytes)


def history(lib, entry, frames, spec):
    reuse = spec["media"]["reuse"]
    if reuse["mode"] != "historical_if_same_locator" or reuse["max_age_hours"] == 0:
        return None
    policy = spec["media"]["image_policy"]
    required = [("original", "original")] if spec["media"]["retain_original"] or entry["kind"] == "ugoira" else []
    if entry["kind"] == "ugoira":
        required.append(("poster", "ugoira-poster-v1"))
    elif policy["profile"] != "original":
        required.append(("derived", profile_id(policy)))
    pointer = read_json(lib.cache / "ONLINE.json")
    db = connect(contained(lib.cache, pointer["file"]))
    try:
        seq = int(db.execute("SELECT value FROM online_state WHERE key='served_seq'").fetchone()[0])
        candidates = list(db.execute("""SELECT m.media_id FROM media_entries m WHERE m.work_id=? AND m.slot_key=? AND m.source_url=?
            AND m.source_variant=? AND m.width IS ? AND m.height IS ? AND m.commit_seq<=?
            ORDER BY m.commit_seq DESC LIMIT 32""", (entry["work_id"], entry["slot_key"], entry["source_url"], entry["source_variant"], entry["width"], entry["height"], seq)))
        for (media_id,) in candidates:
            if frames and list(db.execute("SELECT file_name,delay_ms FROM animation_frames WHERE media_id=? ORDER BY ordinal", (media_id,))) != [(f["file_name"], f["delay_ms"]) for f in frames]:
                continue
            found = []
            for representation, recipe in required:
                row = one(db, """SELECT a.*,o.pack_path,o.offset,o.length,o.stored_ext,o.content_type,o.media_category,o.stored_width,o.stored_height
                    FROM assets a JOIN objects o USING(sha256) WHERE a.media_id=? AND a.representation=? AND a.recipe_id=?
                    AND a.commit_seq<=? AND a.last_verified_at IS NOT NULL ORDER BY a.last_verified_at DESC LIMIT 1""", (media_id, representation, recipe, seq))
                if row is None or (datetime.now(timezone.utc) - datetime.fromisoformat(row["last_verified_at"].replace("Z", "+00:00"))).total_seconds() > reuse["max_age_hours"] * 3600:
                    break
                found.append(row)
            if len(found) == len(required):
                return found
    finally:
        db.close()
    return None


def acquire(lib, task, spec, directory, client, sessions, resources, cancelled, progress=lambda **_: None):
    payload = json.loads(task["payload_json"])
    entry, frames, key = payload["entry"], payload["frames"], task["id"]
    media_url(entry["source_url"])
    policy = spec["media"]["image_policy"]
    reuse = history(lib, entry, frames, spec)
    if reuse:
        required = sum(r["length"] for r in reuse) * 2 + OVERHEAD
        resources.stage_size(directory, key, required)
        files = []
        for row in reuse:
            data = read_object(lib.root, row["pack_path"], row["offset"], row["length"])
            if digest(data) != row["sha256"]:
                raise UpdateError("COLLECTION_INTEGRITY", "Historical object bytes failed verification")
            files.append(stored_file(directory, key, data, role=row["representation"], recipe=row["recipe_id"], ext=row["stored_ext"],
                                     width=row["stored_width"], height=row["stored_height"], category=row["media_category"],
                                     details=dict(reused_asset_id=row["asset_id"], locator=entry["source_url"], original_details=json.loads(row["details_json"])),
                                     evidence="historical_reuse", verified_at=row["last_verified_at"], source_sha=row["source_sha256"], source_bytes=row["source_bytes"]))
        return dict(files=files, download_bytes=0, reused=True, acquired_at=utc())
    raw, download_receipt = directory / (key + ".downloaded"), directory / (key + ".download.json")
    downloaded = None
    if raw.exists() and download_receipt.exists():
        saved = read_json(download_receipt)
        if raw.stat().st_size <= resources.max_download_bytes and file_hash(raw) == saved.get("sha256"):
            downloaded = {**saved, "download_path": str(raw)}
    if downloaded is None:
        downloaded = fetch(directory, key, entry["source_url"], "original", {"post_id": entry["work_id"]}, client, resources, cancelled, sessions, progress)
    if downloaded["state"] != "downloaded":
        raise UpdateError("COLLECTION_REMOTE_UNAVAILABLE" if downloaded["state"] == "failed" else "COLLECTION_MEDIA_INVALID",
                          downloaded.get("reason", "media_download_failed"), retry_after=max(1, downloaded.get("retry_at", time.time() + 60) - time.time()))
    acquired_at = utc()
    if entry["kind"] == "ugoira":
        return animation(directory, key, downloaded, frames, resources, cancelled, acquired_at)
    inspected = inspect_path(raw)
    if inspected is None:
        with Image.open(raw) as header:
            width, height = header.size
    else:
        width, height = inspected.width, inspected.height
    if width * height > resources.config["max_image_pixels"]:
        raise UpdateError("COLLECTION_LIMIT", "Image exceeds the shared pixel budget")
    if entry["width"] is not None and entry["height"] is not None and (width, height) != (entry["width"], entry["height"]):
        raise UpdateError("COLLECTION_SOURCE_CHANGED", "Downloaded dimensions differ from the frozen media manifest")
    plan = Plan(estimate(policy, {"width": width, "height": height}, resources, source_bytes=raw.stat().st_size), spec["media"]["retain_original"])
    resources.stage_size(directory, key, plan.peak())
    with resources.encoding(width * height * 16 + raw.stat().st_size * 3, cancelled):
        data = raw.read_bytes()
        if digest(data) != downloaded["sha256"]:
            raise UpdateError("COLLECTION_INTEGRITY", "Downloaded media receipt changed")
        image, _ = open_image(data)
        with image:
            image.load()
        files = []
        if spec["media"]["retain_original"]:
            _, ext, details = prepare_image(data, {"profile": "original"})
            files.append(stored_file(directory, key, data, role="original", recipe="original", ext=ext, width=width, height=height,
                                     details=details, verified_at=acquired_at, source_sha=downloaded["sha256"], source_bytes=len(data)))
        if policy["profile"] != "original":
            encoded, ext, details = prepare_image(data, policy)
            resources.stage_size(directory, key, 2 * (len(data) + len(encoded)) + OVERHEAD)
            files.append(stored_file(directory, key, encoded, role="derived", recipe=profile_id(policy), ext=ext,
                                     width=details["stored_width"], height=details["stored_height"], details=details, evidence="derived",
                                     verified_at=acquired_at, source_sha=downloaded["sha256"], source_bytes=len(data)))
    resources.stage_size(directory, key, raw.stat().st_size + 2 * sum(f["bytes"] for f in files) + OVERHEAD)
    return dict(files=files, download_bytes=downloaded["download_bytes"], reused=False, acquired_at=acquired_at)


def animation(directory, key, downloaded, frames, resources, cancelled, at):
    raw = Path(downloaded["download_path"])
    if not frames or len(frames) > 20000:
        raise UpdateError("COLLECTION_LIMIT", "Animation frame count exceeds its budget")
    with zipfile.ZipFile(raw) as archive:
        entries = archive.infolist()
        expected = [f["file_name"] for f in frames]
        if len(entries) != len(expected) or len({f.filename for f in entries}) != len(entries) or set(f.filename for f in entries) != set(expected):
            raise UpdateError("COLLECTION_SOURCE_CHANGED", "Animation package differs from the frozen frame list")
        total = sum(e.file_size for e in entries)
        if total > min(512 * 1024**2, resources.max_download_bytes * 8) or any(e.file_size > resources.max_download_bytes or e.flag_bits & 1 for e in entries):
            raise UpdateError("COLLECTION_LIMIT", "Animation decompression exceeds its budget")
        with archive.open(expected[0]) as stream, Image.open(stream) as header:
            width, height = header.size
        if width * height > resources.config["max_image_pixels"]:
            raise UpdateError("COLLECTION_LIMIT", "Animation poster exceeds the pixel budget")
        first_size = archive.getinfo(expected[0]).file_size
        resources.stage_size(directory, key, raw.stat().st_size * 3 + first_size * 3 + OVERHEAD)
        with resources.encoding(width * height * 16 + first_size * 3 + raw.stat().st_size, cancelled):
            first = archive.read(expected[0])
            # CRC validation streams every member; nothing is extracted to a caller-controlled path.
            for member in entries:
                with archive.open(member) as stream:
                    while stream.read(256 * 1024):
                        if cancelled():
                            raise UpdateError("CANCELLED", "Animation verification paused")
            image, _ = open_image(first)
            with image:
                image.load()
                poster = io.BytesIO()
                image.convert("RGBA").save(poster, format="PNG")
            data = raw.read_bytes()
            poster_data = poster.getvalue()
            resources.stage_size(directory, key, len(data) * 3 + len(poster_data) * 2 + OVERHEAD)
            original = stored_file(directory, key, data, role="original", recipe="original", ext="zip", category="archive",
                                   details=dict(variant="web_frame_package", frame_count=len(frames)), verified_at=at,
                                   source_sha=downloaded["sha256"], source_bytes=len(data))
            cover = stored_file(directory, key, poster_data, role="poster", recipe="ugoira-poster-v1", ext="png", width=width, height=height,
                                details=dict(first_frame=expected[0], frame_count=len(frames)), evidence="derived", verified_at=at,
                                source_sha=downloaded["sha256"], source_bytes=len(data))
    return dict(files=[original, cover], download_bytes=downloaded["download_bytes"], reused=False, acquired_at=at)
