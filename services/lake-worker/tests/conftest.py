import hashlib
import io
import json
import sqlite3
import tarfile
from decimal import Decimal

import pyarrow as pa
import pyarrow.parquet as pq
import pytest
from PIL import Image

from studio_lake.config import Config
from studio_lake.library import Library


def png(color):
    f = io.BytesIO()
    Image.new("RGB", (12, 9), color).save(f, format="PNG")
    return f.getvalue()


@pytest.fixture
def lib(tmp_path):
    return Library.initialize(
        Config(
            tmp_path / "主库",
            tmp_path / "SSD",
            pack_target_bytes=140,
            shard_target_bytes=100,
            row_group_rows=2,
            memory_limit="1GB",
            threads=2,
        )
    )


@pytest.fixture
def parquet_source(tmp_path):
    root = tmp_path / "Full-Danbooru-Complement-20260518"
    p = root / "shards" / "rating_g" / "part-000000.parquet"
    p.parent.mkdir(parents=True)
    images = [png("red"), png("blue"), png("red"), png("green")]
    table = pa.table(
        {
            "id": pa.array([11, 12, 13, 14], type=pa.int64()),
            "rating": ["g", "s", "g", "e"],
            "score": [40, 30, 20, 10],
            "fav_count": [4, 3, 2, 1],
            "down_score": [-2, None, 0, -1],
            "tag_string": ["red hair", "blue hair", "red hat", "green"],
            "created_at": ["2020-01-01T00:00:00+00:00"] * 4,
            "image_bytes": images,
            "raw_stored_ext": ["png"] * 4,
            "raw_sha256": [hashlib.sha256(x).hexdigest() for x in images],
            "md5": [hashlib.md5(x).hexdigest() for x in images],
            "unknown_int": pa.array([2**60 + 3, 2**60 + 5, None, -(2**60)], type=pa.int64()),
            "unknown_decimal": pa.array(
                [Decimal("1.000000000001"), None, Decimal("-1.002"), Decimal("0")], type=pa.decimal128(30, 12)
            ),
            "unknown_nested": pa.array(
                [[{"key": "值", "v": None}], [], None, [{"key": "", "v": False}]],
                type=pa.list_(pa.struct([("key", pa.string()), ("v", pa.bool_())])),
            ),
            "unknown_binary": [b"\x00\xff", b"", None, b"test"],
            "unknown_nan": [float("nan"), None, 0.0, float("inf")],
            "unknown_timestamp_ns": pa.array(
                [1, 1001, None, 1735783234567890123], type=pa.timestamp("ns", tz="UTC")
            ),
            "source_file": [r"E:\private\legacy.parquet"] * 4,
        }
    ).replace_schema_metadata({b"original_custom": b"\x00\xffschema", b"meaning": "原始".encode()})
    pq.write_table(table, p, row_group_size=2, version="2.6", compression="zstd")
    return root, p, pq.ParquetFile(p).read(), images


@pytest.fixture
def v2_source(tmp_path):
    root = tmp_path / "DeepGHS_V2"
    pack = root / "images" / "linked_packs" / "0000.tar"
    pack.parent.mkdir(parents=True)
    images = [png("red"), png("blue"), png("green")]
    with tarfile.open(pack, "w", format=tarfile.PAX_FORMAT) as tar:
        for i, data in enumerate(images):
            info = tarfile.TarInfo(f"{21 + i}.png")
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))
    with tarfile.open(pack) as tar:
        offsets = [m.offset_data for m in tar]
    meta = root / "metadata" / "posts" / "rating_g" / "0000.parquet"
    meta.parent.mkdir(parents=True)
    pq.write_table(
        pa.table(
            {
                "id": [21, 22],
                "rating": ["g", "g"],
                "tag_string": ["a", "b"],
                "score": [2, 1],
                "md5": [hashlib.md5(x).hexdigest() for x in images[:2]],
                "kept_unknown": ["中文", None],
            }
        ),
        meta,
        row_group_size=1,
    )
    db = sqlite3.connect(root / "images" / "image_index.sqlite")
    db.executescript("""CREATE TABLE images(item_key TEXT PRIMARY KEY,id INTEGER,pack_root_id TEXT,
        pack_file TEXT,offset INTEGER,length INTEGER,ext TEXT,source_sha256 TEXT,
        metadata_relative_path TEXT,metadata_row_index INTEGER,unknown_index_field TEXT);
        CREATE INDEX idx_images_id ON images(id);""")
    db.executemany(
        "INSERT INTO images VALUES (?,?,?,?,?,?,?,?,?,?,?)",
        [
            (
                str(21 + i),
                21 + i,
                "linked",
                "0000.tar",
                offsets[i],
                len(data),
                "png",
                hashlib.sha256(data).hexdigest(),
                "metadata/posts/rating_g/0000.parquet" if i < 2 else None,
                i if i < 2 else None,
                "额外索引信息",
            )
            for i, data in enumerate(images)
        ],
    )
    db.commit()
    db.close()
    m = {
        "schema_version": "complement_v2.1",
        "library_id": "fixture",
        "source": {},
        "layout": {
            "image_index": "images/image_index.sqlite",
            "image_packs": "images/linked_packs",
            "metadata_posts": "metadata/posts",
        },
        "image_storage": {"pack_roots": {"linked": {"root": "images/linked_packs"}}},
    }
    (root / "library_manifest.json").write_text(json.dumps(m), encoding="utf-8")
    return root, images
