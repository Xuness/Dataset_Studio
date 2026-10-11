import json
import uuid

import numpy as np
import pytest

from studio_visual import indexing
from studio_visual.models import normalized
from studio_visual.vectors import Vectors


def fixture(tmp_path):
    vectors = Vectors(tmp_path)
    recipe = {"id": "c" * 64, "dimension": 256}
    values = normalized(np.random.default_rng(51).normal(size=(512, 256)), 256)
    rows = [{"object_id": n + 1, "source_ids": [1 + n % 2]} for n in range(512)]
    vectors.append(recipe, "initial", rows, values)
    version = vectors.project_sources(recipe, rows[:1])
    return vectors, recipe, version


def test_index_reports_real_progress_and_reconciles_committed_receipt(tmp_path, monkeypatch):
    vectors, recipe, version = fixture(tmp_path)
    identity = str(uuid.uuid4())
    original = indexing.atomic_json
    events = []
    def interrupted(path, value):
        if path.name == "result.json":
            raise OSError("lost receipt after index commit")
        if path.name == "progress.json":
            events.append(value)
        original(path, value)
    monkeypatch.setattr(indexing, "atomic_json", interrupted)
    with pytest.raises(OSError, match="lost receipt"):
        indexing.run_index(vectors, recipe, identity, version, partitions=4, max_iterations=3)
    first_version = vectors.table(recipe).version
    assert any(event["phase"] not in ("compacting", "scalar_indexes", "vector_index") for event in events)
    assert any(str(segment.uuid) == identity for index in vectors.table(recipe).to_lance().describe_indices() for segment in index.segments)
    monkeypatch.setattr(indexing, "atomic_json", original)
    recovered = indexing.run_index(vectors, recipe, identity, version, partitions=4, max_iterations=3)
    assert recovered["version"] == first_version
    assert recovered["indexed_rows"] == 512 and recovered["unindexed_rows"] == 0
    assert vectors.table(recipe).tags.get_version("index_input_" + identity) == version
    indexing.release_input(vectors, recipe, identity)
    assert "index_input_" + identity not in vectors.table(recipe).tags.list()
    current = json.loads((indexing.job_directory(vectors, identity) / "progress.json").read_text())
    assert current["phase"] == "ready_to_publish"
    old = vectors.search(recipe, version, 1, None, [1], 10, True, 4)
    new = vectors.search(recipe, recovered["version"], 1, None, [1], 10, False, 4)
    assert [r["object_id"] for r in old["items"]] == [r["object_id"] for r in new["items"]]


def test_checkpoint_rejects_changed_request(tmp_path):
    vectors, recipe, version = fixture(tmp_path)
    identity = str(uuid.uuid4())
    indexing.run_index(vectors, recipe, identity, version, partitions=4, max_iterations=3)
    with pytest.raises(ValueError, match="different input"):
        indexing.run_index(vectors, recipe, identity, version, partitions=8, max_iterations=3)
