"""Bounded SSD downloads and explicit media-variant reuse; no network inside lake transactions."""

from contextlib import contextmanager
import hashlib
import json
import os
import re
import shutil
import threading
import time
from urllib.parse import urlsplit

from PIL import Image
import requests

from ..image_policy import prepare_image, profile_id, ImagePolicyError
from ..util import atomic_json, digest, file_hash, retry_after_seconds, stable_id
from .archive import online, record_by_id
from .sites import UpdateError


class Resources:
    def __init__(self, max_download_bytes=128 * 1024**2, spool_bytes=8 * 1024**3, reserve_bytes=2 * 1024**3):
        self.max_download_bytes, self.spool_bytes, self.reserve_bytes = (
            max_download_bytes,
            spool_bytes,
            reserve_bytes,
        )
        self.encode = threading.Semaphore(1)
        self.lock = threading.Lock()
        self.reserved = 0
        self.roots = set()

    @contextmanager
    def reservation(self, root):
        need = self.max_download_bytes * 3
        with self.lock:
            self.roots.add(root.parent)
            used = sum(
                p.stat().st_size
                for directory in self.roots
                for p in directory.glob("*/*")
                if p.is_file() and p.suffix in {".ready", ".partial", ".tmp"}
            )
            if (
                used + self.reserved + need > self.spool_bytes
                or shutil.disk_usage(root).free < self.reserve_bytes + need
            ):
                raise UpdateError("UPDATE_SPACE", "Waiting for SSD spool space")
            self.reserved += need
        try:
            yield
        finally:
            with self.lock:
                self.reserved -= need


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


def prepare(lib, job, item, site, resources, cancelled, image_http=None, progress=None):
    progress = progress or (lambda **_: None)
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
            if image_http is None and host != domain and not host.endswith("." + domain):
                continue
            urls.append((kind, url))
    if not urls:
        return {"state": "unavailable", "reason": "no_image_url"}
    directory = lib.cache / "updates" / job["id"]
    directory.mkdir(parents=True, exist_ok=True)
    key = stable_id(item["observation_id"], policy)
    ready, receipt = directory / (key + ".ready"), directory / (key + ".json")
    if ready.exists() and receipt.exists():
        saved = json.loads(receipt.read_text(encoding="utf-8"))
        if saved["sha256"] == file_hash(ready):
            return {**saved, "ready_path": str(ready)}
    own = image_http is None
    http = image_http or requests.Session()
    http.trust_env = False
    http.headers["User-Agent"] = "Dataset-Studio/0.1 (local dataset archiver)"
    http.headers["Referer"] = {
        "danbooru": "https://danbooru.donmai.us/",
        "yandere": "https://yande.re/",
        "gelbooru": "https://gelbooru.com/",
    }[site.name]
    error = {"state": "failed", "reason": "download_failed", "retry_at": time.time() + 60}
    try:
        with resources.reservation(directory):
            for kind, url in urls:
                if cancelled():
                    raise UpdateError("CANCELLED", "Update paused")
                partial = directory / (key + ".partial")
                try:
                    with http.get(url, stream=True, timeout=(10, 45), allow_redirects=False) as response:
                        if response.status_code != 200:
                            delay = retry_after_seconds(response.headers) or 60
                            error = {
                                "state": "failed"
                                if response.status_code == 429 or response.status_code >= 500
                                else "needs_review",
                                "reason": f"image_http_{response.status_code}",
                                "retry_at": time.time() + delay,
                            }
                            continue
                        length = 0
                        reported = 0
                        sampled = time.monotonic()
                        total = response.headers.get("Content-Length", "")
                        total = (
                            int(total)
                            if str(total).isdigit() and not response.headers.get("Content-Encoding")
                            else None
                        )
                        progress(
                            phase="downloading",
                            current_post_id=item["post_id"],
                            current_bytes=0,
                            current_total_bytes=total,
                        )
                        with partial.open("wb") as output:
                            for chunk in response.iter_content(256 * 1024):
                                if cancelled():
                                    raise UpdateError("CANCELLED", "Update paused")
                                length += len(chunk)
                                if length > resources.max_download_bytes:
                                    raise UpdateError(
                                        "UPDATE_RESOURCE_LIMIT", "Image exceeds configured byte limit"
                                    )
                                output.write(chunk)
                                stamp = time.monotonic()
                                if stamp - sampled >= 0.5:
                                    progress(
                                        current_bytes=length,
                                        downloaded_bytes_delta=length - reported,
                                        download_rate_bps=(length - reported) / (stamp - sampled),
                                    )
                                    reported, sampled = length, stamp
                            output.flush()
                            os.fsync(output.fileno())
                        progress(current_bytes=length, downloaded_bytes_delta=length - reported)
                    data = partial.read_bytes()
                    if (
                        kind == "original"
                        and observation.get("md5")
                        and hashlib.md5(data).hexdigest() != observation["md5"].lower()
                    ):
                        return {"state": "needs_review", "reason": "original_md5_mismatch"}
                    with resources.encode:
                        progress(phase="processing_image")
                        import io

                        with Image.open(io.BytesIO(data)) as header:
                            if header.width * header.height > 100_000_000:
                                raise UpdateError(
                                    "UPDATE_RESOURCE_LIMIT", "Image decode exceeds pixel budget"
                                )
                        stored, ext, details = prepare_image(data, policy)
                    temporary = ready.with_suffix(".tmp")
                    with temporary.open("wb") as output:
                        output.write(stored)
                        output.flush()
                        os.fsync(output.fileno())
                    temporary.replace(ready)
                    saved = {
                        "state": "stored",
                        "sha256": digest(stored),
                        "stored_ext": ext,
                        "stored_bytes": len(stored),
                        "details": {
                            **details,
                            "selected_url_kind": kind,
                            "original_md5_verified": kind == "original" and bool(observation.get("md5")),
                        },
                    }
                    atomic_json(receipt, saved)
                    partial.unlink(missing_ok=True)
                    return {**saved, "ready_path": str(ready)}
                except UpdateError:
                    raise
                except requests.RequestException:
                    error = {
                        "state": "failed",
                        "reason": "image_transport_failed",
                        "retry_at": time.time() + 60,
                    }
                except ImagePolicyError:
                    return {"state": "needs_review", "reason": "image_policy_rejected"}
                except (OSError, ValueError, Image.DecompressionBombError):
                    error = {"state": "needs_review", "reason": "image_decode_or_storage_error"}
                finally:
                    partial.unlink(missing_ok=True)
    finally:
        if own:
            http.close()
    return error
