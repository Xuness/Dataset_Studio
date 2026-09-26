"""Small deterministic data lake for browser integration checks; Python stdlib only."""
import ctypes
import hashlib
import io
import json
import re
import sqlite3
import struct
import sys
import tarfile
import uuid
import zlib
from pathlib import Path

repo = Path(__file__).resolve().parents[1]
output = Path(sys.argv[1]).resolve()
if not output.is_relative_to(repo / ".local"):
    raise ValueError("UI fixture must stay within the repository .local directory")
lake = output / "lake"
generation = lake / "indexes" / "gen-ui"
generation.mkdir(parents=True, exist_ok=True)
(lake / "packs").mkdir(exist_ok=True)
library_id = str(uuid.uuid4())
(lake / "CURRENT.json").write_text(json.dumps({"library_id": library_id, "index_version": 1, "generation": "gen-ui"}), encoding="utf-8")
(lake / "library.json").write_text(json.dumps({"library_id": library_id, "format_version": 1, "image_format": "uncompressed-pax-tar"}), encoding="utf-8")

def png(number):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    width, height = 128, 160
    pixels = b"".join(b"\0" + bytes(channel for x in range(width) for channel in ((number * 31 + x) % 160 + 32, (number * 17 + y) % 160 + 40, (x + y + number * 5) % 160 + 48)) for y in range(height))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(pixels)) + chunk(b"IEND", b"")

images = []
pack = lake / "packs" / "fixture.tar"
with tarfile.open(pack, "w", format=tarfile.PAX_FORMAT) as archive:
    for number in range(1, 129):
        data = png(number)
        sha = hashlib.sha256(data).hexdigest()
        name = "objects/" + sha + ".png"
        info = tarfile.TarInfo(name)
        info.size = len(data)
        archive.addfile(info, io.BytesIO(data))
        tags = ["common", "red" if number % 2 == 0 else "blue", "solo" if number % 3 else "group", "portrait" if number % 5 else "comic"]
        if number == 12:
            tags = None
        if number == 16:
            tags = []
        images.append({"number": number, "sha": sha, "member": name, "bytes": len(data), "rating": "gsqe"[number % 4], "tags": tags, "post_ids": [] if number == 128 else [10000 + number]})
with tarfile.open(pack) as archive:
    for item in images:
        item["offset"] = archive.getmember(item["member"]).offset_data
catalog = sqlite3.connect(generation / "catalog.sqlite")
catalog.executescript("CREATE TABLE state(key TEXT PRIMARY KEY,value INTEGER); INSERT INTO state VALUES ('seq',1); CREATE TABLE objects(sha256 TEXT PRIMARY KEY,pack_path TEXT,offset INTEGER,length INTEGER,stored_ext TEXT) WITHOUT ROWID;")
catalog.executemany("INSERT INTO objects VALUES (?,'packs/fixture.tar',?,?,'png')", [(x["sha"], x["offset"], x["bytes"]) for x in images])
catalog.commit()
catalog.close()

class Result(ctypes.Structure):
    _fields_ = [("columns", ctypes.c_uint64), ("rows", ctypes.c_uint64), ("changed", ctypes.c_uint64), ("data", ctypes.c_void_p), ("error", ctypes.c_char_p), ("internal", ctypes.c_void_p)]

library = ctypes.CDLL(str(repo / "vendor" / "duckdb" / "duckdb.dll"))
library.duckdb_open.argtypes = [ctypes.c_char_p, ctypes.POINTER(ctypes.c_void_p)]
library.duckdb_connect.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_void_p)]
library.duckdb_query.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.POINTER(Result)]
library.duckdb_destroy_result.argtypes = [ctypes.POINTER(Result)]
library.duckdb_disconnect.argtypes = [ctypes.POINTER(ctypes.c_void_p)]
library.duckdb_close.argtypes = [ctypes.POINTER(ctypes.c_void_p)]
database, connection = ctypes.c_void_p(), ctypes.c_void_p()
if library.duckdb_open(str(generation / "analysis.duckdb").encode(), ctypes.byref(database)):
    raise RuntimeError("Unable to create fixture analysis database")
if library.duckdb_connect(database, ctypes.byref(connection)):
    raise RuntimeError("Unable to connect fixture database")

def sql(text):
    result = Result()
    status = library.duckdb_query(connection, text.encode(), ctypes.byref(result))
    error = result.error.decode() if result.error else ""
    library.duckdb_destroy_result(ctypes.byref(result))
    if status:
        raise RuntimeError(error)

def quote(value):
    if value is None:
        return "NULL"
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int):
        return str(value)
    return "'" + str(value).replace("'", "''") + "'"

