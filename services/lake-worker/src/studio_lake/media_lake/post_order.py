"""Rebuildable object ordering from immutable Pixiv work/page associations.

The publisher owns this projection. Readers never upgrade or write lake indexes.
Intervals retain the minimum (numeric work ID, page ordinal) at every watermark.
"""

from itertools import groupby

from ..util import IntegrityError


SCHEMA = """
CREATE TABLE IF NOT EXISTS pixiv_object_order(
  sha256 TEXT NOT NULL, valid_from INTEGER NOT NULL, valid_until INTEGER,
  post_id TEXT, page_ordinal INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(sha256,valid_from)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS pixiv_post_asc
  ON pixiv_object_order(post_id,page_ordinal,sha256,valid_from,valid_until);
CREATE INDEX IF NOT EXISTS pixiv_post_desc
  ON pixiv_object_order(post_id DESC,page_ordinal ASC,sha256 DESC,valid_from,valid_until);
"""

# The source contract accepts up to 20 decimal digits, beyond SQLite's i64.
NUMERIC_ID = "length(m.work_id) BETWEEN 1 AND 20 AND m.work_id NOT GLOB '*[^0-9]*' AND m.work_id NOT LIKE '0%'"
POST_ID = "substr('00000000000000000000'||m.work_id,-20)"


def ensure(db, stop=None):
    """Resume a bounded backfill of an older online-v3 index under .online.lock."""
    end = int(db.execute("SELECT value FROM online_state WHERE key='served_seq'").fetchone()[0])
    ready = db.execute("SELECT value FROM online_state WHERE key='pixiv_post_order_version'").fetchone() == ("1",)
    covered = db.execute("SELECT value FROM online_state WHERE key='pixiv_post_order_seq'").fetchone()
    if ready and covered and int(covered[0]) >= end:
        return
    with db:
        db.execute(SCHEMA)
        if ready:
            # An older publisher may have advanced the lake without maintaining
            # this derived index. Hide it and rebuild from retained immutable facts.
            db.execute('DELETE FROM pixiv_object_order')
            db.execute("DELETE FROM online_state WHERE key IN ('pixiv_post_order_version','pixiv_post_order_after','pixiv_post_order_seq')")
    saved = db.execute("SELECT value FROM online_state WHERE key='pixiv_post_order_after'").fetchone()
    after = int(saved[0]) if saved else 0
    while True:
        rows = list(db.execute("SELECT object_row,sha256,first_seq FROM objects WHERE object_row>? AND first_seq<=? AND media_category='image' ORDER BY object_row LIMIT 256", (after, end)))
        if not rows:
            break
        with db:
            for _, sha, first in rows:
                start, best = first, None
                origins = db.execute(f"SELECT a.commit_seq,{POST_ID},m.ordinal FROM assets a JOIN media_entries m USING(media_id) WHERE a.sha256=? AND a.commit_seq<=? AND {NUMERIC_ID} ORDER BY a.commit_seq", (sha, end))
                for seq, group in groupby(origins, key=lambda row: row[0]):
                    candidate = min((row[1], row[2]) for row in group)
                    if best is not None and candidate >= best:
                        continue
                    if start < seq:
                        db.execute("INSERT INTO pixiv_object_order VALUES(?,?,?,?,?)", (sha, start, seq, best[0] if best else None, best[1] if best else 0))
                    start, best = seq, candidate
                db.execute("INSERT INTO pixiv_object_order VALUES(?,?,NULL,?,?)", (sha, start, best[0] if best else None, best[1] if best else 0))
            after = rows[-1][0]
            db.execute("INSERT INTO online_state VALUES('pixiv_post_order_after',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", (str(after),))
        if stop:
            stop("post_order_backfill", end)
    with db:
        db.execute("INSERT INTO online_state VALUES('pixiv_post_order_version','1') ON CONFLICT(key) DO UPDATE SET value=excluded.value")
        db.execute("INSERT INTO online_state VALUES('pixiv_post_order_seq',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", (str(end),))
        db.execute("DELETE FROM online_state WHERE key='pixiv_post_order_after'")


def project(db, records, seq):
    changed = {r['sha256'] for name in ('objects', 'assets') for r in records.get(name, [])}
    for sha in sorted(changed):
        with db:
            if not db.execute("SELECT 1 FROM objects WHERE sha256=? AND first_seq<=? AND media_category='image'", (sha, seq)).fetchone():
                continue
            best = db.execute(f"SELECT {POST_ID} AS post_id,m.ordinal FROM assets a JOIN media_entries m USING(media_id) WHERE a.sha256=? AND a.commit_seq<=? AND {NUMERIC_ID} ORDER BY post_id,m.ordinal LIMIT 1", (sha, seq)).fetchone() or (None, 0)
            old = db.execute("SELECT valid_from,post_id,page_ordinal FROM pixiv_object_order WHERE sha256=? AND valid_until IS NULL", (sha,)).fetchone()
            if old and old[1:] == best:
                continue
            if old and old[0] == seq:
                raise IntegrityError("Inconsistent Pixiv object order replay")
            db.execute("UPDATE pixiv_object_order SET valid_until=? WHERE sha256=? AND valid_until IS NULL", (seq, sha))
            db.execute("INSERT INTO pixiv_object_order VALUES(?,?,NULL,?,?)", (sha, seq, *best))
