import hashlib
import io
import json
import os
import time
import uuid

import pyarrow as pa
import requests
from PIL import Image, ImageOps, __version__ as PILLOW_VERSION

from . import __version__
from .index import Index, original_record
from .library import Batch
from .metadata import asset, normalize, iso_time
from .png_compat import open_image
from .util import (
    IntegrityError,
    atomic_json,
    digest,
    failpoint,
    file_hash,
    now,
    read_json,
    retry_after_seconds,
    split_json_posts,
    stable_id,
    typed_value,
)


def response_stats(posts, observations, error, request):
    ids = [o["post_id"] for o in observations if o["post_id"] is not None]
    stats = {
        "count": len(posts),
        "min_id": min(ids) if ids else None,
        "max_id": max(ids) if ids else None,
        "valid": error is None,
        "error": error,
    }
    if request.get("strict_pagination") and not error:
        problem = None
        if len(ids) != len(posts) or len(set(ids)) != len(ids):
            problem = "分页包含无效或重复帖子 ID"
        elif request.get("mode") == "refresh":
            if set(ids) - set(request["expected_ids"]):
                problem = "刷新响应包含未请求的帖子 ID"
        elif ids:
            if ids != sorted(ids, reverse=True):
                problem = "分页不是按 ID 降序返回"
            elif min(ids) <= request["baseline"]:
                problem = "分页超出起始 ID 范围"
            elif request.get("before_id") and max(ids) >= request["before_id"]:
                problem = "分页与上一页重叠或没有前进"
            elif request.get("high_id") and max(ids) > request["high_id"]:
                problem = "分页超出本轮冻结的上界"
        if problem:
            stats["pagination_error"] = problem
    stats["accepted"] = (
        error is None and not stats.get("pagination_error") and request.get("status", 200) == 200
    )
    return stats


def next_api_settings(active, stats):
    if not stats["valid"] or not stats.get("accepted", True) or stats.get("pagination_error"):
        return {}
    if stats["count"] == 0:
        return {"active_api_run": None, "api_watermark": active["high_id"] or active["baseline"]}
    if stats["min_id"] is None or stats["min_id"] <= active["baseline"]:
        return {}
    if active["before_id"] and stats["min_id"] >= active["before_id"]:
        return {}
    return {
        "active_api_run": {
            **active,
            "page": active["page"] + 1,
            "before_id": stats["min_id"],
            "high_id": active["high_id"] or stats["max_id"],
        }
    }


def publication_context(observation, request):
    if request.get("mode") == "refresh":
        previous = request.get("publication_context", {}).get(str(observation["post_id"]), {})
        observation["publication_kind"] = previous.get("publication_kind") or request.get(
            "default_publication_kind", "metadata_refresh"
        )
        observation["publication_group"] = previous.get("publication_group")
    return observation


def capture_api(
    lib,
    body,
    key=None,
    observed_at=None,
    request_info=None,
    update_settings=None,
    ingest_run_id=None,
    sync_index=True,
):
    key = key or stable_id("offline-api", digest(body))
    observed_at = iso_time(observed_at) if observed_at else now()
    with lib.writer_lock():
        old = lib.committed_key(key)
        if old:
            with lib.journal() as db:
                m = json.loads(
                    db.execute("SELECT manifest_json FROM commits WHERE dedupe_key=?", (key,)).fetchone()[0]
                )
            return {"status": "already_committed", **old, **m["source"].get("response_stats", {})}
        info = {
            "kind": "api",
            "source_key": key,
            "observed_at": observed_at,
            "response_sha256": digest(body),
            "request": request_info or {},
        }
        if ingest_run_id:
            info.update(ingest_run_id=ingest_run_id, ingest_role="api")
        batch = Batch(lib, key, info)
        # Persist before parsing. Even invalid JSON and unknown fields survive.
        batch.write_bytes("response_body.bin", body)
        failpoint("after_response_saved")
        error = None
        try:
            posts = split_json_posts(body)
        except (ValueError, UnicodeError) as e:
            posts, error = [], str(e)
        if posts:
            batch.add_source(
                pa.table(
                    {
                        "post_json": [raw for raw, _ in posts],
                        "response_sha256": [digest(body)] * len(posts),
                        "ordinal": list(range(len(posts))),
                    }
                )
            )
        for i, (_, record) in enumerate(posts):
            batch.observations.append(
                publication_context(
                    normalize(record, key, i, "api_json", i, observed_at, "exact"), info["request"]
                )
            )
        stats = response_stats(posts, batch.observations, error, info["request"])
        batch.source["response_stats"] = stats
        if not stats["accepted"]:
            batch.observations.clear()
        settings = (
            update_settings(stats)
            if update_settings and not error and not stats.get("pagination_error")
            else {}
        )
        seq = batch.commit(settings=settings)
    if sync_index:
        Index(lib).sync()
    return {"status": "committed", "seq": seq, "batch_id": batch.id, **stats}


