"""Hold an uncommitted control-row write while Studio reads; always roll it back."""
import json
import sys
import time
import uuid
from pathlib import Path
import apsw

root=Path(sys.argv[1]).resolve()
pointer=json.loads((root/"ONLINE.json").read_text(encoding="utf-8"))
path=(root/pointer["file"]).resolve()
if not path.is_relative_to(root):
    raise ValueError("database escapes online root")
if tuple(map(int,apsw.sqlitelibversion().split("."))) < (3,51,3):
    raise ValueError("SQLite needs the WAL-reset fix")
db=apsw.Connection(str(path),flags=apsw.SQLITE_OPEN_READWRITE)
db.set_busy_timeout(3000)
try:
    db.execute("BEGIN IMMEDIATE")
    seq=next(db.execute("SELECT CAST(value AS INTEGER) FROM online_state WHERE key='served_seq'"))[0]
    db.execute("INSERT INTO leases VALUES(?,?,?,?,?)",("probe/"+uuid.uuid4().hex,seq,int(time.time()*1000)+10000,"diagnostic","rollback-only"))
    print("ready",flush=True)
    sys.stdin.readline()
finally:
    if db.in_transaction:
        db.execute("ROLLBACK")
    db.close()
