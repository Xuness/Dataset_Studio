import pytest

from pixiv_fixtures import add_images, add_work, new_batch, sample
from studio_lake.media_lake.online import Publisher, rebuild
from studio_lake.media_lake.post_order import ensure
from studio_lake.online_storage import connect


def positions(db, seq):
    return list(db.execute("SELECT sha256,post_id,page_ordinal FROM pixiv_object_order WHERE valid_from<=? AND (valid_until IS NULL OR valid_until>?) ORDER BY post_id NULLS LAST,page_ordinal,sha256", (seq, seq)))


def test_numeric_work_ids_pages_dedup_history_and_rebuild(tmp_path):
    lib, state = sample(tmp_path)
    records, _, _ = add_work(lib, state, work_id="100", pages=3)
    ids = []
    for entry, color in zip(records['media_entries'], ('red', 'green', 'blue')):
        sha, _, _ = add_images(lib, state, {'media_entries': [entry]}, color=color)
        ids.append(sha)
    lib.sync_online()
    db = connect(lib.cache / 'online.sqlite')
    old = int(db.execute("SELECT value FROM online_state WHERE key='served_seq'").fetchone()[0])
    assert [(int(row[1]), row[2]) for row in positions(db, old)] == [(100, 0), (100, 1), (100, 2)]
    db.close()
    # Same bytes in another work move exactly once, while retained pages stay stable.
    smaller, _, _ = add_work(lib, state, work_id="9", pages=1)
    duplicate, _, _ = add_images(lib, state, smaller, color='blue')
    assert duplicate == ids[2]
    large, _, _ = add_work(lib, state, work_id="99999999999999999999", pages=1)
    huge, _, _ = add_images(lib, state, large, color='yellow')
    batch = new_batch(lib, state)
    orphan = batch.add_blob(b'orphan', 'png', content_type='image/png')
    with lib.writer_lock():
        latest = batch.commit()
    lib.sync_online()
    db = connect(lib.cache / 'online.sqlite')
    assert [row[0] for row in positions(db, latest)] == [ids[2], ids[0], ids[1], huge, orphan]
    assert [row[0] for row in positions(db, old)] == ids
    expected = list(db.execute('SELECT * FROM pixiv_object_order ORDER BY sha256,valid_from'))
    # Simulate an older online-v3 database and interrupt its resumable upgrade.
    with db:
        db.execute('DROP TABLE pixiv_object_order')
        db.execute("DELETE FROM online_state WHERE key='pixiv_post_order_version'")
    def stop(*_):
        raise RuntimeError('injected backfill interruption')
    with pytest.raises(RuntimeError, match='backfill interruption'):
        ensure(db, stop=stop)
    assert not db.execute("SELECT 1 FROM online_state WHERE key='pixiv_post_order_version'").fetchone()
    db.close()
    Publisher(lib).prepare_post_order()
    db = connect(lib.cache / 'online.sqlite')
    assert list(db.execute('SELECT * FROM pixiv_object_order ORDER BY sha256,valid_from')) == expected
    assert db.execute("SELECT value FROM online_state WHERE key='pixiv_post_order_seq'").fetchone() == (str(latest),)
    # Reopening a lake touched by an older publisher must not trust stale order rows.
    with db:
        db.execute("UPDATE online_state SET value='0' WHERE key='pixiv_post_order_seq'")
    ensure(db)
    assert list(db.execute('SELECT * FROM pixiv_object_order ORDER BY sha256,valid_from')) == expected
    assert db.execute('PRAGMA integrity_check').fetchone() == ('ok',)
    for direction in ('ASC', 'DESC'):
        plans = db.execute(f"EXPLAIN QUERY PLAN SELECT sha256 FROM pixiv_object_order WHERE post_id>? AND valid_from<=? AND (valid_until IS NULL OR valid_until>?) ORDER BY post_id {direction},page_ordinal ASC,sha256 {direction} LIMIT 48", ('00000000000000000009', latest, latest))
        assert not any('TEMP B-TREE' in row[-1] for row in plans)
    db.close()
    rebuilt = tmp_path / 'rebuilt'
    rebuild(lib, rebuilt)
    db = connect(rebuilt / 'online.sqlite')
    assert list(db.execute('SELECT * FROM pixiv_object_order ORDER BY sha256,valid_from')) == expected
    db.close()
