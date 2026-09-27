"""Validated HTTP ranges over durable, bounded SSD checkpoints."""

import hashlib
import json
import os
import re
import time

import requests

from ..util import atomic_json, digest, now, retry_after_seconds
from . import rate
from .sites import UpdateError

CHUNK = 256 * 1024
CHECKPOINT_BYTES = 4 * 1024**2
CHECKPOINT_SECONDS = 2


def strong_etag(value):
    return value if isinstance(value, str) and re.fullmatch(r'"[\x21\x23-\x7e]{0,1022}"', value) else None


class Partial:
    def __init__(self, directory, key, url, kind, md5, limit):
        self.path = directory / (key + ".partial")
        self.receipt = directory / (key + ".partial.json")
        self.raw = directory / (key + ".downloaded")
        self.raw_receipt = directory / (key + ".download.json")
        self.identity = {"version": 1, "url_sha256": digest(url.encode()), "kind": kind, "md5": md5}
        self.limit = limit
        self.reset_memory()
        self.load()

    def reset_memory(self):
        self.size, self.total, self.etag, self.complete, self.encoded = 0, None, None, False, False
        self.sha, self.md5 = hashlib.sha256(), hashlib.md5()

    def discard(self):
        self.path.unlink(missing_ok=True)
        self.receipt.unlink(missing_ok=True)
        self.reset_memory()

    def load(self):
        # Atomic JSON can leave an uncommitted temporary receipt when the process is killed.
        for path in self.receipt.parent.glob(self.receipt.name + ".*.tmp"):
            path.unlink(missing_ok=True)
        try:
            saved = json.loads(self.receipt.read_text(encoding="utf-8"))
            count = saved["bytes"]
            if (
                any(saved.get(k) != v for k, v in self.identity.items())
                or type(count) is not int
                or count < 0
                or saved.get("total") is not None
                and (type(saved["total"]) is not int or saved["total"] < count)
                or self.path.stat().st_size < count
            ):
                raise ValueError("Invalid download checkpoint")
            if count > self.limit or (saved.get("total") or 0) > self.limit:
                raise UpdateError("UPDATE_RESOURCE_LIMIT", "Saved image exceeds configured byte limit")
            self.total, self.etag = saved.get("total"), strong_etag(saved.get("etag"))
            self.complete, self.encoded = saved.get("complete") is True, saved.get("encoded") is True
            if not self.complete and (self.encoded or not (self.identity["md5"] or self.etag)):
                raise ValueError("Partial representation has no reliable validator")
            with self.path.open("r+b") as source:
                while self.size < count:
                    data = source.read(min(CHUNK, count - self.size))
                    if not data:
                        raise ValueError("Short download checkpoint")
                    self.size += len(data)
                    self.sha.update(data)
                    self.md5.update(data)
                if self.sha.hexdigest() != saved["sha256"]:
                    raise ValueError("Changed download checkpoint")
                # A process may have died after writing bytes but before checkpoint publication.
                source.truncate(count)
        except (FileNotFoundError, ValueError, KeyError, TypeError, AttributeError):
            self.discard()

    def checkpoint(self, output, *, complete=False):
        output.flush()
        os.fsync(output.fileno())
        self.complete = complete
        atomic_json(
            self.receipt,
            {
                **self.identity,
                "bytes": self.size,
                "sha256": self.sha.hexdigest(),
                "total": self.total,
                "etag": self.etag,
                "complete": complete,
                "encoded": self.encoded,
            },
        )

    def finish(self):
        if self.identity["md5"] and self.md5.hexdigest() != self.identity["md5"]:
            self.discard()
            return {"state": "needs_review", "reason": "original_md5_mismatch"}
        saved = {
            "state": "downloaded",
            "sha256": self.sha.hexdigest(),
            "download_bytes": self.size,
            "selected_url_kind": self.identity["kind"],
            "original_md5_verified": bool(self.identity["md5"]),
        }
        # Receipt first: either a complete checkpoint or the renamed, verified raw file survives a kill.
        atomic_json(self.raw_receipt, saved)
        self.path.replace(self.raw)
        self.receipt.unlink(missing_ok=True)
        return {**saved, "download_path": str(self.raw)}


def transport_error(error, post_id, partial, started):
    names, current, seen = [], error, set()
    while current is not None and id(current) not in seen and len(names) < 5:
        seen.add(id(current))
        names.append(type(current).__name__)
        current = current.__cause__ or current.__context__
    # Exception text can contain URLs or credentials. Only bounded, structural diagnostics leave the worker.
    return {
        "post_id": post_id,
        "exception": "/".join(names),
        "received_bytes": partial.size,
        "resumable_bytes": partial.size
        if not partial.encoded and (partial.identity["md5"] or partial.etag)
        else 0,
        "elapsed_seconds": time.perf_counter() - started,
        "at": now(),
    }


