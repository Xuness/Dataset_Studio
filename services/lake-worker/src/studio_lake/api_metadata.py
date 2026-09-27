"""Site-specific projections over losslessly retained API objects."""

from datetime import datetime, timezone
from email.utils import parsedate_to_datetime
import json

from .metadata import normalize

API_KINDS = {"api_json": "danbooru", "api_yandere_v1": "yandere", "api_gelbooru_v1": "gelbooru"}
SITE_KINDS = {v: k for k, v in API_KINDS.items()}


def source_time(value):
    if value is None or value == "":
        return None
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return datetime.fromtimestamp(value, timezone.utc).isoformat()
    try:
        dt = datetime.fromisoformat(str(value).replace("Z", "+00:00"))
    except ValueError:
        dt = parsedate_to_datetime(str(value))
    if dt.tzinfo is None:
        raise ValueError("source timestamp has no timezone")
    return dt.astimezone(timezone.utc).isoformat()


def normalize_api(
    record, source_key, row, kind, archive_row, observed_at=None, time_quality="exact", tag_types=None
):
    site = API_KINDS[kind]
    projected, issues = dict(record), []
    if site != "danbooru":
        for target, field in {
            "tag_string": "tags",
            "image_width": "width",
            "image_height": "height",
            "uploader_id": "creator_id",
            "preview_file_url": "preview_url",
        }.items():
            projected[target] = record.get(field)
        rating = record.get("rating")
        ratings = (
            {"s": "g", "q": "q", "e": "e"}
            if site == "yandere"
            else {
                "safe": "g",
                "general": "g",
                "sensitive": "s",
                "questionable": "q",
                "explicit": "e",
            }
        )
        projected["rating"] = ratings.get(rating)
        if rating is not None and projected["rating"] is None:
            issues.append({"field": "rating", "reason": "unknown site rating retained in raw"})
        status = record.get("status")
        if status in {"active", "deleted", "pending", "flagged"}:
            for value in ("deleted", "pending", "flagged"):
                projected["is_" + value] = status == value
        projected["large_file_url"] = record.get("sample_url")
        if site == "gelbooru":
            projected["updated_at"] = record.get("change")
            # File extension inferred from the documented source filename, never a saved WebP.
            name = record.get("image")
            if isinstance(name, str) and "." in name:
                projected["file_ext"] = name.rsplit(".", 1)[1].lower()
    for field in ("created_at", "updated_at"):
        try:
            projected[field] = source_time(projected.get(field))
        except (ValueError, TypeError, OverflowError, OSError):
            projected[field] = None
            issues.append({"field": field, "reason": "ambiguous time retained in raw"})
    out = normalize(projected, source_key, row, kind, archive_row, observed_at, time_quality)
    if site != "danbooru" and tag_types is not None:
        tags = (projected.get("tag_string") or "").split(" ")
        tags = [tag for tag in tags if tag]
        if all(tag_types.get(tag) is not None for tag in tags):
            groups = {0: "general", 1: "artist", 3: "copyright", 4: "character"}
            if site == "gelbooru":
                groups[5] = "meta"
            for category, label in groups.items():
                out["tag_string_" + label] = " ".join(tag for tag in tags if tag_types.get(tag) == category)
        else:
            issues.append(
                {
                    "field": "tag_categories",
                    "reason": "source dictionary incomplete; unknown categories retained as NULL",
                }
            )
    out.update(source_priority=100, publication_kind="api")
    out["issues_json"] = json.dumps(json.loads(out["issues_json"]) + issues, ensure_ascii=False)
    return out
