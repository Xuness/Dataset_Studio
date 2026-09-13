"""Deterministic rich Danbooru lake for complete ranking tests; stdlib and pinned C API."""
import ctypes as c
import datetime as dt
import hashlib
import io
import json
import sqlite3
import struct
import sys
import tarfile
import uuid
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = Path(sys.argv[1]).resolve()
N = int(sys.argv[2]) if len(sys.argv) > 2 else 2048
SPARSE_BROWSE = '--sparse-browse' in sys.argv[3:]
V2_TAGS = '--v2-tags' in sys.argv[3:]
DUPLICATE_HEAT = '--duplicate-heat' in sys.argv[3:]
if not OUT.is_relative_to(ROOT / '.local') or not 32 <= N <= 100000:
    raise ValueError('Fixture must use a bounded repository-local directory')
LAKE = OUT / 'lake'
GEN = LAKE / 'indexes' / 'gen-ranking'
GEN.mkdir(parents=True, exist_ok=True)
(LAKE / 'packs').mkdir(exist_ok=True)
LIBRARY_ID = str(uuid.uuid4())
(LAKE / 'CURRENT.json').write_text(json.dumps({'library_id': LIBRARY_ID, 'index_version': 1, 'generation': 'gen-ranking'}), encoding='utf-8')
(LAKE / 'library.json').write_text(json.dumps({'library_id': LIBRARY_ID, 'format_version': 1, 'image_format': 'uncompressed-pax-tar'}), encoding='utf-8')

def chunk(kind, data):
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))


pixel_cache = {}
def png(number, width, height):
    key = (number % 16, width, height)
    if key not in pixel_cache:
        color = bytes((70 + number % 16 * 7, 100 + number % 16 * 5, 160 - number % 16 * 4))
        raw = (b'\0' + color * width) * height
        pixel_cache[key] = zlib.compress(raw, 6)
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0))
            + chunk(b'IDAT', pixel_cache[key]) + chunk(b'tEXt', b'fixture\0' + str(number).encode()) + chunk(b'IEND', b''))


images = []
with tarfile.open(LAKE / 'packs' / 'fixture.tar', 'w', format=tarfile.PAX_FORMAT) as archive:
    for number in range(N + 1):
        width, height = (256, 512) if number % 12 == 0 else (1024, 768)
        payload = png(number, width, height)
        sha = hashlib.sha256(payload).hexdigest()
        member = 'objects/' + sha + '.png'
        info = tarfile.TarInfo(member)
        info.size = len(payload)
        archive.addfile(info, io.BytesIO(payload))
        images.append({'number': number, 'sha': sha, 'md5': hashlib.md5(payload).hexdigest(), 'member': member,
                       'width': width, 'height': height, 'bytes': len(payload)})
with tarfile.open(LAKE / 'packs' / 'fixture.tar') as archive:
    offsets = {member.name: member.offset_data for member in archive}
    for image in images:
        image['offset'] = offsets[image['member']]
catalog = sqlite3.connect(GEN / 'catalog.sqlite')
catalog.executescript("CREATE TABLE state(key TEXT PRIMARY KEY,value INTEGER); INSERT INTO state VALUES('seq',1); CREATE TABLE objects(sha256 TEXT PRIMARY KEY,pack_path TEXT,offset INTEGER,length INTEGER,stored_ext TEXT) WITHOUT ROWID;")
catalog.executemany("INSERT INTO objects VALUES(?,'packs/fixture.tar',?,?,'png')", [(i['sha'], i['offset'], i['bytes']) for i in images])
catalog.commit()
catalog.close()

class Result(c.Structure):
    _fields_ = [('columns', c.c_uint64), ('rows', c.c_uint64), ('changed', c.c_uint64), ('data', c.c_void_p), ('error', c.c_char_p), ('internal', c.c_void_p)]


lib = c.CDLL(str(ROOT / 'vendor/duckdb/duckdb.dll'))
for name, args in [('open', [c.c_char_p, c.POINTER(c.c_void_p)]), ('connect', [c.c_void_p, c.POINTER(c.c_void_p)]),
                   ('query', [c.c_void_p, c.c_char_p, c.POINTER(Result)]), ('destroy_result', [c.POINTER(Result)]),
                   ('disconnect', [c.POINTER(c.c_void_p)]), ('close', [c.POINTER(c.c_void_p)])]:
    getattr(lib, 'duckdb_' + name).argtypes = args
database, connection = c.c_void_p(), c.c_void_p()
assert not lib.duckdb_open(str(GEN / 'analysis.duckdb').encode(), c.byref(database))
assert not lib.duckdb_connect(database, c.byref(connection))


def sql(statement):
    result = Result()
    status = lib.duckdb_query(connection, statement.encode(), c.byref(result))
    error = result.error.decode() if result.error else 'SQL failed'
    lib.duckdb_destroy_result(c.byref(result))
    if status:
        raise RuntimeError(error)


