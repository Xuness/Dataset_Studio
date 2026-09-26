"""Build isolated empty v2 lakes for the update-control integration suite."""

from pathlib import Path
import json
import sys

root, store = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()
sys.path.insert(0, str(store / "src"))
from danbooru_store.config import Config
from danbooru_store.library import Library
from danbooru_store.index import Index
from danbooru_store.online_migrate import migrate, verify_projection, activate_projection
from danbooru_store.util import atomic_json

items = []
for site in ["danbooru", "yandere", "gelbooru"]:
    directory = root / site
    lib = Library.initialize(Config(root=directory / "archive", cache=directory / "index", threads=1, memory_limit="1GB"))
    if site != "danbooru":
        atomic_json(lib.root / "source_manifests/hf-conversion-plan.json", {"site": site})
    Index(lib).sync()
    migrate(lib.root, lib.cache, lib.cache, site)
    verify_projection(lib.cache)
    activate_projection(lib.root, lib.cache)
    items.append({"library_id": lib.info["library_id"], "site": site, "media_root": str(lib.root), "index_root": str(lib.cache)})
atomic_json(root / "targets.json", items)
print(json.dumps({"targets": len(items)}))