def finish_saved_response(lib, path):
    """Recover a durable response saved before parsing/indexing completed."""
    batch = Batch.resume_metadata(lib, path)
    body = (path / "response_body.bin").read_bytes()
    if digest(body) != batch.source["response_sha256"]:
        raise IntegrityError("未提交 API 正文不完整；保留文件，不能推进游标")
    error = None
    try:
        posts = split_json_posts(body)
    except (ValueError, UnicodeError) as e:
        posts, error = [], str(e)
    if posts:
        batch.add_source(
            pa.table(
                {
                    "post_json": [r for r, _ in posts],
                    "response_sha256": [digest(body)] * len(posts),
                    "ordinal": list(range(len(posts))),
                }
            )
        )
    for i, (_, r) in enumerate(posts):
        batch.observations.append(
            publication_context(
                normalize(
                    r, batch.source["source_key"], i, "api_json", i, batch.source["observed_at"], "exact"
                ),
                batch.source.get("request", {}),
            )
        )
    active, request = lib.setting("active_api_run"), batch.source.get("request", {})
    stats = response_stats(posts, batch.observations, error, request)
    batch.source["response_stats"] = stats
    if not stats["accepted"]:
        batch.observations.clear()
    settings = {}
    if (
        active
        and request.get("run_id") == active["run_id"]
        and request.get("page") == active["page"]
        and not error
    ):
        settings = next_api_settings(active, stats)
    return batch.seal(settings=settings)


def session(proxy=None):
    s = requests.Session()
    s.trust_env = False
    s.headers["User-Agent"] = f"Danbooru-Store/{__version__} (local dataset archiver)"
    if proxy:
        s.proxies.update({"http": proxy, "https": proxy})
    return s


def api_get(lib, http, params, *, ingest_run_id=None, emit=None, control=None):
    def pause(seconds):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if control:
                control.checkpoint()
            time.sleep(max(0, min(0.2, end - time.monotonic())))

    if ingest_run_id:
        from .daily_state import DailyStore

        state = DailyStore(lib).get(ingest_run_id)["api_state"] or {}
        delay = max(0, state.get("next_request_after", 0) - time.time())
        if delay > lib.config.retry_max_seconds:
            raise RuntimeError(f"API 限速等待尚未结束，还需约 {delay:.0f} 秒")
        pause(delay)
    for attempt in range(lib.config.api_attempts):
        if control:
            control.checkpoint()
        try:
            response = http.get(
                "https://danbooru.donmai.us/posts.json", params=params, timeout=lib.config.timeout
            )
        except requests.RequestException as error:
            if emit:
                emit({"event": "api_request_retry", "attempt": attempt + 1, "error": str(error)})
            if attempt + 1 == lib.config.api_attempts:
                raise
            pause(min(lib.config.retry_base_seconds * 2**attempt, lib.config.retry_max_seconds))
            continue
        if response.status_code == 200:
            return response
        delay = retry_after_seconds(response.headers) or 0
        capture_api(
            lib,
            response.content,
            key=stable_id("api-http-error", uuid.uuid4().hex),
            request_info={
                "endpoint": "posts.json",
                "status": response.status_code,
                "parameters": params,
                "attempt": attempt + 1,
                "retry_after_at": time.time() + delay if delay else None,
            },
            ingest_run_id=ingest_run_id,
            sync_index=not ingest_run_id,
        )
        if response.status_code not in {429} and response.status_code < 500:
            raise RuntimeError(f"API HTTP {response.status_code}；响应已存档，游标未推进")
        if attempt + 1 == lib.config.api_attempts or delay > lib.config.retry_max_seconds:
            raise RuntimeError(f"API HTTP {response.status_code}；响应和等待时间已存档，游标未推进")
        pause(max(delay, min(lib.config.retry_base_seconds * 2**attempt, lib.config.retry_max_seconds)))
    raise RuntimeError("API 重试结束")


