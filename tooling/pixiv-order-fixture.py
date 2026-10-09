"""Offline numeric-ID/page-order fixture with unique pages and shared objects."""
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / 'services/lake-worker/src'), str(ROOT / 'services/lake-worker/tests')]
from pixiv_fixtures import add_images, add_work, new_batch, sample
from studio_lake.config import Config
from studio_lake.media_lake.library import MediaLibrary
from studio_lake.util import atomic_json

output = Path(sys.argv[1]).resolve()
if not output.is_relative_to(ROOT / '.local/test-runs'):
    raise ValueError('fixture must stay in .local/test-runs')
path = output / 'pixiv-order.json'
if len(sys.argv) > 2 and sys.argv[2] == 'append':
    refs = json.loads(path.read_text(encoding='utf-8'))
    lib = MediaLibrary(Config(Path(refs['media_root']), Path(refs['index_root'])))
    state = refs['state']
    records, _, _ = add_work(lib, state, work_id='2', pages=1, tags=['order_fixture'])
    sha, _, _ = add_images(lib, state, records, color=(20, 40, 60))
    refs['positions'][sha] = ['2', 0]
else:
    lib, state = sample(output)
    refs = dict(media_root=str(lib.root), index_root=str(lib.cache), state=state, positions={})
    for number, (work, pages) in enumerate([('100', 15), ('9', 4), ('10', 14), ('99999999999999999999', 2)]):
        records, _, _ = add_work(lib, state, work_id=work, pages=pages, tags=['order_fixture'])
        for entry in records['media_entries']:
            ordinal = entry['ordinal']
            color = (20 + number * 40, 40 + ordinal * 10, 60)
            # Duplicate bytes at different work/page positions, not duplicate cards.
            if work == '9' and ordinal == 3:
                color = (20, 40, 60)
            sha, _, _ = add_images(lib, state, {'media_entries': [entry]}, color=color)
            previous = refs['positions'].get(sha)
            if previous is None or (int(work), ordinal) < (int(previous[0]), previous[1]):
                refs['positions'][sha] = [work, ordinal]
    batch = new_batch(lib, state)
    # Reuse a valid PNG as an unlinked object with distinct contents.
    import io
    from PIL import Image
    data = io.BytesIO()
    Image.new('RGB', (12, 9), 'black').save(data, format='PNG')
    refs['unlinked'] = batch.add_blob(data.getvalue(), 'png', content_type='image/png', width=12, height=9)
    with lib.writer_lock():
        batch.commit()
lib.sync_online()
atomic_json(path, refs)
print(json.dumps({'fixture': str(path)}))
