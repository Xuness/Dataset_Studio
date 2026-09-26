"""Synthetic three-lake serving fixture. Requires DuckDB and APSW (SQLite >= 3.51.3)."""
import datetime
import hashlib
import json
import runpy
import sys
import zlib
from pathlib import Path

import apsw
import duckdb

repo = Path(__file__).resolve().parents[1]
output = Path(sys.argv[1]).resolve()
action = sys.argv[2] if len(sys.argv) > 2 else "create"
if not output.is_relative_to(repo / ".local/test-runs"):
    raise ValueError("fixture must be inside .local/test-runs")
if apsw.sqlitelibversion().split(".") < ["3", "51", "3"]:
    raise ValueError("APSW SQLite needs the WAL-reset fix")


def scalar(v):
    if isinstance(v, (datetime.datetime, datetime.date)):
        return v.isoformat()
    return int(v) if isinstance(v, bool) else v


if action == "create":
    sys.argv = [str(repo / "tooling/multibooru-fixture.py"), str(output)]
    runpy.run_path(sys.argv[0], run_name="__main__")
    refs = json.loads((output / "multibooru.json").read_text(encoding="utf-8"))
    for site, ref in refs.items():
        root = Path(ref["lake"])
        source = duckdb.connect(str(root / "indexes/gen-ui/analysis.duckdb"), read_only=True)
        db = apsw.Connection(str(root / "serving.sqlite"))
        db.execute("PRAGMA journal_mode=WAL")
        db.execute((repo / "crates/studio-sources/src/online/schema.sql").read_text())
        generation = "online-fixture-" + site
        with db:
            for table in ["objects", "observations", "assets"]:
                columns = [r[1] for r in db.execute("PRAGMA table_info(" + table + ")")]
                native = [r[0] for r in source.execute("DESCRIBE " + table).fetchall()]
                names = [n for n in columns if n in native and n not in {"commit_seq", "batch_id"}]
                values = source.execute("SELECT " + ",".join('"' + n + '"' for n in names) + " FROM " + table).fetchall()
                defaults = {"first_seq": 1} if table == "objects" else {"commit_seq": 1, "batch_id": "fixture-1"}
                defaults = {n:v for n,v in defaults.items() if n not in names}
                sql = "INSERT INTO " + table + "(" + ",".join('"' + n + '"' for n in names+list(defaults)) + ") VALUES(" + ",".join("?" for _ in names+list(defaults)) + ")"
                db.executemany(sql, [tuple(scalar(v) for v in row)+tuple(defaults.values()) for row in values])
            for row in source.execute("SELECT * FROM raw_metadata").fetchall():
                oid, raw, fmt, schema = row
                body = (raw or "").encode()
                db.execute("INSERT INTO raw_metadata(observation_id,source_metadata_format,source_schema_id,raw_bytes,raw_sha256,raw_zlib) VALUES(?,?,?,?,?,?)",(oid,fmt,schema,-1 if raw is None else len(body),hashlib.sha256(body).hexdigest(),zlib.compress(body)))
            if site != "danbooru":
                db.executemany("INSERT INTO source_schemas VALUES(?,?)",source.execute("SELECT * FROM source_schemas").fetchall())
            db.executemany("INSERT INTO post_versions VALUES(?,1,NULL,?,?)",source.execute("SELECT * FROM current_posts").fetchall())
            db.execute("INSERT INTO object_versions SELECT o.sha256,1,NULL,min(a.post_id) FROM objects o LEFT JOIN assets a USING(sha256) GROUP BY o.sha256")
            for oid, tags in list(db.execute("SELECT row_id,tag_string FROM observations")):
                tokens=[]
                for tag in sorted(set((tags or "").split(" ")) - {""}):
                    db.execute("INSERT OR IGNORE INTO tags(tag) VALUES(?)",(tag,))
                    tid=next(db.execute("SELECT tag_id FROM tags WHERE tag=?",(tag,)))[0]
                    tokens.append(f"t{tid:x}")
                db.execute("INSERT INTO tag_index(rowid,tokens) VALUES(?,?)",(oid," ".join(tokens)))
            counts={name:next(db.execute("SELECT count(*) FROM "+name))[0] for name in ["objects","observations","assets"]}
            state={"library_id":ref["library_id"],"generation":generation,"site":site,"base_seq":1,"min_seq":1,"served_seq":1,"archive_seq":1,"analysis_seq":1,"object_count":counts["objects"],"observation_count":counts["observations"],"asset_count":counts["assets"]}
            db.executemany("INSERT INTO online_state VALUES(?,?)",[(k,str(v)) for k,v in state.items()])
            db.execute("INSERT INTO publications VALUES(1,'fixture-1','now',?,?,?)",tuple(counts.values()))
        db.execute("ANALYZE")
        db.close()
        source.close()
        (root / "indexes/gen-ui/analysis.duckdb").rename(root / "indexes/gen-ui/analysis.offline")
        (root / "ONLINE.json").write_text(json.dumps({"schema_version":2,"site":site,"generation":generation,"library_id":ref["library_id"],"file":"serving.sqlite"}),encoding="utf-8")
elif action in ["stage", "publish"]:
    refs=json.loads((output / "multibooru.json").read_text(encoding="utf-8"))
    db=apsw.Connection(str(Path(refs["danbooru"]["lake"])/"serving.sqlite"))
    db.set_busy_timeout(3000)
    with db:
        if action == "stage":
            old=list(db.execute("SELECT row_id,asset_id FROM post_versions WHERE post_id=10001 AND valid_until IS NULL"))[0]
            cols=[r[1] for r in db.execute("PRAGMA table_info(observations)") if r[1] != "row_id"]
            db.execute("INSERT INTO observations("+",".join('"'+c+'"' for c in cols)+") SELECT "+",".join("9999" if c=="score" else "2" if c=="commit_seq" else "'"+f'{900001:064x}'+"'" if c=="observation_id" else '"'+c+'"' for c in cols)+" FROM observations WHERE row_id=?",(old[0],))
            oid=db.last_insert_rowid()
            db.execute("UPDATE post_versions SET valid_until=2 WHERE post_id=10001 AND valid_until IS NULL")
            db.execute("INSERT INTO post_versions VALUES(10001,2,NULL,?,?)",(oid,old[1]))
            db.execute("UPDATE online_state SET value='2' WHERE key='archive_seq'")
        else:
            db.execute("INSERT INTO publications SELECT 2,'update','now',objects_count,observations_count+1,assets_count FROM publications WHERE seq=1")
            db.execute("UPDATE online_state SET value='2' WHERE key='served_seq'")
    db.close()
else:
    raise ValueError(action)
