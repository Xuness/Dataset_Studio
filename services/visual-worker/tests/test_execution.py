import hashlib
from io import BytesIO

import numpy as np
from PIL import Image
import pytest

from studio_visual import execution
from studio_visual.vectors import Vectors


class FakeModels:
    calls = []

    def __init__(self, device):
        pass

    def encode_bounded(self, recipe, images):
        self.calls.append((recipe["id"], len(images)))
        if recipe.get("fail"):
            raise RuntimeError("deliberate model failure")
        result = np.zeros((len(images), recipe["dimension"]), dtype=np.float32)
        for n, image in enumerate(images):
            result[n, np.asarray(image).sum().item() % recipe["dimension"]] = 1
        return result


@pytest.fixture(autouse=True)
def fake_models(monkeypatch):
    FakeModels.calls = []
    monkeypatch.setattr(execution, "Models", FakeModels)


def recipe(n=1, dimension=256, fail=False):
    return {"id": f"{n:064x}", "dimension": dimension, "fail": fail}


def images(root):
    data = []
    for n in range(4):
        output = BytesIO()
        Image.new("RGBA", (8 + n, 12), (n * 40, 90, 220, 160)).save(output, format="PNG")
        data.append(output.getvalue())
    output = BytesIO()
    Image.new("RGB", (8, 8), "red").save(output, format="GIF", save_all=True,
        append_images=[Image.new("RGB", (8, 8), "blue")], duration=10, loop=0)
    data.extend([output.getvalue(), b"broken image"])
    pack = root / "pack.tar"
    pack.write_bytes(b"".join(data))
    rows, offset = [], 0
    for n, raw in enumerate(data):
        rows.append({"object_id": n + 1, "sha256": hashlib.sha256(raw).hexdigest(),
                     "source_ids": [1], "source_id": "lake", "media_root": str(root),
                     "pack": "pack.tar", "offset": offset, "length": len(raw), "hash_done": False})
        offset += len(raw)
    return rows


def request(rows, recipes, batch="one"):
    return {"batch_id": batch, "items": rows, "recipes": recipes, "projection_recipes": recipes}


def test_shared_read_partial_model_failure_and_animation(tmp_path):
    rows = images(tmp_path)
    recipes = [recipe(), recipe(2, 768, True), recipe(3, 1152)]
    worker = execution.Execution(tmp_path / "features", "cpu")
    prepared = worker.prepare(request(rows, recipes))
    assert prepared["bytes_read"] == sum(r["length"] for r in rows)
    assert prepared["opens"] == 1
    result = worker.finish("one")
    assert len(result["object_ids"]) == 6
    assert len([r for r in result["results"] if r["status"] == "success"]) == 12
    assert any(r["error"] == "unsupported_animation" for r in result["results"])
    assert len(result["versions"]) == 2
    assert sorted(n for _, n in FakeModels.calls) == [4, 4, 4]
    assert worker.prepare(request(rows, recipes))["bytes_read"] == 0
    assert worker.finish("one") == result


def test_crash_after_lance_commit_recovers_without_reinference(tmp_path, monkeypatch):
    rows = images(tmp_path)[:2]
    recipes = [recipe()]
    root = tmp_path / "features"
    worker = execution.Execution(root, "cpu")
    worker.prepare(request(rows, recipes))
    original = execution.atomic_json

    def crash(*_):
        raise RuntimeError("power loss after vector commit")

    monkeypatch.setattr(execution, "atomic_json", crash)
    with pytest.raises(RuntimeError, match="power loss"):
        worker.finish("one")
    assert worker.vectors.table(recipes[0]).count_rows() == 2
    assert len(FakeModels.calls) == 1
    monkeypatch.setattr(execution, "atomic_json", original)
    restarted = execution.Execution(root, "cpu")
    restarted.prepare(request(rows, recipes))
    result = restarted.finish("one")
    assert all(r["status"] == "success" for r in result["results"])
    assert len(FakeModels.calls) == 1
    assert restarted.vectors.table(recipes[0]).count_rows() == 2


