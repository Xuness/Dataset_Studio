"""Lossless captures and conservative Pixiv work/page normalization."""

import json
import re
from urllib.parse import urlsplit

from . import ADAPTER_VERSION
from ...media_lake.schema import NORMALIZER, canonical, utc
from ...util import IntegrityError, digest, stable_id


def source_id(value):
    value = str(value)
    if not re.fullmatch(r"[1-9][0-9]{0,19}", value):
        raise IntegrityError("Invalid Pixiv source identity")
    return value


def integer(value, minimum=0):
    return value if isinstance(value, int) and not isinstance(value, bool) and minimum <= value < 2**63 else None


def media_url(value):
    if not isinstance(value, str) or len(value) > 8192:
        raise IntegrityError("Missing or invalid Pixiv media URL")
    url = urlsplit(value)
    if url.scheme != "https" or not (url.hostname == "pximg.net" or (url.hostname or "").endswith(".pximg.net")) or url.username or url.password or url.port not in (None, 443):
        raise IntegrityError("Pixiv media must use its HTTPS image hosts")
    return value


def visibility(viewer_key, *, login="anonymous", user_id=None, observed_at=None, r18="unknown", r18g="unknown", ai_display="unknown"):
    observed_at = utc(observed_at)
    policy = dict(login=login, user_id=user_id, r18=r18, r18g=r18g, ai_display=ai_display)
    verified = login == "authenticated" and all(v != "unknown" for v in (r18, r18g, ai_display))
    comparison = digest(canonical(dict(viewer_key=viewer_key, **policy)).encode())
    return dict(context_id=stable_id("visibility-context-v1", viewer_key, canonical(policy), observed_at),
                viewer_key=viewer_key, policy_json=canonical(policy), comparison_key=comparison,
                verified=verified, observed_at=observed_at)


def capture(library_id, context, receipt_id, endpoint, subject_kind, subject_id, body, *, observed_at=None, request=None, http_status=200, source_error=None):
    observed_at = utc(observed_at)
    return dict(capture_id=stable_id("capture-v1", library_id, receipt_id), request_receipt_id=receipt_id,
                endpoint=endpoint, subject_kind=subject_kind, subject_id=source_id(subject_id),
                request_json=canonical(request or {}), context_id=context["context_id"], observed_at=observed_at,
                http_status=http_status, source_error=source_error, adapter_version=ADAPTER_VERSION,
                raw_format="json", raw_bytes=len(body), raw_sha256=digest(body), raw_body=body)


def body(captured):
    payload = json.loads(captured["raw_body"])
    if not isinstance(payload, dict) or payload.get("error") is not False or "body" not in payload:
        raise IntegrityError("An unsuccessful Pixiv response cannot become a successful observation")
    return payload["body"]


def author(captured):
    data = body(captured)
    identity = source_id(data.get("userId", captured["subject_id"]))
    if identity != captured["subject_id"]:
        raise IntegrityError("Author response identity mismatch")
    record = dict(observation_id=stable_id("author-observation-v1", captured["capture_id"], identity, NORMALIZER),
                  author_id=identity, capture_id=captured["capture_id"], observed_at=captured["observed_at"],
                  normalizer_version=NORMALIZER, display_name=data.get("name"), profile_json=canonical(data))
    return {"authors": [{"author_id": identity}], "author_observations": [record]}


def work(captured):
    data = body(captured)
    identity = source_id(data.get("id"))
    if identity != captured["subject_id"]:
        raise IntegrityError("Work response identity mismatch")
    author_id = source_id(data["userId"])
    issues = []

    def date(key):
        if not data.get(key):
            return None
        try:
            return utc(data[key])
        except (TypeError, ValueError, AttributeError):
            issues.append("invalid_" + key)
            return None

    observation = stable_id("work-observation-v1", captured["capture_id"], identity, NORMALIZER)
    record = dict(observation_id=observation, work_id=identity, capture_id=captured["capture_id"],
                  observed_at=captured["observed_at"], normalizer_version=NORMALIZER, author_id=author_id,
                  work_type={0: "illustration", 1: "manga", 2: "ugoira"}.get(data.get("illustType"), "unknown"),
                  title=data.get("title"), caption_html=data.get("description"), page_count=integer(data.get("pageCount")),
                  created_at=date("createDate"), updated_at=date("uploadDate"),
                  source_fields_json=canonical({"pixiv": {"x_restrict": integer(data.get("xRestrict")),
                      "ai_type": integer(data.get("aiType")), "bookmark_count": integer(data.get("bookmarkCount")),
                      "view_count": integer(data.get("viewCount")), "like_count": integer(data.get("likeCount"))}}),
                  issues_json=canonical(issues))
    from ...media_lake.content import revision

    fields = json.loads(record["source_fields_json"])
    fields["pixiv"]["content_revision"] = revision(data)
    record["source_fields_json"] = canonical(fields)
    tags = []
    for ordinal, value in enumerate(data.get("tags", {}).get("tags", [])):
        if not isinstance(value, dict) or not isinstance(value.get("tag"), str):
            raise IntegrityError("Invalid Pixiv tag entry")
        tags.append(dict(observation_id=observation, ordinal=ordinal, tag=value["tag"],
                         translations_json=canonical(value.get("translation") or {}),
                         locked=value.get("locked") if isinstance(value.get("locked"), bool) else None))
    return {"authors": [{"author_id": author_id}], "works": [{"work_id": identity}],
            "work_observations": [record], "work_tags": tags}


