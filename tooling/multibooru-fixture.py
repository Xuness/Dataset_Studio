"""Three small canonical lakes, distinct identities and shared bytes. Stdlib only."""
import base64
import ctypes
import hashlib
import io
import json
import runpy
import sqlite3
import sys
import tarfile
from pathlib import Path

repo = Path(__file__).resolve().parents[1]
output = Path(sys.argv[1]).resolve()
if not output.is_relative_to(repo / ".local/test-runs"):
    raise ValueError("fixture output must be inside .local/test-runs")
output.mkdir(parents=True, exist_ok=True)
webp = base64.b64decode("UklGRh4AAABXRUJQVlA4TBEAAAAvH8AJAAdQwfJUuv+BiOh/AAA=")
schema_hex = "ffffffff400100001000000000000a000c000600050008000a000000000104000c0000000800080000000400080000000400000002000000d00000000400000048ffffff0000010d1800000020000000040000000200000048000000140000000500000065787472610000007cffffff78ffffff00000105100000002000000004000000000000000c0000006861735f6368696c6472656e00000000acffffffa8ffffff0000010c140000001c000000040000000100000014000000040000007461677300000000d8ffffffd4ffffff00000105100000001c0000000400000000000000040000006974656d000000000400040004000000100014000800060007000c000000100010000000000001021000000024000000040000000000000008000000696d6167655f69640000000008000c000800070008000000000000014000000000000000"
schema_id = hashlib.sha256(bytes.fromhex(schema_hex)).hexdigest()
tags = ["yaoi\u3000translated", "large_breasts\tlong_hair", "censored\n", "common"]
references = {}
for site in ["danbooru", "yandere", "gelbooru"]:
    destination = output / site
    destination.mkdir()
    sys.argv = [str(repo / "tooling/ui-fixture.py"), str(destination)]
    namespace = runpy.run_path(sys.argv[0], run_name="__main__")
    reference = json.loads((destination / "fixture.json").read_text(encoding="utf-8"))
    references[site] = reference
    if site == "danbooru":
        continue
    lake = Path(reference["lake"])
    (lake / "source_manifests").mkdir()
    (lake / "source_manifests/hf-conversion-plan.json").write_text(json.dumps({"site": site, "normalizer": "hf_" + site + "_v1"}), encoding="utf-8")
    sha = hashlib.sha256(webp).hexdigest()
    with tarfile.open(lake / "packs/stored.webp.tar", "w", format=tarfile.PAX_FORMAT) as archive:
        info = tarfile.TarInfo(sha + ".webp")
        info.size = len(webp)
        archive.addfile(info, io.BytesIO(webp))
    with tarfile.open(lake / "packs/stored.webp.tar") as archive:
        offset = archive.getmember(sha + ".webp").offset_data
    first = reference["images"][0]
    old_sha = first["sha"]
    with sqlite3.connect(lake / "indexes/gen-ui/catalog.sqlite") as db:
        db.execute("DELETE FROM objects WHERE sha256=?", [old_sha])
        db.execute("INSERT INTO objects VALUES (?,?,?,?,?)", [sha, "packs/stored.webp.tar", offset, len(webp), "webp"])
    sql = namespace["sql"]
    globals_ = sql.__globals__
    library = globals_["library"]
    database, connection = ctypes.c_void_p(), ctypes.c_void_p()
    if library.duckdb_open(str(lake / "indexes/gen-ui/analysis.duckdb").encode(), ctypes.byref(database)):
        raise RuntimeError("cannot reopen fixture")
    if library.duckdb_connect(database, ctypes.byref(connection)):
        raise RuntimeError("cannot connect fixture")
    globals_["database"], globals_["connection"] = database, connection
    quote = namespace["quote"]
    raw = {"image_id": 10001, "extra": {"tags": tags, "has_children": "false"}}
    try:
        sql("ALTER TABLE assets ADD COLUMN details_json VARCHAR DEFAULT '{}'")
        sql("CREATE TABLE source_schemas(source_schema_id VARCHAR PRIMARY KEY,schema_ipc BLOB)")
        sql(f"INSERT INTO source_schemas VALUES ({quote(schema_id)},from_hex({quote(schema_hex)}))")
        sql(f"UPDATE observations SET source_kind={quote('hf_' + site + '_v1')},fav_count=NULL")
        sql("DROP INDEX observations_post")
        sql("ALTER TABLE observations ALTER created_at TYPE TIMESTAMPTZ USING try_cast(created_at AS TIMESTAMPTZ)")
        sql("CREATE INDEX observations_post ON observations(post_id)")
        if site == "gelbooru":
            sql("UPDATE observations SET created_at='2020-01-01 00:00:00+00'")
        sql(f"UPDATE observations SET tag_string={quote(' '.join(tags))},rating='g',issues_json={quote(json.dumps([{'field':'extra.tags','reason':'literal source whitespace'}]))} WHERE post_id=10001")
        sql(f"UPDATE raw_metadata SET source_metadata_json={quote(json.dumps(raw,ensure_ascii=False))},source_metadata_format='arrow-row-typed-json/v1',source_schema_id={quote(schema_id)} WHERE observation_id='{100001:064x}'")
        sql(f"DELETE FROM objects WHERE sha256={quote(old_sha)}")
        sql(f"INSERT INTO objects VALUES ({quote(sha)},'packs/stored.webp.tar',{offset},{len(webp)},'webp')")
        details = json.dumps({"stored_width": 32, "stored_height": 40, "dimension_method": "webp-container-header-v1"})
        sql(f"UPDATE assets SET sha256={quote(sha)},details_json={quote(details)} WHERE sha256={quote(old_sha)}")
        sql("INSERT INTO observations(observation_id,row_id,post_id,source_kind,rating) VALUES ('" + "f" * 64 + "',999999,999777,'fixture','g')")
        sql("INSERT INTO current_posts VALUES (999777,999999,NULL)")
    finally:
        library.duckdb_disconnect(ctypes.byref(connection))
        library.duckdb_close(ctypes.byref(database))
    first.update(sha=sha, bytes=len(webp), tags=tags, rating="g", current=[{"rating": "g", "tags": tags}])
    reference.update(raw=raw, schema_id=schema_id, schema_hex=schema_hex, missing_post=999777)
(output / "multibooru.json").write_text(json.dumps(references, ensure_ascii=False, indent=2), encoding="utf-8")
print(json.dumps({"multibooru": str(output / "multibooru.json"), "sites": list(references)}))
