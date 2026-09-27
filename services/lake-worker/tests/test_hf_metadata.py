import json

import pytest

from studio_lake.hf_metadata import normalize_hf, tags_text
from studio_lake.util import IntegrityError


@pytest.mark.parametrize(
    "value, expected", [(None, None), ([], ""), (["solo", "测试", "solo"], "solo 测试 solo")]
)
def test_regular_tags_keep_original_projection(value, expected):
    issues = []
    assert tags_text(value, "extra.tags", issues) == expected
    assert issues == []


@pytest.mark.parametrize("whitespace", ["\t", "\n", "\r", "\v", "\f", "\u00a0", "\u2003", "\u3000"])
def test_non_delimiter_whitespace_keeps_tag_identity(whitespace):
    tags = ["solo", f"v{whitespace}sign", f"tail{whitespace}", "solo"]
    issues = []
    text = tags_text(tags, "extra.tags", issues)
    assert text.split(" ") == tags
    assert issues == [
        {
            "field": "extra.tags",
            "reason": "whitespace other than U+0020 retained literally inside source tags",
            "tag_indexes": [1, 2],
        }
    ]


@pytest.mark.parametrize("value", ["solo", [None], [42], [""], ["red hair"], [" leading"], ["trailing "]])
def test_ambiguous_tags_still_fail_with_source_context(value):
    record = {"image_id": 1025, "extra": {"id": 1025, "tags": value}}
    with pytest.raises(IntegrityError, match=r"gelbooru post 1025, source row 123: HF extra.tags"):
        normalize_hf(record, "source", 123, "hf_gelbooru_v1", 0)


def test_tag_issues_preserve_other_normalization_issues():
    record = {
        "image_id": 1025,
        "image_created_at": "2026-01-01T00:00:00",
        "extra": {"id": 1025, "rating": "unknown", "tags": ["v\u3000sign"], "tags_character": ["name\n"]},
    }
    output = normalize_hf(record, "source", 123, "hf_gelbooru_v1", 0)
    assert output["tag_string"] == "v\u3000sign"
    assert output["tag_string_character"] == "name\n"
    assert {issue["field"] for issue in json.loads(output["issues_json"])} == {
        "created_at",
        "extra.tags",
        "extra.tags_character",
        "rating",
    }
