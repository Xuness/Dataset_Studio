"""Advance only a bounded private ranking fixture, with real append-shaped data."""
import ctypes as c
import hashlib
import io
import json
import sqlite3
import sys
import tarfile
from pathlib import Path

root = Path(__file__).resolve().parents[1]
folder = Path(sys.argv[1]).resolve()
if not folder.is_relative_to(root / '.local/test-runs'):
    raise ValueError('Updates must remain in a test run')
fixture = json.loads((folder / 'fixture.json').read_text(encoding='utf-8'))
lake = Path(fixture['lake']).resolve()
if not lake.is_relative_to(folder):
    raise ValueError('Fixture lake escaped its directory')
gen = lake / 'indexes' / 'gen-ranking'
catalog = sqlite3.connect(gen / 'catalog.sqlite')
seq = catalog.execute("SELECT value FROM state WHERE key='seq'").fetchone()[0] + 1
batch = f'daily-fixture-{seq}'

class Result(c.Structure):
    _fields_ = [('columns', c.c_uint64), ('rows', c.c_uint64), ('changed', c.c_uint64), ('data', c.c_void_p), ('error', c.c_char_p), ('internal', c.c_void_p)]

lib = c.CDLL(str(root / 'vendor/duckdb/duckdb.dll'))
for name, args in [('open', [c.c_char_p, c.POINTER(c.c_void_p)]), ('connect', [c.c_void_p, c.POINTER(c.c_void_p)]), ('query', [c.c_void_p, c.c_char_p, c.POINTER(Result)]), ('destroy_result', [c.POINTER(Result)]), ('disconnect', [c.POINTER(c.c_void_p)]), ('close', [c.POINTER(c.c_void_p)])]:
    getattr(lib, 'duckdb_' + name).argtypes = args
database, connection = c.c_void_p(), c.c_void_p()
assert not lib.duckdb_open(str(gen / 'analysis.duckdb').encode(), c.byref(database))
assert not lib.duckdb_connect(database, c.byref(connection))
def sql(text):
    result = Result()
    code = lib.duckdb_query(connection, text.encode(), c.byref(result))
    error = result.error.decode() if result.error else ''
    lib.duckdb_destroy_result(c.byref(result))
    if code:
        raise RuntimeError(error)

try:
    sql('BEGIN')
    if len(sys.argv) < 3 or sys.argv[2] != 'noop':
        row = 900000 + seq * 10
        obs = f'{row:064x}'
        # Update a current observation while leaving its stored image identity.
        sql(f"INSERT INTO observations SELECT * REPLACE({row} AS row_id,'{obs}' AS observation_id,{seq} AS commit_seq,'{batch}' AS batch_id,CASE WHEN rating='g' THEN 'e' ELSE 'g' END AS rating,'daily_changed' AS tag_string) FROM observations WHERE row_id=(SELECT MIN(row_id) FROM current_posts)")
        sql(f'UPDATE current_posts SET row_id={row} WHERE row_id=(SELECT MIN(row_id) FROM current_posts)')
        with tarfile.open(lake / 'packs/fixture.tar') as archive:
            member = next(m for m in archive if m.isfile())
            payload = archive.extractfile(member).read() + f'daily-{seq}'.encode()
        sha = hashlib.sha256(payload).hexdigest()
        pack = f'segments/{batch}/images.tar'
        (lake / pack).parent.mkdir(parents=True, exist_ok=True)
        with tarfile.open(lake / pack, 'w') as archive:
            item = tarfile.TarInfo(f'objects/{sha}.png')
            item.size = len(payload)
            archive.addfile(item, io.BytesIO(payload))
        aid, oid, post = f'{row+1:064x}', f'{row+2:064x}', row + 2
        sql(f"INSERT INTO observations SELECT * REPLACE({post} AS row_id,'{oid}' AS observation_id,{post} AS post_id,{seq} AS commit_seq,'{batch}' AS batch_id,'g' AS rating,'daily_new' AS tag_string) FROM observations WHERE row_id={row}")
        sql(f"INSERT INTO assets SELECT * REPLACE('{aid}' AS asset_id,'{oid}' AS observation_id,{post} AS post_id,'{sha}' AS sha256,{seq} AS commit_seq,'{batch}' AS batch_id) FROM assets LIMIT 1")
        sql(f"INSERT INTO current_posts VALUES({post},{post},'{aid}')")
        sql(f"INSERT INTO objects VALUES('{sha}','{pack}',512,{len(payload)},'png')")
        catalog.execute("INSERT INTO objects VALUES(?,?,?,?,?)", (sha,pack,512,len(payload),'png'))
    sql(f"INSERT INTO applied VALUES({seq},'{batch}'); COMMIT; CHECKPOINT")
    catalog.execute("UPDATE state SET value=? WHERE key='seq'", (seq,))
    catalog.commit()
    print(json.dumps({'sequence':seq,'batch':batch}))
finally:
    catalog.close()
    lib.duckdb_disconnect(c.byref(connection))
    lib.duckdb_close(c.byref(database))