def quote(value):
    if value is None:
        return 'NULL'
    if isinstance(value, bool):
        return str(value).lower()
    if isinstance(value, int):
        return str(value)
    return "'" + str(value).replace("'", "''") + "'"


observations, assets, current = [], [], []
def add(image, row_id, post_id, rating, tags, created=None, observed=None, quality='exact', fav=None, up=None, down=None, make_current=True):
    n = image['number']
    if V2_TAGS and n >= 50 and tags is not None:
        if n % 7 == 0:
            tags += ' comic full_page_comic vertical_scroll_comic'
        if n % 11 == 0:
            tags += ' comic full_page_comic 4koma cut-in'
    created = created or dt.datetime(2024, 1, 1, tzinfo=dt.timezone.utc) + dt.timedelta(days=n % 540, hours=n % 24)
    observed = observed or created + dt.timedelta(days=[1, 5, 12, 25, 60, 300, 800, 1500, 3000][n % 9])
    values = {'row_id': row_id, 'observation_id': f'{row_id:064x}', 'post_id': post_id,
              'source_key': 'ranking-fixture', 'source_kind': 'api_json', 'source_row': row_id,
              'archive_row': row_id, 'observed_at': observed.isoformat() if observed else None,
              'time_quality': quality, 'ingested_at': '2026-09-09T00:00:00Z',
              'issues_json': '[]', 'publication_kind': 'fixture', 'publication_group': 'fixture',
              'source_priority': 10, 'rating': rating, 'tag_string': tags,
              'tag_string_general': tags, 'tag_string_artist': 'artist_' + str(n % 17),
              'tag_string_character': '', 'tag_string_copyright': '', 'tag_string_meta': '',
              'source': '', 'md5': image['md5'], 'file_ext': 'png',
              'file_url': None, 'large_file_url': None, 'preview_file_url': None,
              'created_at': created.isoformat(), 'updated_at': observed.isoformat() if observed else None,
              'fav_count': n % 53 if fav is None else fav, 'up_score': n % 47 if up is None else up,
              'down_score': -(n % 6) if down is None else down,
              'score': (n % 47) - (n % 6), 'image_width': 9999, 'image_height': 9999,
              'file_size': image['bytes'], 'tag_count': len(tags.split()) if tags is not None else None,
              'uploader_id': 1, 'parent_id': 70000 if 20 <= n < 40 else None, 'pixiv_id': None,
              'is_deleted': n % 21 == 0, 'is_banned': n % 31 == 0, 'is_pending': False,
              'is_flagged': False, 'batch_id': 'fixture', 'commit_seq': 1}
    if n == 4:
        values.update(fav_count=None, up_score=None, down_score=None, score=None)
    if n == 5:
        values['observed_at'] = (created - dt.timedelta(days=100)).isoformat()
    if n == 6:
        values['created_at'] = '2026-05-18T12:00:00Z'
        values['observed_at'] = '2026-05-18T00:00:00Z'
        values['time_quality'] = 'date_only'
    if n % 19 == 0:
        values['observed_at'] = None
        values['time_quality'] = 'unknown'
    if n == 8:
        values['tag_string_artist'] = 'artist_1 artist_2'
    if n == 14:
        values['rating'] = None
    if n % 23 == 0:
        values['tag_string_artist'] = None
    observations.append(values)
    aid = hashlib.sha256(('asset:' + str(row_id)).encode()).hexdigest()
    details = {} if n % 3 == 0 or n == 11 else {'stored_width': image['width'], 'stored_height': image['height']}
    raw = {} if n == 11 else {'raw_stored_width': image['width'], 'raw_stored_height': image['height']}
    asset = {'asset_id': aid, 'observation_id': values['observation_id'], 'post_id': post_id, 'sha256': image['sha'],
             'source_md5': image['md5'], 'stored_ext': 'png', 'stored_bytes': image['bytes'], 'storage_profile': 'fixture',
             'details_json': json.dumps(details), 'batch_id': 'fixture', 'commit_seq': 1}
    assets.append(asset)
    values['raw'] = raw
    if make_current:
        current.append((post_id, row_id, aid))
    return values


for image in images[:N]:
    n = image['number']
    if n == 15:
        continue
    tags = 'common alpha ' + ('red' if n % 2 == 0 else 'blue')
    if SPARSE_BROWSE and (n % 4 == 1 or n == 7):
        tags += ' sparse_browse'
    if n == 9:
        tags += ' not_jpeg_artifacts'
    if n == 10:
        tags += ' jpeg_artifacts scan_artifacts'
    add(image, n + 1, 10000 + n, 'gsqe'[n % 4], tags, make_current=n != 3)
extra_post_base = max(20001, 10000 + N)
add(images[1], N + 1, extra_post_base, 'e', 'common beta', observed=dt.datetime(2040, 1, 1, tzinfo=dt.timezone.utc), fav=999, up=800)
add(images[2], N + 2, extra_post_base + 1, 'e', 'common beta', observed=dt.datetime(2040, 1, 1, tzinfo=dt.timezone.utc), fav=777, up=700)
add(images[N], N + 3, 10003, 'e', 'common replacement', observed=dt.datetime(2040, 1, 1, tzinfo=dt.timezone.utc))