def fetch_api(
    lib, after_id=None, max_pages=None, proxy=None, emit=None, http=None, ingest_run_id=None, control=None
):
    if max_pages is not None and max_pages <= 0:
        raise ValueError("max_pages 必须为正数")
    lib.recover()
    active = lib.setting("active_api_run")
    if active and after_id is not None and after_id != active["baseline"]:
        raise ValueError("存在未完成采集，请先恢复原采集范围")
    if active and ingest_run_id and active["run_id"] != ingest_run_id:
        raise ValueError("存在属于其它任务的未完成 API 采集")
    baseline = after_id if after_id is not None else lib.setting("api_watermark")
    if not active and baseline is None:
        raise ValueError("首次采集必须显式指定 --after-id；迁移不会擅自设置 API 游标")
    if not active:
        active = dict(
            run_id=ingest_run_id or uuid.uuid4().hex,
            baseline=int(baseline),
            high_id=None,
            before_id=None,
            page=0,
        )
        with lib.writer_lock():
            lib.set_setting("active_api_run", active)
    own = http is None
    http = http or session(proxy)
    login, token = os.environ.get("DANBOORU_LOGIN"), os.environ.get("DANBOORU_API_KEY")
    if login and token:
        http.auth = (login, token)
    pages = 0
    try:
        while active:
            if control:
                control.checkpoint()
            params = {"limit": lib.config.page_size, "tags": f"id:>{active['baseline']}"}
            if ingest_run_id:
                params["tags"] += " order:id_desc"
            if active["before_id"]:
                params["page"] = f"b{active['before_id']}"
            response = api_get(lib, http, params, ingest_run_id=ingest_run_id, emit=emit, control=control)
            result = capture_api(
                lib,
                response.content,
                key=stable_id("api-page", active["run_id"], active["page"], digest(response.content)),
                request_info={
                    "endpoint": "posts.json",
                    "status": 200,
                    "parameters": params,
                    "run_id": active["run_id"],
                    "page": active["page"],
                    "content_type": response.headers.get("Content-Type"),
                    "strict_pagination": bool(ingest_run_id),
                    "baseline": active["baseline"],
                    "before_id": active["before_id"],
                    "high_id": active["high_id"],
                },
                update_settings=lambda stats: next_api_settings(active, stats),
                ingest_run_id=ingest_run_id,
                sync_index=not ingest_run_id,
            )
            if result.get("valid") is False:
                raise RuntimeError("API JSON 解析失败；原始正文已存档")
            if result.get("pagination_error"):
                raise IntegrityError(result["pagination_error"] + "；响应已存档，游标未推进")
            stored = lib.setting("active_api_run")
            if stored == active:
                raise RuntimeError("API 分页没有前进；响应已存档，需检查返回范围")
            active = stored
            pages += 1
            if emit:
                emit(
                    {
                        "event": "api_page_archived",
                        "page": pages,
                        "rows": result.get("count"),
                        "seq": result.get("seq"),
                    }
                )
            if max_pages and pages >= max_pages:
                break
            if active:
                time.sleep(lib.config.request_delay)
    finally:
        if own:
            http.close()
    return {
        "pages": pages,
        "status": "paused" if active else "complete",
        "watermark": lib.setting("api_watermark"),
    }


