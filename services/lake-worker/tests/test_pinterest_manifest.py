import copy

import pytest

from studio_lake.pinterest.media_manifest import media_url, parse


def pin(identity="123", *, story=False):
    value = dict(id=identity, images={"orig": dict(url="https://i.pinimg.com/originals/ab/test.png", width=8, height=6)},
                 image_signature="a" * 32, is_video=False, videos=None, carousel_data=None, story_pin_data=None)
    if story:
        value["story_pin_data"] = dict(page_count=1, static_page_count=1, total_video_duration=0,
            pages=[dict(blocks=[dict(block_type=2, image_signature=value["image_signature"],
                                    image={"images": {"originals": value["images"]["orig"]}})])])
    return value


def test_static_and_single_image_story_preserve_original_evidence():
    for story in (False, True):
        value = pin(story=story)
        result = parse(value)
        assert result["complete"] and result["entries"][0]["field_path"] == "images.orig"
        assert result["entries"][0]["source_url"] == value["images"]["orig"]["url"]
        updated = {**value, "title": "new title", "repin_count": 300}
        assert parse(updated)["content_revision"] == result["content_revision"]
        updated = copy.deepcopy(value)
        updated["images"]["orig"]["url"] += "?revision=2"
        assert parse(updated)["content_revision"] != result["content_revision"]


@pytest.mark.parametrize("field", ["images", "carousel_data", "story_pin_data", "videos", "is_video"])
def test_missing_decision_field_needs_detail_even_when_original_is_present(field):
    value = pin()
    del value[field]
    assert parse(value, observation_kind="list")["reason"] == "media_decision_fields_missing"


@pytest.mark.parametrize("location", ["top", "page", "block"])
def test_hidden_video_never_downloads_its_cover(location):
    value = pin(story=True)
    if location == "top":
        value["videos"] = {"video_list": {"V_720P": {"url": "movie.mp4"}}}
    elif location == "page":
        value["story_pin_data"]["pages"][0]["videos"] = {"movie": {}}
    else:
        value["story_pin_data"]["pages"][0]["blocks"][0]["video"] = {"url": "movie.mp4"}
    assert not parse(value)["complete"] and parse(value)["kind"] == "video"


def test_complex_story_and_carousel_cannot_be_satisfied_by_one_image():
    value = pin(story=True)
    value["story_pin_data"]["pages"][0]["blocks"].append({"block_type": 1, "text": "overlay"})
    assert parse(value)["reason"] == "complex_story_not_supported"
    value = pin(story=True)
    value["story_pin_data"]["pages"][0]["blocks"][0]["image_signature"] = "b" * 32
    assert parse(value)["reason"] == "story_image_identity_unknown"
    value = pin()
    value["carousel_data"] = {"carousel_slots": []}
    assert parse(value)["reason"] == "carousel_not_supported"


def test_url_identity_does_not_merge_distinct_queries_or_untrusted_hosts():
    assert media_url("https://i.pinimg.com:443/originals/A.png?a=1&b=2#x") == "https://i.pinimg.com/originals/A.png?a=1&b=2"
    assert media_url("https://i.pinimg.com/originals/A.png?b=2&a=1") != media_url("https://i.pinimg.com/originals/A.png?a=1&b=2")
    for url in ("http://i.pinimg.com/originals/a.png", "https://i.pinimg.com.evil.test/originals/a.png",
                "https://i.pinimg.com/736x/a.png", "https://u@i.pinimg.com/originals/a.png"):
        assert media_url(url) is None