def manifest(captured, detail):
    data = body(captured)
    identity = captured["subject_id"]
    if identity != detail["work_id"]:
        raise IntegrityError("Media response belongs to another work")
    animation = detail["work_type"] == "ugoira"
    mid = stable_id("media-manifest-v1", captured["capture_id"], identity, detail["observation_id"], NORMALIZER)
    entries, frames = [], []
    if animation:
        media_id = stable_id("media-entry-v1", mid, "animation:0")
        variant = "originalSrc" if data.get("originalSrc") else "src"
        entries.append(dict(media_id=media_id, manifest_id=mid, work_id=identity, slot_key="animation:0", ordinal=0,
                            kind="ugoira", width=None, height=None, source_url=media_url(data.get(variant)),
                            source_variant="ugoira_" + variant, auxiliary_urls_json=canonical({k: data[k] for k in ("src", "originalSrc") if data.get(k)})))
        for ordinal, frame in enumerate(data.get("frames", [])):
            filename = frame.get("file", "")
            if not re.fullmatch(r"[A-Za-z0-9_.-]{1,255}", filename) or filename in {".", ".."} or integer(frame.get("delay")) is None:
                raise IntegrityError("Invalid animation frame manifest")
            frames.append(dict(media_id=media_id, ordinal=ordinal, file_name=filename, delay_ms=frame["delay"]))
        complete, reason, expected = bool(frames), None if frames else "empty_frames", 1
    else:
        if not isinstance(data, list):
            raise IntegrityError("Invalid Pixiv pages response")
        for ordinal, page in enumerate(data):
            slot = "page:" + str(ordinal)
            entries.append(dict(media_id=stable_id("media-entry-v1", mid, slot), manifest_id=mid, work_id=identity,
                                slot_key=slot, ordinal=ordinal, kind="image", width=integer(page.get("width"), 1),
                                height=integer(page.get("height"), 1), source_url=media_url(page.get("urls", {}).get("original")),
                                source_variant="original", auxiliary_urls_json=canonical(page.get("urls", {}))))
        expected = detail["page_count"]
        complete = bool(entries) and (expected is None or len(entries) == expected)
        reason = None if complete else "empty_or_count_mismatch"
    record = dict(manifest_id=mid, work_id=identity, capture_id=captured["capture_id"],
                  detail_observation_id=detail["observation_id"], context_id=captured["context_id"],
                  observed_at=captured["observed_at"], normalizer_version=NORMALIZER,
                  kind="ugoira" if animation else "image_pages", expected_count=expected, item_count=len(entries),
                  complete=complete, reason=reason)
    return {"media_manifests": [record], "media_entries": entries, "animation_frames": frames}


def discovery(captured, *, job_id, stream_key, relation, root_kind, members, page_key="0", next_cursor=None, exhausted=True):
    snapshot_id = stable_id("discovery-v1", captured["capture_id"], stream_key, "pixiv-plan-v1")
    snapshot = dict(snapshot_id=snapshot_id, capture_id=captured["capture_id"], root_kind=root_kind,
                    root_id=captured["subject_id"], relation=relation, context_id=captured["context_id"],
                    observed_at=captured["observed_at"], scan_id=job_id, stream_key=stream_key, planner_version="pixiv-plan-v1",
                    page_key=str(page_key), next_cursor_json=canonical(next_cursor) if next_cursor is not None else None,
                    page_complete=True, traversal_exhausted=exhausted)
    entries = [dict(snapshot_id=snapshot_id, ordinal=i, target_kind=kind, target_id=source_id(identity)) for i, (kind, identity) in enumerate(members)]
    return {"discovery_snapshots": [snapshot], "discovery_members": entries}


def directory(captured, job_id):
    data = body(captured)
    ids = set()
    for kind in ("illusts", "manga"):
        values = data.get(kind)
        # Pixiv uses [] for an empty map. A missing field is a parsing failure.
        if not isinstance(values, dict) and values != []:
            raise IntegrityError("Author directory is missing a recognized work collection")
        ids.update(source_id(v) for v in values)
    return discovery(captured, job_id=job_id, stream_key="author:" + captured["subject_id"] + ":directory",
                     relation="author_works", root_kind="author", members=[("work", v) for v in sorted(ids, key=int)])