def prepare_image(data, profile):
    details = {
        "download_sha256": digest(data),
        "download_bytes": len(data),
        "storage_profile": profile,
        "processing_version": 3,
        "pillow_version": PILLOW_VERSION,
    }
    image, recovery = open_image(data)
    details.update(recovery)
    with image as im:
        fmt = (im.format or "bin").lower()
        ext = {"jpeg": "jpg", "tiff": "tif"}.get(fmt, fmt)
        details.update(
            source_width=im.width,
            source_height=im.height,
            frames=getattr(im, "n_frames", 1),
            source_format=fmt,
            source_image_info=typed_value(dict(im.info)),
        )
        if profile == "original" or getattr(im, "n_frames", 1) > 1:
            details.update(
                stored_width=im.width,
                stored_height=im.height,
                animation_preserved=getattr(im, "n_frames", 1) > 1,
            )
            return data, ext, details
        if profile != "webp-2048-q95":
            raise ValueError("未知图片保存策略")
        im = ImageOps.exif_transpose(im)
        im.load()
        details["source_image_info"] = typed_value(dict(image.info))
        im = im.convert("RGBA" if "A" in im.getbands() or "transparency" in im.info else "RGB")
        im.thumbnail((2048, 2048), Image.Resampling.LANCZOS)
        out = io.BytesIO()
        im.save(out, format="WEBP", quality=95, method=6)
        details.update(stored_width=im.width, stored_height=im.height, webp_quality=95, webp_method=6)
    return out.getvalue(), "webp", details


def candidate_image_urls(record):
    urls, seen = [], set()
    for kind in ["file_url", "large_file_url"]:
        url = record.get(kind)
        if url and url not in seen:
            seen.add(url)
            urls.append((kind, url))
    return urls


def download_body(lib, http, observation, url):
    directory = lib.cache / "downloads"
    directory.mkdir(exist_ok=True)
    key = stable_id(observation["observation_id"], url)
    target, marker = directory / (key + ".bin"), directory / (key + ".json")
    if target.exists() and marker.exists():
        m = read_json(marker)
        if target.stat().st_size > lib.config.max_download_bytes:
            raise ValueError("max_download_bytes: 下载缓存超过单文件资源限制")
        if m["sha256"] != file_hash(target):
            raise IntegrityError("下载暂存内容校验失败，保留原文件等待检查")
        return target, m
    temp = directory / (key + "." + uuid.uuid4().hex + ".partial")
    with http.get(url, timeout=lib.config.timeout, stream=True) as response:
        response.raise_for_status()
        if int(response.headers.get("Content-Length") or 0) > lib.config.max_download_bytes:
            raise ValueError("max_download_bytes: 下载内容超过单文件资源限制")
        h = hashlib.sha256()
        length = 0
        with temp.open("xb") as f:
            for data in response.iter_content(1024**2):
                length += len(data)
                if length > lib.config.max_download_bytes:
                    raise ValueError("max_download_bytes: 下载内容超过单文件资源限制")
                f.write(data)
                h.update(data)
            f.flush()
            os.fsync(f.fileno())
        m = {
            "sha256": h.hexdigest(),
            "bytes": temp.stat().st_size,
            "downloaded_at": now(),
            "status": response.status_code,
            "content_type": response.headers.get("Content-Type"),
        }
    temp.replace(target)
    atomic_json(marker, m)
    return target, m


