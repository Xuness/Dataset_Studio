"""Append a producer-shaped commit to a private UI fixture; never real archives."""
import ctypes
import hashlib
import io
import json
import shutil
import sqlite3
import sys
import tarfile
from pathlib import Path

repo = Path(__file__).resolve().parents[1]
output = Path(sys.argv[1]).resolve()
if not output.is_relative_to(repo / ".local"):
    raise ValueError("Fixture updates must stay within .local")
reference_path = output / "fixture.json"
reference = json.loads(reference_path.read_text(encoding="utf-8"))
lake = Path(reference["lake"]).resolve()
if not lake.is_relative_to(output):
    raise ValueError("Fixture lake escaped its run directory")
pointer = json.loads((lake / "CURRENT.json").read_text(encoding="utf-8"))
generation = lake / "indexes" / pointer["generation"]
mode = sys.argv[2] if len(sys.argv) > 2 else "advance"
if mode == "rebuild":
    destination = lake / "indexes" / (pointer["generation"] + "-rebuilt")
    shutil.copytree(generation, destination)
    pointer["generation"] = destination.name
    (lake / "CURRENT.json").write_text(json.dumps(pointer), encoding="utf-8")
    print(json.dumps({"generation": destination.name}))
    sys.exit(0)

class Result(ctypes.Structure):
    _fields_ = [("columns", ctypes.c_uint64), ("rows", ctypes.c_uint64),
                ("changed", ctypes.c_uint64), ("data", ctypes.c_void_p),
                ("error", ctypes.c_char_p), ("internal", ctypes.c_void_p)]

lib = ctypes.CDLL(str(repo / "vendor/duckdb/duckdb.dll"))
for name, args in {
    "duckdb_open": [ctypes.c_char_p, ctypes.POINTER(ctypes.c_void_p)],
    "duckdb_connect": [ctypes.c_void_p, ctypes.POINTER(ctypes.c_void_p)],
    "duckdb_query": [ctypes.c_void_p, ctypes.c_char_p, ctypes.POINTER(Result)],
    "duckdb_destroy_result": [ctypes.POINTER(Result)],
    "duckdb_disconnect": [ctypes.POINTER(ctypes.c_void_p)],
    "duckdb_close": [ctypes.POINTER(ctypes.c_void_p)],
}.items():
    getattr(lib, name).argtypes = args
db, connection = ctypes.c_void_p(), ctypes.c_void_p()
if lib.duckdb_open(str(generation / "analysis.duckdb").encode(), ctypes.byref(db)) or lib.duckdb_connect(db, ctypes.byref(connection)):
    raise RuntimeError("Fixture analysis could not be opened")

def sql(text):
    result = Result()
    status = lib.duckdb_query(connection, text.encode(), ctypes.byref(result))
    error = result.error.decode() if result.error else ""
    lib.duckdb_destroy_result(ctypes.byref(result))
    if status:
        raise RuntimeError(error)

catalog = sqlite3.connect(generation / "catalog.sqlite")
sequence = catalog.execute("SELECT value FROM state WHERE key='seq'").fetchone()[0] + 1
if mode == "gap":
    sequence += 1
batch = "fixture-" + str(sequence)
try:
    # Old post 10003 changes classification. Post 10004 gets a new physical image.
    # A lower new post links an already stored image, changing its display ID.
    sql("BEGIN")
    old = reference["images"][2]
    rating = "g" if sequence % 2 == 0 else "e"
    row = 500000 + sequence * 10
    observation_id = f"{row:064x}"
    sql(f"INSERT INTO observations SELECT * REPLACE('{observation_id}' AS observation_id,{row} AS row_id,{sequence} AS commit_seq,'{rating}' AS rating) FROM observations WHERE row_id=100003")
    sql(f"UPDATE current_posts SET row_id={row} WHERE post_id=10003")
    old["current"][0]["rating"] = rating

    original = reference["images"][3]
    with tarfile.open(lake / "packs/fixture.tar") as archive:
        data = archive.extractfile(original["member"]).read() + bytes([sequence % 256])
    sha = hashlib.sha256(data).hexdigest()
    pack_path = "segments/" + batch + "/images.tar"
    pack = lake / pack_path
    pack.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(pack, "w", format=tarfile.PAX_FORMAT) as archive:
        info = tarfile.TarInfo("objects/" + sha + ".png")
        info.size = len(data)
        archive.addfile(info, io.BytesIO(data))
    asset_id = f"{row + 1:064x}"
    new_observation = f"{row + 2:064x}"
    sql(f"INSERT INTO objects VALUES ('{sha}','{pack_path}',512,{len(data)},'png')")
    sql(f"INSERT INTO observations SELECT * REPLACE('{new_observation}' AS observation_id,{row + 2} AS row_id,{sequence} AS commit_seq,'e' AS rating) FROM observations WHERE row_id=100004")
    sql(f"INSERT INTO assets VALUES ('{asset_id}','{new_observation}',10004,'{sha}',NULL,'fixture',{sequence})")
    sql(f"UPDATE current_posts SET row_id={row + 2},asset_id='{asset_id}' WHERE post_id=10004")
    for item in reference["images"]:
        if 10004 in item["post_ids"]:
            item["current"] = []
    reference["images"].append({"number": row, "sha": sha, "post_ids": [10004], "current": [{"rating": "e", "tags": original["tags"]}]})

    shared = reference["images"][6]
    post = 8000 - sequence
    shared_row = row + 3
    shared_observation = f"{shared_row:064x}"
    shared_asset = f"{shared_row + 1:064x}"
    sql(f"INSERT INTO observations SELECT * REPLACE('{shared_observation}' AS observation_id,{shared_row} AS row_id,{post} AS post_id,{sequence} AS commit_seq,'g' AS rating) FROM observations WHERE row_id=100007")
    sql(f"INSERT INTO assets VALUES ('{shared_asset}','{shared_observation}',{post},'{shared['sha']}',NULL,'fixture',{sequence})")
    sql(f"INSERT INTO current_posts VALUES ({post},{shared_row},'{shared_asset}')")
    shared["post_ids"].append(post)
    shared["current"].append({"rating": "g", "tags": shared["tags"]})
    sql(f"INSERT INTO applied VALUES ({sequence},'{batch}'); COMMIT")
    catalog.execute("INSERT INTO objects VALUES (?,?,?,?,?)", (sha, pack_path, 512, len(data), "png"))
    catalog.execute("UPDATE state SET value=? WHERE key='seq'", (sequence,))
    catalog.commit()
finally:
    catalog.close()
    lib.duckdb_disconnect(ctypes.byref(connection))
    lib.duckdb_close(ctypes.byref(db))
reference["sequence"] = sequence
reference_path.write_text(json.dumps(reference, ensure_ascii=False, indent=2), encoding="utf-8")
print(json.dumps({"sequence": sequence, "objects": len(reference["images"]), "mode": mode}))
