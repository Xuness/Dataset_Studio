"""Conservative, field-presence-aware recognition of Pinterest original static media."""

import re
from urllib.parse import urlsplit, urlunsplit

from ..util import stable_id

DECISION_FIELDS = frozenset(("images", "carousel_data", "story_pin_data", "videos", "is_video"))


def media_url(value):
    """Only normalize syntax with no representation meaning; retain the exact query and path."""
    if not isinstance(value, str) or len(value) > 8192 or any(ord(c) < 33 for c in value):
        return None
    try:
        part = urlsplit(value)
        if (part.scheme.lower() != "https" or part.hostname != "i.pinimg.com"
                or part.username or part.password or part.port not in (None, 443)
                or not part.path.startswith("/originals/") or "\\" in value):
            return None
    except ValueError:
        return None
    return urlunsplit(("https", "i.pinimg.com", part.path, part.query, ""))


def _video(value):
    if not isinstance(value, dict):
        return False
    return bool(value.get("video") or value.get("videos") or value.get("is_video")
                or value.get("block_type") == 3 or value.get("type") == "story_pin_video_block")


def parse(pin, *, observation_kind="detail"):
    def gap(reason, kind="unknown"):
        return dict(state="needs_review", reason=reason, kind=kind, entries=[], complete=False,
                    content_revision=None)

    if not isinstance(pin, dict):
        return gap("pin_shape_unknown")
    if DECISION_FIELDS - pin.keys():
        return gap("media_decision_fields_missing")
    if type(pin["is_video"]) is not bool:
        return gap("media_decision_fields_invalid")
    if pin["videos"] is not None and not isinstance(pin["videos"], dict):
        return gap("media_decision_fields_invalid")
    story = pin["story_pin_data"]
    if _video(pin):
        return gap("video_not_supported", "video")
    if pin["carousel_data"] is not None:
        return gap("carousel_not_supported", "carousel")
    shape = "static_image"
    if story is not None:
        if not isinstance(story, dict) or not isinstance(story.get("pages"), list):
            return gap("story_shape_unknown", "story")
        pages = story["pages"]
        if (_video(story) or story.get("total_video_duration", 0)
                or any(_video(page) or any(_video(b) for b in page.get("blocks", []) if isinstance(b, dict))
                       for page in pages if isinstance(page, dict) and isinstance(page.get("blocks"), list))):
            return gap("story_video_not_supported", "video")
        if (story.get("page_count") != 1 or story.get("static_page_count", 1) != 1
                or len(pages) != 1 or not isinstance(pages[0], dict)
                or not isinstance(pages[0].get("blocks"), list) or len(pages[0]["blocks"]) != 1):
            return gap("complex_story_not_supported", "story")
        block = pages[0]["blocks"][0]
        signature = pin.get("image_signature")
        if (not isinstance(block, dict) or block.get("block_type") != 2
                or not isinstance(block.get("image"), dict) or not isinstance(signature, str)
                or not re.fullmatch("[a-fA-F0-9]{32}", signature)
                or block.get("image_signature") != signature):
            return gap("story_image_identity_unknown", "story")
        shape = "single_image_story"
    images = pin.get("images")
    original = images.get("orig") if isinstance(images, dict) else None
    if not isinstance(original, dict):
        return gap("original_field_missing", shape)
    url = media_url(original.get("url"))
    width, height = original.get("width"), original.get("height")
    if not url or any(type(v) is not int or not 0 < v <= 1_000_000 for v in (width, height)):
        return gap("original_field_invalid", shape)
    entry = dict(ordinal=0, slot_key="image:0", kind="image", role="original",
                 source_url=original["url"], normalized_url=url, width=width, height=height,
                 field_path="images.orig", image_signature=pin.get("image_signature") if isinstance(pin.get("image_signature"), str) else None)
    return dict(state="ready", reason=None, kind=shape, entries=[entry], complete=True,
                content_revision=stable_id("pinterest-content-v1", shape, [entry]),
                observation_kind=observation_kind)
