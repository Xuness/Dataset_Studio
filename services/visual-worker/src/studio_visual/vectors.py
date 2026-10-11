"""Versioned full-float Lance tables. Only the engine publishes their versions."""

import math
from pathlib import Path
import re

import lancedb
import numpy as np
import pyarrow as pa

from .models import normalized


def table_name(recipe_id):
    if not re.fullmatch(r"[0-9a-f]{64}", recipe_id):
        raise ValueError("invalid recipe id")
    return "feature_" + recipe_id


class Vectors:
    def __init__(self, root):
        self.root = Path(root)
        self.db = lancedb.connect(str(self.root / "vectors"))

    def table(self, recipe, create=False):
        name = table_name(recipe["id"])
        if create:
            schema = pa.schema([
                pa.field("object_id", pa.int64(), nullable=False),
                pa.field("batch_id", pa.string(), nullable=False),
                pa.field("source_ids", pa.list_(pa.int64()), nullable=False),
                pa.field("vector", pa.list_(pa.float32(), recipe["dimension"]), nullable=False),
            ])
            return self.db.create_table(name, schema=schema, exist_ok=True)
        return self.db.open_table(name)

    def existing(self, recipe, ids):
        if not ids:
            return set()
        try:
            table = self.table(recipe)
        except (ValueError, FileNotFoundError):
            return set()
        predicate = "object_id IN (" + ",".join(str(int(i)) for i in ids) + ")"
        return {int(row["object_id"]) for batch in table.search().where(predicate).select(["object_id"]).to_batches()
                for row in batch.to_pylist()}

    def append(self, recipe, batch_id, rows, values):
        table = self.table(recipe, create=True)
        values = np.asarray(values, dtype=np.float32)
        if values.shape != (len(rows), recipe["dimension"]) or not np.isfinite(values).all() \
                or not np.allclose(np.linalg.norm(values, axis=1), 1, rtol=1e-5, atol=1e-6):
            raise ValueError("stored vectors must be finite normalized full float32 values")
        array = pa.FixedSizeListArray.from_arrays(pa.array(values.ravel(), type=pa.float32()), recipe["dimension"])
        data = pa.Table.from_arrays([
            pa.array([r["object_id"] for r in rows], type=pa.int64()),
            pa.array([batch_id] * len(rows)),
            pa.array([r["source_ids"] for r in rows], type=pa.list_(pa.int64())), array,
        ], schema=table.schema)
        table.merge_insert("object_id").when_not_matched_insert_all().execute(data)
        self._maintain(table, recipe)
        return table.version

    def _maintain(self, table, recipe):
        """Bound tiny fragments and unindexed scalar tails; preserve old versions."""
        import json
        from lancedb.index import BTree, LabelList
        state_file = self.root / (table_name(recipe["id"]) + ".maintenance.json")
        try:
            last = json.loads(state_file.read_text())["version"]
        except (OSError, ValueError, KeyError):
            last = 0
        if table.version - last < 32:
            return
        # Use the underlying public Lance operations separately. LanceDB's
        # combined optimize() also prunes versions and is deliberately not used.
        table.to_lance().optimize.compact_files(target_rows_per_fragment=8192, max_rows_per_group=1024)
        table.checkout_latest()
        indexes = {tuple(i.columns) for i in table.list_indices()}
        if ("object_id",) not in indexes:
            table.create_index("object_id", config=BTree())
        if ("source_ids",) not in indexes:
            table.create_index("source_ids", config=LabelList())
        table.to_lance().optimize.optimize_indices()
        table.checkout_latest()
        # This is only a maintenance hint. If it is lost, repeating compaction is
        # harmless; it does not authorize deleting any historical material.
        state_file.write_text(json.dumps({"version": table.version}))

    def project_sources(self, recipe, rows):
        """Refresh associations without rerunning a model or replacing its floats."""
        if not rows:
            return None
        try:
            table = self.table(recipe)
        except (ValueError, FileNotFoundError):
            return None
        ids = ",".join(str(int(r["object_id"])) for r in rows)
        current = {r["object_id"]: r["source_ids"] for batch in table.search().where(f"object_id IN ({ids})")
                   .select(["object_id", "source_ids"]).to_batches() for r in batch.to_pylist()}
        changed = {r["object_id"]: r["source_ids"] for r in rows
                   if r["object_id"] in current and sorted(current[r["object_id"]]) != sorted(r["source_ids"])}
        if changed:
            selected = ",".join(str(int(i)) for i in changed)
            replacements = []
            for batch in table.search().where(f"object_id IN ({selected})").to_batches(batch_size=512):
                for row in batch.to_pylist():
                    row["source_ids"] = changed[row["object_id"]]
                    replacements.append(row)
            # Preserve the original float values and batch provenance. One bounded
            # merge avoids a new fragment/version for every source association.
            table.merge_insert("object_id").when_matched_update_all().execute(pa.Table.from_pylist(replacements, schema=table.schema))
            self._maintain(table, recipe)
        version = table.version
        # Retain every published version until the engine gains explicit reference
        # collection. No cleanup_old_versions/optimize call may remove snapshots.
        tag = f"published_{version}"
        try:
            pinned = table.tags.get_version(tag)
        except RuntimeError as error:
            if "Ref not found" not in str(error):
                raise
            table.tags.create(tag, version)
        else:
            if pinned != version:
                raise ValueError("published version tag points to a different version")
        return version

    def search(self, recipe, version, object_id, vector, source_ids, limit, exact, nprobes):
        table = self.table(recipe)
        table.checkout(int(version))
        if object_id is not None:
            rows = table.search().where(f"object_id = {int(object_id)}").select(["vector"]).limit(1).to_list()
            if not rows:
                raise ValueError("query image has no published vector for this recipe")
            vector = rows[0]["vector"]
        query_vector = normalized(np.asarray([vector], dtype=np.float32), recipe["dimension"])[0]
        indexes = table.list_indices()
        indexed = any("vector" in i.columns for i in indexes)
        query = table.search(query_vector).distance_type("cosine")
        if source_ids:
            query = query.where(" OR ".join(f"array_contains(source_ids, {int(s)})" for s in source_ids), prefilter=True)
        if exact or not indexed:
            query = query.bypass_vector_index()
            method = "exact"
        else:
            query = query.nprobes(int(nprobes))
            method = "ivf_flat"
        rows = query.select(["object_id", "_distance"]).limit(int(limit)).to_list()
        return {"items": [{"object_id": r["object_id"], "distance": r["_distance"]} for r in rows],
                "method": method, "version": int(version), "score_type": "cosine_distance"}

    def index(self, recipe, partitions):
        from lancedb.index import BTree, IvfFlat, LabelList
        table = self.table(recipe)
        rows = table.count_rows()
        if rows < 256:
            raise ValueError("IVF_FLAT requires at least 256 vectors; exact search is available")
        partitions = int(partitions or min(4096, max(1, int(math.sqrt(rows))), max(1, rows // 32)))
        if partitions < 1 or partitions > max(1, rows // 32):
            raise ValueError("partitions must leave at least 32 training rows per partition")
        table.create_index("object_id", config=BTree(), replace=True)
        table.create_index("source_ids", config=LabelList(), replace=True)
        table.create_index("vector", config=IvfFlat(distance_type="cosine", num_partitions=partitions), replace=True)
        version = self.project_sources(recipe, [{"object_id": -1, "source_ids": []}])
        return {"version": version, "rows": rows, "partitions": partitions, "index_type": "IVF_FLAT"}
