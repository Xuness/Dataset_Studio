import numpy as np
import pytest
from studio_visual.models import normalized
from studio_visual.vectors import Vectors


@pytest.mark.parametrize("dimension", [1152, 768, 256])
def test_full_float_streaming_ivf_and_reopen(tmp_path, dimension):
    recipe = {"id": f"{dimension:064x}", "dimension": dimension}
    vectors = Vectors(tmp_path)
    values = normalized(np.random.default_rng(17).standard_normal((320, dimension)), dimension)
    rows = [{"object_id": n + 1, "source_ids": [1 if n % 2 == 0 else 2]} for n in range(320)]
    vectors.append(recipe, "initial", rows, values)
    published = vectors.project_sources(recipe, rows[:1])
    # Verify full float32 values, without converting the complete table to Arrow.
    total = 0
    for batch in vectors.table(recipe).search().select(["object_id", "vector"]).to_batches(batch_size=41):
        assert len(batch) <= 41
        for item in batch.to_pylist():
            np.testing.assert_allclose(item["vector"], values[item["object_id"] - 1], rtol=1e-6, atol=1e-7)
            total += 1
    assert total == 320
    built = vectors.index(recipe, 4)
    reopened = Vectors(tmp_path)
    exact = reopened.search(recipe, published, 1, None, [1], 8, True, 4)
    indexed = reopened.search(recipe, built["version"], 1, None, [1], 8, False, 4)
    assert indexed["method"] == "ivf_flat"
    assert [r["object_id"] for r in exact["items"]] == [r["object_id"] for r in indexed["items"]]
    vectors.append(recipe, "replayed", rows[:8], values[:8])
    assert vectors.table(recipe).count_rows() == 320


def test_invalid_vectors_are_rejected():
    for value in ([[0, 0]], [[float("nan"), 1]], [[float("inf"), 1]], [[1, 2, 3]]):
        with pytest.raises(ValueError):
            normalized(value, 2)


def test_fragment_maintenance_keeps_published_versions(tmp_path):
    recipe = {"id": "f" * 64, "dimension": 256}
    vectors = Vectors(tmp_path)
    values = normalized(np.ones((1, 256)), 256)
    initial = None
    for n in range(40):
        rows = [{"object_id": n + 1, "source_ids": [1]}]
        vectors.append(recipe, f"batch-{n}", rows, values)
        version = vectors.project_sources(recipe, rows)
        if initial is None:
            initial = version
    table = vectors.table(recipe)
    assert table.count_rows() == 40
    assert any("object_id" in index.columns for index in table.list_indices())
    table.checkout(initial)
    assert table.count_rows() == 1


def test_snapshot_entries_and_arrow_export_keep_exact_floats(tmp_path):
    import hashlib
    from pathlib import Path
    import pyarrow as pa

    recipe = {"id": "e" * 64, "dimension": 1152}
    vectors = Vectors(tmp_path)
    values = normalized(np.random.default_rng(41).normal(size=(10, 1152)), 1152)
    rows = [{"object_id": n + 1, "source_ids": [1]} for n in range(10)]
    vectors.append(recipe, "old", rows[:5], values[:5])
    old = vectors.project_sources(recipe, rows[:1])
    vectors.pin(recipe, old, "snapshot_frozen")
    vectors.append(recipe, "new", rows[5:], values[5:])
    assert vectors.entries(recipe, old, list(range(1, 11)))["object_ids"] == list(range(1, 6))
    exported = vectors.export_page(recipe, old, [2, 4, 6, 8])
    assert exported["rows"] == 2
    path = Path(exported["path"])
    assert hashlib.sha256(path.read_bytes()).hexdigest() == exported["sha256"]
    table = pa.ipc.open_file(path).read_all()
    assert b"studio_visual" in table.schema.metadata
    for row in table.to_pylist():
        np.testing.assert_array_equal(np.asarray(row["vector"], dtype=np.float32), values[row["object_id"] - 1])
    assert vectors.table(recipe).tags.get_version("snapshot_frozen") == old
