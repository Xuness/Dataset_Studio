"""Versioned projections; the untouched Arrow rows remain the authoritative metadata."""

import json

from .metadata import integer, normalize
from .util import IntegrityError

HF_KINDS = {"hf_yandere_v1": "yandere", "hf_gelbooru_v1": "gelbooru"}
RATINGS = {
    "yandere": {"s": "g", "q": "q", "e": "e"},
    "gelbooru": {"general": "g", "safe": "g", "sensitive": "s", "questionable": "q", "explicit": "e"},
}


def tags_text(value, field, issues):
    if value is None:
        return None
    if not isinstance(value, list):
        raise IntegrityError(f"HF {field} must be a list of nonempty strings")
    unusual = []
    for position, tag in enumerate(value):
        if not isinstance(tag, str) or not tag:
            raise IntegrityError(f"HF {field}[{position}] must be a nonempty string")
        # The canonical tag index splits only on U+0020. Other whitespace can
        # occur inside a source token and is safe to preserve literally; do not
        # strip, split, or replace it and invent a different tag identity.
        if " " in tag:
            raise IntegrityError(f"HF {field}[{position}] contains the U+0020 tag delimiter")
        if any(c.isspace() for c in tag):
            unusual.append(position)
    if unusual:
        issues.append(
            {
                "field": field,
                "reason": "whitespace other than U+0020 retained literally inside source tags",
                "tag_indexes": unusual,
            }
        )
    return " ".join(value)


def normalize_hf(record, source_key, row, kind, archive_row, observed_at=None, time_quality="unknown"):
    site = HF_KINDS[kind]
    extra = record.get("extra")
    post_id = integer(record.get("image_id"))
    if not isinstance(extra, dict) or post_id is None or post_id <= 0 or integer(extra.get("id")) != post_id:
        raise IntegrityError("HF image_id and extra.id must be the same positive integer")
    issues = []
    try:
        tag_string = tags_text(extra.get("tags"), "extra.tags", issues)
        character_tags = tags_text(extra.get("tags_character"), "extra.tags_character", issues)
    except IntegrityError as error:
        raise IntegrityError(f"{site} post {post_id}, source row {row}: {error}") from error
    projected = {
        "id": post_id,
        "rating": RATINGS[site].get(extra.get("rating")),
        "tag_string": tag_string,
        "tag_string_character": character_tags,
        "created_at": record.get("image_created_at"),
        "updated_at": extra.get("updated_at"),
        "image_width": extra.get("width"),
        "image_height": extra.get("height"),
        "uploader_id": extra.get("creator_id"),
        **{
            k: extra.get(k)
            for k in ["score", "source", "md5", "parent_id", "file_url", "file_ext", "file_size"]
        },
    }
    # Gelbooru's top-level file_size/dimensions/SHA describe the stored WebP, not
    # the site's original image. Unknown values stay NULL, including naive times.
    out = normalize(projected, source_key, row, kind, archive_row, observed_at, time_quality)
    if extra.get("rating") not in RATINGS[site]:
        issues.append({"field": "rating", "reason": "unmapped source rating; raw value retained"})
    if issues:
        out["issues_json"] = json.dumps(json.loads(out["issues_json"]) + issues, ensure_ascii=False)
    return out
