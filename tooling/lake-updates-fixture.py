"""Build isolated v2 lakes; --assets adds synthetic images without network I/O."""

from pathlib import Path
import json
import sys

root = Path(sys.argv[1]).resolve()
worker = Path(__file__).resolve().parents[1] / "services/lake-worker"
sys.path.insert(0, str(worker / "src"))
from studio_lake.config import Config
from studio_lake.library import Library
from studio_lake.index import Index
from studio_lake.online_migrate import migrate, verify_projection, activate_projection
from studio_lake.util import atomic_json

items = []
for site in ["danbooru", "yandere", "gelbooru"]:
    directory = root / site
    lib = Library.initialize(Config(root=directory / "archive", cache=directory / "index", threads=1, memory_limit="1GB"))
    if site != "danbooru":
        atomic_json(lib.root / "source_manifests/hf-conversion-plan.json", {"site": site, "normalizer": "hf_" + site + "_v1"})
    Index(lib).sync()
    migrate(lib.root, lib.cache, lib.cache, site)
    verify_projection(lib.cache)
    activate_projection(lib.root, lib.cache)
    items.append({"library_id": lib.info["library_id"], "site": site, "media_root": str(lib.root), "index_root": str(lib.cache)})
    if "--assets" in sys.argv:
        # Use the Studio worker's recorded-site fixture, never a real API.
        import importlib.util
        sys.path.insert(0, str(worker / "tests"))
        module_spec = importlib.util.spec_from_file_location("lake_update_fixture", worker / "tests/test_updates.py")
        support = importlib.util.module_from_spec(module_spec)
        module_spec.loader.exec_module(support)
        state = support.State(root / "controller")
        state.register(items[-1])
        data = support.png({"danbooru":"red", "yandere":"green", "gelbooru":"blue"}[site])
        records = [support.post(site, pid, data) for pid in (11, 12)]
        task = support.job(state, lib, {"kind":"ids", "ids":[11,12]}, "original")
        outcome = support.Runner(state, {site:support.FakeSite(site,records)}, support.Resources(reserve_bytes=0), support.Images(data)).run(task["id"])
        if outcome["state"] != "completed":
            raise RuntimeError("Synthetic lake preparation failed: " + outcome["state"])
atomic_json(root / "targets.json", items)
print(json.dumps({"targets": len(items)}))
