"""Bounded SSD downloads and explicit media-variant reuse; no network inside lake transactions."""

import errno
import json
import os
import re
import threading
import time
from urllib.parse import urlsplit

from PIL import Image
import requests

from ..image_policy import prepare_image, profile_id, ImagePolicyError
from ..util import atomic_json, digest, file_hash, stable_id
from .archive import online, record_by_id
from .sites import UpdateError
from .resources import Resources as Resources
from .resources import Reservation
from .transfer import fetch


# The service applies its explicit configurable pixel and memory guards before load().
Image.MAX_IMAGE_PIXELS = 1_000_000_000


class ImageSessions:
    """Each download thread owns its connection pool; no Session is shared between workers."""

    def __init__(self, site, injected=None):
        self.site, self.injected = site, injected
        self.local, self.lock, self.sessions = threading.local(), threading.Lock(), []

    def get(self):
        if self.injected is not None:
            return self.injected
        if not hasattr(self.local, "session"):
            session = requests.Session()
            session.trust_env = False
            session.headers["User-Agent"] = "Dataset-Studio/0.1 (local dataset archiver)"
            session.headers["Referer"] = {
                "danbooru": "https://danbooru.donmai.us/",
                "yandere": "https://yande.re/",
                "gelbooru": "https://gelbooru.com/",
            }[self.site]
            self.local.session = session
            with self.lock:
                self.sessions.append(session)
        return self.local.session

    def close(self):
        for session in self.sessions:
            session.close()


def paths(lib, job, item):
    directory = lib.cache / "updates" / job["id"]
    key = stable_id(item["observation_id"], job["definition"]["media"])
    return directory, key


def staging_bytes(job, item, resources):
    policy = job["definition"]["media"]
    extra = 0
    if policy["profile"] in {"custom", "webp-2048-q95"}:
        record = json.loads(item["record_json"])
        w, h = (
            record.get("image_width", record.get("width")),
            record.get("image_height", record.get("height")),
        )
        pixels = (
            w * h
            if isinstance(w, int) and isinstance(h, int) and w > 0 and h > 0
            else resources.config["max_image_pixels"]
        )
        edge = (policy.get("encoding") or {}).get("max_edge") if policy["profile"] == "custom" else 2048
        extra = 8 * min(pixels, resources.config["max_image_pixels"], edge * edge if edge else pixels)
    return resources.max_download_bytes * 4 + extra


def reserve(lib, job, item, resources, site, allow_external=False):
    directory, key = paths(lib, job, item)
    inspection = inspect_download(lib, job, item, site, allow_external)
    item["_inspection"] = inspection
    if "state" in inspection:
        return Reservation(resources, (directory, key))
    ready = cached(directory / (key + ".ready"), directory / (key + ".json"))
    return resources.try_reserve(
        directory,
        key,
        staging_bytes(job, item, resources),
        prepared=bool(ready and ready.get("state") == "stored"),
    )


def cached(path, receipt):
    if not path.exists() or not receipt.exists():
        return None
    try:
        value = json.loads(receipt.read_text(encoding="utf-8"))
        if value.get("sha256") == file_hash(path):
            return value
    except (ValueError, OSError):
        pass
    return None


def reusable(lib, observation, profile, allow_sample, existing="keep"):
    md5 = observation.get("md5")
    if not isinstance(md5, str) or not re.fullmatch("[a-fA-F0-9]{32}", md5):
        return None
    with online(lib) as (db, status):
        # Profile and source variant matter: a preserved HF thumbnail is not an original.
        profile_clause = " AND a.storage_profile=?" if existing == "match_profile" else ""
        args = [observation["post_id"], md5, int(status["served_seq"])]
        if profile_clause:
            args.append(profile)
        cur = db.execute(
            "SELECT a.asset_id,a.sha256,a.details_json FROM assets a JOIN objects o USING(sha256) "
            "WHERE a.post_id=? AND a.source_md5=? AND a.commit_seq<=? "
            + profile_clause
            + " ORDER BY a.commit_seq DESC,a.asset_id DESC LIMIT 32",
            args,
        )
        for aid, sha, details in cur:
            info = json.loads(details or "{}")
            if (
                existing == "keep"
                or allow_sample
                or info.get("selected_url_kind") in {"original", "file_url"}
            ):
                return {"asset_id": aid, "sha256": sha}
    return None


def inspect_download(lib, job, item, site, allow_external=False):
    policy = job["definition"]["media"]
    with online(lib) as (db, status):
        observation = record_by_id(
            db,
            "SELECT * FROM observations WHERE observation_id=? AND commit_seq<=?",
            (item["observation_id"], int(status["served_seq"])),
        )
    if observation is None:
        return {"state": "needs_review", "reason": "metadata_not_available"}
    old = reusable(
        lib, observation, profile_id(policy), policy["allow_sample"], policy.get("existing", "keep")
    )
    if old:
        return {"state": "reused", **old}
    record = json.loads(item["record_json"])
    if observation.get("is_deleted"):
        return {"state": "unavailable", "reason": "source_deleted"}
    if observation.get("is_banned"):
        return {"state": "unavailable", "reason": "source_restricted"}
    if str(observation.get("file_ext") or "").lower() in {"mp4", "webm", "zip", "swf"}:
        return {"state": "unavailable", "reason": "unsupported_media"}
    candidates = [("original", record.get("file_url"))]
    if policy["allow_sample"]:
        candidates += [
            ("sample", record.get("large_file_url")),
            ("sample", record.get("sample_url")),
            ("sample", record.get("jpeg_url")),
        ]
    seen, urls = set(), []
    for kind, url in candidates:
        if isinstance(url, str) and url and url not in seen:
            seen.add(url)
            parsed = urlsplit(url)
            if parsed.scheme != "https" or parsed.username or parsed.password:
                continue
            host = parsed.hostname or ""
            domain = {"danbooru": "donmai.us", "yandere": "yande.re", "gelbooru": "gelbooru.com"}[site.name]
            if not allow_external and host != domain and not host.endswith("." + domain):
                continue
            urls.append((kind, url))
    if not urls:
        return {"state": "unavailable", "reason": "no_image_url"}
    return {"observation": observation, "urls": urls}


