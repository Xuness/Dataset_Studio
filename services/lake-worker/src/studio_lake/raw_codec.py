"""Lossless online-v2 raw records, shared by publication, rebuild and verification."""

import hashlib
import json
import zlib

from .api_metadata import API_KINDS
from .util import IntegrityError


def source_record(record, source_kind):
    if source_kind in API_KINDS:
        return record["post_json"], "api-json-original-object/v1"
    return (
        json.dumps(record, ensure_ascii=False, allow_nan=False, separators=(",", ":")),
        "arrow-row-typed-json/v1",
    )


def encode(raw):
    body = b"" if raw is None else raw.encode("utf-8")
    return -1 if raw is None else len(body), hashlib.sha256(body).hexdigest(), zlib.compress(body, 1)


def decode(compressed, length, sha256, *, maximum_bytes):
    """Check the declared bound before allocating, and verify the original bytes."""
    if length < -1 or maximum_bytes < 0 or length > maximum_bytes:
        raise IntegrityError("原始元数据超过解压预算或长度无效")
    expected = max(length, 0)
    try:
        stream = zlib.decompressobj()
        body = stream.decompress(compressed, expected + 1)
        if len(body) != expected or not stream.eof or stream.unused_data or stream.unconsumed_tail:
            raise IntegrityError("原始元数据压缩内容或长度校验失败")
        if hashlib.sha256(body).hexdigest() != sha256:
            raise IntegrityError("原始元数据摘要校验失败")
        return None if length == -1 else body.decode("utf-8")
    except (zlib.error, UnicodeError) as error:
        raise IntegrityError("原始元数据压缩内容无效") from error
