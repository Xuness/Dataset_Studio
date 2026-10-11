import time

import numpy as np
import pytest

from studio_visual.models import normalized
from studio_visual.retention import cleanup
from studio_visual.vectors import Vectors


def test_cleanup_preserves_snapshots_current_and_recent_versions(tmp_path):
    vectors = Vectors(tmp_path)
    recipe = {"id": "b" * 64, "dimension": 256}
    values = normalized(np.ones((1, 256)), 256)
    versions = []
    for n in range(36):
        row = [{"object_id": n + 1, "source_ids": [1]}]
        vectors.append(recipe, str(n), row, values)
        versions.append(vectors.project_sources(recipe, row))
    frozen = versions[2]
    vectors.pin(recipe, frozen, "snapshot_keep")
    time.sleep(.01)
    preview = cleanup(vectors, recipe, versions[-1], [frozen], 3, 0, False)
    before = {v["version"] for v in vectors.table(recipe).list_versions()}
    assert preview["candidate_versions"] > 0 and preview["removed_versions"] == 0
    applied = cleanup(vectors, recipe, versions[-1], [frozen], 3, 0, True)
    assert applied["removed_versions"] > 0
    after = {v["version"] for v in vectors.table(recipe).list_versions()}
    assert {frozen, versions[-1], *sorted(before)[-3:]}.issubset(after)
    assert len(after) < len(before)
    assert vectors.entries(recipe, frozen, [1, 2, 3, 4])["object_ids"] == [1, 2, 3]
    assert vectors.table(recipe).count_rows() == 36
    assert cleanup(vectors, recipe, versions[-1], [frozen], 3, 0, True)["removed_versions"] == 0
    with pytest.raises(ValueError, match="referenced feature version is missing"):
        cleanup(vectors, recipe, versions[-1], [999999], 3, 0, True)