def test_sources_and_search_obey_published_versions_without_image_reads(tmp_path):
    rows = images(tmp_path)[:1]
    recipes = [recipe()]
    worker = execution.Execution(tmp_path / "features", "cpu")
    worker.prepare(request(rows, recipes))
    first = worker.finish("one")["versions"][recipes[0]["id"]]
    rows[0].update(source_ids=[1, 2], hash_done=True)
    (tmp_path / "pack.tar").unlink()
    prepared = worker.prepare(request(rows, recipes, "two"))
    assert prepared["bytes_read"] == 0
    second = worker.finish("two")["versions"][recipes[0]["id"]]
    vectors = Vectors(tmp_path / "features")
    assert not vectors.search(recipes[0], first, 1, None, [2], 10, True, 1)["items"]
    result = vectors.search(recipes[0], second, 1, None, [2], 10, True, 1)
    assert result["items"][0]["object_id"] == 1
    assert abs(result["items"][0]["distance"]) < 1e-6
    assert len(FakeModels.calls) == 1


def test_sha_and_root_validation_before_model_execution(tmp_path):
    rows = images(tmp_path)[:2]
    rows[0]["sha256"] = "0" * 64
    rows[1]["pack"] = "../pack.tar"
    worker = execution.Execution(tmp_path / "features", "cpu")
    worker.prepare(request(rows, [recipe()]))
    result = worker.finish("one")
    assert {r["error"] for r in result["results"]} == {"sha256_mismatch", "range_path_invalid"}
    assert not FakeModels.calls


def test_pixel_budget_chunks_decode_without_failing_valid_images(tmp_path, monkeypatch):
    rows = images(tmp_path)[:4]
    monkeypatch.setattr(execution, "MAX_DECODE_PIXELS", 200)
    worker = execution.Execution(tmp_path / "features", "cpu")
    worker.prepare(request(rows, [recipe()]))
    result = worker.finish("one")
    assert result["object_ids"] == [1, 2, 3, 4]
    assert [n for _, n in FakeModels.calls] == [1, 1, 1, 1]
    assert all(r["status"] == "success" for r in result["results"])


def test_independent_micro_batches_and_persisted_metrics(tmp_path):
    rows = images(tmp_path)[:4]
    recipes = [{**recipe(), "model": "siglip2_naflex"}, {**recipe(2), "model": "kaloscope3"}]
    value = request(rows, recipes)
    value["micro_batch_sizes"] = {"siglip2_naflex": 3, "kaloscope3": 2}
    worker = execution.Execution(tmp_path / "features", "cpu")
    worker.prepare(value)
    result = worker.finish("one")
    assert [n for rid, n in FakeModels.calls if rid == recipes[0]["id"]] == [3, 1]
    assert [n for rid, n in FakeModels.calls if rid == recipes[1]["id"]] == [2, 2]
    assert result["metrics"]["decoded_images"] == 4
    assert result["metrics"]["peak_rss_bytes"] > 0
    assert result["metrics"]["model_batches"] == {r["id"]: 2 for r in recipes}
    assert worker.finish("one")["metrics"] == result["metrics"]


def test_maintenance_failure_stops_batch_and_replays_committed_vectors(tmp_path, monkeypatch):
    rows = images(tmp_path)[:2]
    recipes = [recipe()]
    worker = execution.Execution(tmp_path / "features", "cpu")
    def fail(*args):
        raise OSError("maintenance disk failure")
    monkeypatch.setattr(worker.vectors, "_maintain", fail)
    worker.prepare(request(rows, recipes))
    with pytest.raises(OSError, match="maintenance disk failure"):
        worker.finish("one")
    assert not worker.receipt_path("one").exists()
    assert worker.vectors.table(recipes[0]).count_rows() == 2
    restarted = execution.Execution(tmp_path / "features", "cpu")
    restarted.prepare(request(rows, recipes))
    result = restarted.finish("one")
    assert all(r["status"] == "success" for r in result["results"])
    assert len(FakeModels.calls) == 1


def test_model_load_failure_keeps_batch_unconfirmed(tmp_path, monkeypatch):
    worker = execution.Execution(tmp_path / "features", "cpu")
    def fail(*args):
        raise execution.ModelLoadError("weights changed")
    monkeypatch.setattr(worker.models, "encode_bounded", fail)
    worker.prepare(request(images(tmp_path)[:2], [recipe()]))
    with pytest.raises(execution.ModelLoadError, match="weights changed"):
        worker.finish("one")
    assert not worker.receipt_path("one").exists()