try:
    fields_text = (repo / "crates/studio-sources/src/profiles.rs").read_text(encoding="utf-8").split("const METADATA_FIELDS:", 1)[1].split("];", 1)[0]
    fields = re.findall(r'\(\s*"([^"]+)",\s*"([^"]+)",\s*"([^"]+)"\s*\)', fields_text)
    base_columns = "observation_id VARCHAR PRIMARY KEY,row_id BIGINT,post_id BIGINT,source_key VARCHAR,source_kind VARCHAR,observed_at VARCHAR,time_quality VARCHAR,ingested_at VARCHAR,commit_seq BIGINT"
    columns = ",".join(column + " " + {"integer": "BIGINT", "boolean": "BOOLEAN"}.get(kind, "VARCHAR") for _, column, kind in fields)
    sql("SET threads=1; SET memory_limit='128MB'; CREATE TABLE applied(seq BIGINT PRIMARY KEY,batch_id VARCHAR); INSERT INTO applied VALUES (1,'fixture-1'); CREATE TABLE objects(sha256 VARCHAR PRIMARY KEY,pack_path VARCHAR,\"offset\" BIGINT,length BIGINT,stored_ext VARCHAR); CREATE TABLE assets(asset_id VARCHAR PRIMARY KEY,observation_id VARCHAR,post_id BIGINT,sha256 VARCHAR,source_md5 VARCHAR,storage_profile VARCHAR,commit_seq BIGINT); CREATE INDEX assets_sha ON assets(sha256); CREATE TABLE current_posts(post_id BIGINT PRIMARY KEY,row_id BIGINT,asset_id VARCHAR); CREATE TABLE observations(" + base_columns + "," + columns + "); CREATE INDEX observations_post ON observations(post_id); CREATE TABLE raw_metadata(observation_id VARCHAR PRIMARY KEY,source_metadata_json VARCHAR,source_metadata_format VARCHAR,source_schema_id VARCHAR);")
    def observation(row_id, post_id, rating, tags, item):
        values = {"observation_id": f"{row_id:064x}", "row_id": row_id, "post_id": post_id, "source_key": "ui-fixture", "source_kind": "fixture", "observed_at": "2026-09-08 00:00:00+00", "time_quality": "date_only", "ingested_at": "2026-09-08 00:00:00+00", "commit_seq": 1, "rating": rating, "tag_string": None if tags is None else " ".join(tags), "image_width": 128, "image_height": 160, "file_size": item["bytes"], "file_ext": "png", "score": item["number"], "fav_count": 5, "is_deleted": False}
        sql("INSERT INTO observations(" + ",".join(values) + ") VALUES (" + ",".join(quote(v) for v in values.values()) + ")")
        sql("INSERT INTO raw_metadata VALUES (" + quote(values["observation_id"]) + "," + quote(json.dumps({"post_id": post_id, "rating": rating, "tags": tags})) + ",'json','fixture-v1')")
    for item in images:
        sql("INSERT INTO objects VALUES (" + ",".join(map(quote, [item["sha"], "packs/fixture.tar", item["offset"], item["bytes"], "png"])) + ")")
        number = item["number"]
        if number == 128:
            item["current"] = []
            continue
        post, row = 10000 + number, 100000 + number
        asset_id = f"{number:064x}"
        sql("INSERT INTO assets VALUES (" + ",".join(map(quote, [asset_id, f"{row:064x}", post, item["sha"], None, "fixture", 1])) + ")")
        observation(row, post, item["rating"], item["tags"], item)
        sql("INSERT INTO current_posts VALUES (" + ",".join(map(quote, [post, row, asset_id])) + ")")
        item["current"] = [{"rating": item["rating"], "tags": item["tags"]}]
        if number == 7:
            other = 200007
            other_asset = f"{90007:064x}"
            other_tags = ["common", "red", "solo", "portrait"]
            sql("INSERT INTO assets VALUES (" + ",".join(map(quote, [other_asset, f"{other:064x}", other, item["sha"], None, "fixture", 1])) + ")")
            observation(other, other, "g", other_tags, item)
            sql("INSERT INTO current_posts VALUES (" + ",".join(map(quote, [other, other, other_asset])) + ")")
            item["post_ids"].append(other)
            item["current"].append({"rating": "g", "tags": other_tags})
        if number == 3:
            observation(300003, post, "g", ["history_only"], item)
finally:
    library.duckdb_disconnect(ctypes.byref(connection))
    library.duckdb_close(ctypes.byref(database))

reference = {"library_id": library_id, "lake": str(lake), "images": images}
(output / "fixture.json").write_text(json.dumps(reference, ensure_ascii=False, indent=2), encoding="utf-8")
print(json.dumps({"fixture": str(output / "fixture.json"), "objects": len(images)}))