def download(lib, job, item, site, resources, cancelled, sessions, progress=None):
    progress = progress or (lambda **_: None)
    inspection = item.get("_inspection") or inspect_download(
        lib, job, item, site, sessions.injected is not None
    )
    if "state" in inspection:
        return inspection
    observation, urls = inspection["observation"], inspection["urls"]
    directory, key = paths(lib, job, item)
    directory.mkdir(parents=True, exist_ok=True)
    ready, receipt = directory / (key + ".ready"), directory / (key + ".json")
    saved = cached(ready, receipt)
    if saved:
        return {**saved, "ready_path": str(ready)}
    raw, raw_receipt = directory / (key + ".downloaded"), directory / (key + ".download.json")
    saved = cached(raw, raw_receipt)
    if saved:
        return {**saved, "download_path": str(raw)}
    error = {"state": "failed", "reason": "download_failed", "retry_at": time.time() + 60}
    for kind, url in urls:
        try:
            error = fetch(directory, key, url, kind, observation, site, resources,
                          cancelled, sessions, progress)
            if error["state"] in {"downloaded", "stored"}:
                return error
        except UpdateError:
            raise
        except (OSError, ValueError) as failure:
            if isinstance(failure, OSError) and failure.errno == errno.ENOSPC:
                raise UpdateError("UPDATE_SPACE", "Waiting for SSD space during download") from None
            error = {"state": "needs_review", "reason": "image_download_or_storage_error"}
    return error



def encode_download(lib, job, item, downloaded, resources, cancelled, progress=None):
    progress = progress or (lambda **_: None)
    directory, key = paths(lib, job, item)
    raw, ready = directory / (key + ".downloaded"), directory / (key + ".ready")
    if str(raw) != downloaded.get("download_path"):
        raise UpdateError("UPDATE_INTEGRITY", "Downloaded image escaped its task")
    try:
        progress(phase="waiting_encode", current_post_id=item["post_id"])
        with Image.open(raw) as header:
            pixels = header.width * header.height
            if pixels > resources.config["max_image_pixels"]:
                raise UpdateError("UPDATE_RESOURCE_LIMIT", "Image exceeds configured pixel budget")
        # Source, conversion, alpha and output buffers; admission happens before reading full bytes.
        estimate = pixels * 16 + raw.stat().st_size * 3
        with resources.encoding(estimate, cancelled):
            if cancelled():
                raise UpdateError("CANCELLED", "Update paused")
            progress(phase="processing_image")
            started = time.perf_counter()
            data = raw.read_bytes()
            if digest(data) != downloaded["sha256"]:
                raise UpdateError("UPDATE_INTEGRITY", "Downloaded image hash mismatch")
            stored, ext, details = prepare_image(data, job["definition"]["media"])
            progress(encode_seconds_delta=time.perf_counter() - started)
            saved = {
                "state": "stored",
                "sha256": digest(stored),
                "stored_ext": ext,
                "stored_bytes": len(stored),
                "details": {
                    **details,
                    "selected_url_kind": downloaded["selected_url_kind"],
                    "original_md5_verified": downloaded["original_md5_verified"],
                },
            }
            # Admission estimates may use source dimensions; check actual output before writing it.
            resources.check_staged_size(
                directory,
                key,
                raw.stat().st_size
                + len(stored)
                + 2 * len(json.dumps(saved, ensure_ascii=False, indent=2).encode("utf-8"))
                + 4096,
            )
            temporary = ready.with_suffix(".tmp")
            with temporary.open("wb") as output:
                output.write(stored)
                output.flush()
                os.fsync(output.fileno())
            temporary.replace(ready)
            atomic_json(directory / (key + ".json"), saved)
        raw.unlink(missing_ok=True)
        (directory / (key + ".download.json")).unlink(missing_ok=True)
        progress(phase="ready")
        return {**saved, "ready_path": str(ready)}
    except ImagePolicyError:
        return {"state": "needs_review", "reason": "image_policy_rejected"}
    except (OSError, ValueError, Image.DecompressionBombError) as error:
        if isinstance(error, OSError) and error.errno == errno.ENOSPC:
            raise UpdateError("UPDATE_SPACE", "Waiting for SSD space during encoding") from None
        return {"state": "needs_review", "reason": "image_decode_or_storage_error"}


def prepare(lib, job, item, site, resources, cancelled, image_http=None, progress=None):
    """Standalone compatibility entry; production uses the separate pipeline stages."""
    directory, key = paths(lib, job, item)
    directory.mkdir(parents=True, exist_ok=True)
    sessions = ImageSessions(site.name, image_http)
    try:
        reservation = reserve(lib, job, item, resources, site, image_http is not None)
        if reservation is None:
            raise UpdateError("UPDATE_SPACE", "Waiting for SSD spool space")
        try:
            result = download(lib, job, item, site, resources, cancelled, sessions, progress)
            if result["state"] == "downloaded":
                result = encode_download(lib, job, item, result, resources, cancelled, progress)
            return result
        finally:
            reservation.release()
    finally:
        sessions.close()