def fetch(directory, key, url, kind, observation, site, resources, cancelled, sessions, progress):
    md5 = observation.get("md5") if kind == "original" else None
    md5 = md5.lower() if isinstance(md5, str) and re.fullmatch("[a-fA-F0-9]{32}", md5) else None
    partial = Partial(directory, key, url, kind, md5, resources.max_download_bytes)
    if (
        partial.complete
        or partial.size
        and (partial.size == partial.total or md5 is not None and partial.md5.hexdigest() == md5)
    ):
        progress(phase="verifying", current_bytes=partial.size, current_total_bytes=partial.total)
        return partial.finish()
    started = time.perf_counter()
    try:
        # A 416 may mean the resource changed. Retry once without a range, through the same rate lane.
        for _ in range(2):
            if cancelled():
                raise UpdateError("CANCELLED", "Update paused")
            offset = partial.size
            headers = {"Accept-Encoding": "identity"}
            if offset:
                headers["Range"] = f"bytes={offset}-"
                if partial.etag:
                    headers["If-Range"] = partial.etag
            progress(
                phase="rate_wait", current_bytes=offset, current_total_bytes=partial.total, resume_from=0
            )
            if site.rate_root is not None and sessions.injected is None:
                rate.wait_start(
                    site.rate_root,
                    site.name,
                    cancelled,
                    lambda: resources.config["sites"][site.name]["image_requests_per_second"],
                )
            progress(phase="connecting", image_requests_delta=1)
            connected = time.perf_counter()
            try:
                response = sessions.get().get(
                    url, stream=True, timeout=(10, 45), allow_redirects=False, headers=headers
                )
            finally:
                progress(connect_seconds_delta=time.perf_counter() - connected)
            with response:
                status = response.status_code
                if status == 416 and offset:
                    partial.discard()
                    continue
                if status not in {200, 206}:
                    delay = retry_after_seconds(response.headers) or 60
                    if status in {429, 503}:
                        progress(throttled_requests_delta=1)
                        if site.rate_root is not None:
                            rate.cooldown(site.rate_root, site.name, delay, image=True)
                    return {
                        "state": "failed" if status == 429 or status >= 500 else "needs_review",
                        "reason": f"image_http_{status}",
                        "retry_at": time.time() + delay,
                    }
                content_length = response.headers.get("Content-Length", "")
                content_length = int(content_length) if str(content_length).isdigit() else None
                encoded = response.headers.get("Content-Encoding", "identity").lower() != "identity"
                etag = strong_etag(response.headers.get("ETag"))
                expected_end = None
                if status == 206:
                    match = re.fullmatch(
                        r"bytes (\d+)-(\d+)/(\d+)", response.headers.get("Content-Range", "")
                    )
                    valid = False
                    if match and not encoded:
                        first, end, total = map(int, match.groups())
                        valid = (
                            first == offset
                            and first <= end < total
                            and (content_length is None or content_length == end - first + 1)
                            and (partial.total is None or partial.total == total)
                            and (partial.etag is None or etag is None or partial.etag == etag)
                        )
                    if not valid:
                        partial.discard()
                        return {
                            "state": "failed",
                            "reason": "image_range_invalid",
                            "retry_at": time.time() + 60,
                        }
                    partial.total, expected_end = total, end + 1
                    if offset:
                        progress(resumed_requests_delta=1, resume_from=offset)
                else:
                    # Range ignored or If-Range validator changed: a 200 is a new complete representation.
                    partial.discard()
                    offset = 0
                    partial.total = content_length if not encoded else None
                partial.etag, partial.encoded = etag or partial.etag, encoded
                if partial.total is not None and partial.total > partial.limit:
                    raise UpdateError("UPDATE_RESOURCE_LIMIT", "Image exceeds configured byte limit")
                received, reported, sampled = 0, 0, time.perf_counter()
                checkpoint_at, checkpoint_size = sampled, partial.size
                transfer_started = sampled
                progress(phase="downloading", current_bytes=offset, current_total_bytes=partial.total)
                try:
                    with partial.path.open("ab" if offset else "wb") as output:
                        partial.checkpoint(output)
                        try:
                            for chunk in response.iter_content(CHUNK):
                                if cancelled():
                                    raise UpdateError("CANCELLED", "Update paused")
                                if partial.size + len(chunk) > partial.limit:
                                    raise UpdateError(
                                        "UPDATE_RESOURCE_LIMIT", "Image exceeds configured byte limit"
                                    )
                                if partial.total is not None and partial.size + len(chunk) > partial.total:
                                    raise requests.exceptions.ChunkedEncodingError(
                                        "Image exceeded declared size"
                                    )
                                resources.bandwidth(len(chunk), cancelled)
                                output.write(chunk)
                                partial.size += len(chunk)
                                received += len(chunk)
                                partial.sha.update(chunk)
                                partial.md5.update(chunk)
                                stamp = time.perf_counter()
                                if stamp - sampled >= 0.5:
                                    progress(
                                        current_bytes=partial.size, downloaded_bytes_delta=received - reported
                                    )
                                    reported, sampled = received, stamp
                                if (
                                    stamp - checkpoint_at >= CHECKPOINT_SECONDS
                                    or partial.size - checkpoint_size >= CHECKPOINT_BYTES
                                ):
                                    partial.checkpoint(output)
                                    checkpoint_at, checkpoint_size = stamp, partial.size
                            if expected_end is not None and partial.size != expected_end:
                                raise requests.exceptions.ChunkedEncodingError("Short range response")
                            if partial.total is not None and partial.size != partial.total:
                                raise requests.exceptions.ChunkedEncodingError("Short image response")
                        finally:
                            partial.checkpoint(output)
                        partial.checkpoint(output, complete=True)
                finally:
                    progress(
                        downloaded_bytes_delta=received - reported,
                        download_seconds_delta=time.perf_counter() - transfer_started,
                    )
                progress(phase="verifying", current_bytes=partial.size)
                result = partial.finish()
                if result["state"] == "downloaded":
                    progress(phase="waiting_encode")
                return result
        return {"state": "failed", "reason": "image_range_invalid", "retry_at": time.time() + 60}
    except requests.RequestException as error:
        detail = transport_error(error, observation["post_id"], partial, started)
        progress(transport_failures_delta=1, last_transfer_error=detail)
        return {
            "state": "failed",
            "reason": "image_transport_failed",
            "retry_at": time.time() + 60,
            "transfer_error": detail,
        }