if DUPLICATE_HEAT:
    original = next(o for o in observations if o['post_id'] == 10007)
    original.update(rating='g', fav_count=50, up_score=32, down_score=0, score=32,
                    observed_at='2026-05-18T00:00:00Z', time_quality='date_only',
                    updated_at='2026-01-01T00:00:00Z', created_at='2020-07-22T00:00:00Z', is_deleted=False)
    duplicate = add(images[7], N + 4, 4529147, 's', 'common duplicate pixel-perfect_duplicate',
                    created=dt.datetime(2021,5,18,tzinfo=dt.timezone.utc),
                    observed=dt.datetime(2026,5,18,tzinfo=dt.timezone.utc),quality='date_only',fav=12,up=7,down=0)
    duplicate.update(score=7,is_deleted=True,parent_id=10007,updated_at='2026-05-11T00:00:00Z')
    history = add(images[7], N + 5, 10007, 'q', 'old common',
                  observed=dt.datetime(2024,1,1,tzinfo=dt.timezone.utc),fav=9999,up=9999,down=0,make_current=False)
    history['score'] = 9999
    for o in [original,duplicate,history]:
        o['raw'].update({k:o[k] for k in ['rating','score','fav_count','up_score','down_score','is_deleted','parent_id']})
        o['raw']['id'] = o['post_id']

try:
    sql("SET threads=2; SET memory_limit='512MB'; CREATE TABLE applied(seq BIGINT,batch_id VARCHAR); INSERT INTO applied VALUES(1,'fixture'); CREATE TABLE objects(sha256 VARCHAR PRIMARY KEY,pack_path VARCHAR,\"offset\" BIGINT,length BIGINT,stored_ext VARCHAR); CREATE TABLE current_posts(post_id BIGINT PRIMARY KEY,row_id BIGINT,asset_id VARCHAR); CREATE TABLE assets(asset_id VARCHAR PRIMARY KEY,observation_id VARCHAR,post_id BIGINT,sha256 VARCHAR,source_md5 VARCHAR,stored_ext VARCHAR,stored_bytes BIGINT,storage_profile VARCHAR,details_json VARCHAR,batch_id VARCHAR,commit_seq BIGINT); CREATE INDEX assets_sha ON assets(sha256); CREATE TABLE raw_metadata(observation_id VARCHAR PRIMARY KEY,source_metadata_json VARCHAR,source_metadata_format VARCHAR,source_schema_id VARCHAR);")
    names = [key for key in observations[0] if key != 'raw']
    bools = {'is_deleted', 'is_banned', 'is_pending', 'is_flagged'}
    dates = {'observed_at', 'ingested_at', 'created_at', 'updated_at'}
    ints = {'row_id', 'post_id', 'source_row', 'archive_row', 'source_priority', 'fav_count', 'up_score', 'down_score', 'score', 'image_width', 'image_height', 'file_size', 'tag_count', 'uploader_id', 'parent_id', 'pixiv_id', 'commit_seq'}
    sql('CREATE TABLE observations(' + ','.join(k + ' ' + ('BOOLEAN' if k in bools else 'TIMESTAMPTZ' if k in dates else 'BIGINT' if k in ints else 'VARCHAR') for k in names) + '); CREATE INDEX observations_row ON observations(row_id); CREATE INDEX observations_post ON observations(post_id);')
    def insert(table, columns, rows):
        for start in range(0, len(rows), 256):
            sql('INSERT INTO ' + table + '(' + ','.join('"' + name + '"' for name in columns) + ') VALUES ' + ','.join('(' + ','.join(quote(value) for value in row) + ')' for row in rows[start:start + 256]))
    insert('objects', ['sha256', 'pack_path', 'offset', 'length', 'stored_ext'], [(i['sha'], 'packs/fixture.tar', i['offset'], i['bytes'], 'png') for i in images])
    insert('observations', names, [[o[k] for k in names] for o in observations])
    insert('assets', list(assets[0]), [list(a.values()) for a in assets])
    insert('current_posts', ['post_id', 'row_id', 'asset_id'], current)
    insert('raw_metadata', ['observation_id', 'source_metadata_json', 'source_metadata_format', 'source_schema_id'], [(o['observation_id'], json.dumps(o['raw']), 'json', 'fixture') for o in observations])
    sql('CHECKPOINT')
finally:
    lib.duckdb_disconnect(c.byref(connection))
    lib.duckdb_close(c.byref(database))

manifest = {'library_id': LIBRARY_ID, 'lake': str(LAKE), 'objects': images, 'observations': observations, 'assets': assets, 'current': current}
(OUT / 'fixture.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding='utf-8')
print(json.dumps({'objects': len(images), 'observations': len(observations), 'fixture': str(OUT / 'fixture.json')}))