def download_pending(lib, limit=None, proxy=None, profile=None, batch_items=128, emit=None, http=None):
    if batch_items <= 0 or limit is not None and limit <= 0:
        raise ValueError("下载数量限制必须为正数")
    lib.recover()
    index = Index(lib)
    with index.read() as con:
        # Latest observation of each post; archived failed/unsupported metadata is retained.
        query = "SELECT p.* FROM posts p WHERE source_kind='api_json' AND NOT has_image "
        query += "AND NOT EXISTS (SELECT 1 FROM events e WHERE e.observation_id=p.observation_id "
        query += "AND e.status='unavailable') ORDER BY post_id"
        if limit:
            query += f" LIMIT {int(limit)}"
        cur = con.execute(query)
        names = [c[0] for c in cur.description]
        pending = [dict(zip(names, r)) for r in cur.fetchall()]
    profile = profile or lib.config.storage_profile
    own = http is None
    http = http or session(proxy)
    done = failed = unavailable = 0
    raw_cache = {}
    try:
        for start in range(0, len(pending), batch_items):
            completed_downloads = []
            with lib.writer_lock(), index.object_lookup() as lookup:
                batch = Batch(
                    lib, stable_id("download-attempt", uuid.uuid4().hex), {"kind": "image_download"}
                )
                for o in pending[start : start + batch_items]:
                    record, _, _ = original_record(lib, o, raw_cache)
                    urls = candidate_image_urls(record)
                    ext = str(record.get("file_ext") or "").lower()
                    if (
                        not urls
                        or ext in {"mp4", "webm", "zip", "swf"}
                        or record.get("is_deleted") is True
                        or record.get("is_banned") is True
                    ):
                        unavailable += 1
                        batch.events.append(
                            dict(
                                observation_id=o["observation_id"],
                                status="unavailable",
                                reason="no_image_url_or_unsupported_media",
                                details_json="{}",
                                recorded_at=now(),
                            )
                        )
                        continue
                    error = None
                    for kind, url in urls:
                        try:
                            path, details = download_body(lib, http, o, url)
                            data = path.read_bytes()
                            if kind == "file_url" and record.get("md5"):
                                if hashlib.md5(data).hexdigest() != record["md5"].lower():
                                    raise IntegrityError("原站 MD5 与下载内容不符")
                            stored, stored_ext, processing = prepare_image(data, profile)
                            details.update(processing, selected_url=url, selected_url_kind=kind)
                            sha, stored_ext = batch.add_blob(stored, stored_ext, lookup)
                            batch.assets.append(asset(o, sha, stored_ext, len(stored), profile, details))
                            batch.events.append(
                                dict(
                                    observation_id=o["observation_id"],
                                    status="stored",
                                    reason="",
                                    details_json=json.dumps(details),
                                    recorded_at=now(),
                                )
                            )
                            done += 1
                            completed_downloads.append(path)
                            error = None
                            break
                        except (requests.RequestException, OSError, ValueError, IntegrityError) as e:
                            error = f"{type(e).__name__}: {e}"
                    if error:
                        failed += 1
                        batch.events.append(
                            dict(
                                observation_id=o["observation_id"],
                                status="failed",
                                reason=error,
                                details_json="{}",
                                recorded_at=now(),
                            )
                        )
                seq = batch.commit()
            index.sync()
            # The durable asset commit now owns the stored bytes. Failed spools are retained.
            for path in completed_downloads:
                if path.parent.resolve() != (lib.cache / "downloads").resolve():
                    raise IntegrityError("下载清理路径越界")
                path.unlink(missing_ok=True)
                path.with_suffix(".json").unlink(missing_ok=True)
            if emit:
                emit(
                    {
                        "event": "download_batch_committed",
                        "seq": seq,
                        "stored": done,
                        "failed": failed,
                        "unavailable": unavailable,
                    }
                )
    finally:
        if own:
            http.close()
    return {
        "stored": done,
        "failed": failed,
        "unavailable": unavailable,
        "status": "incomplete" if failed else "complete",
    }
