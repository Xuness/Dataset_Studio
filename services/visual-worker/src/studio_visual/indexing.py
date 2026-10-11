"""Checkpointed index construction. Engine owns the exclusive writer admission.

The engine runs this in a separate supervised process, so cancellation can stop
a native build while ordinary snapshot readers remain available. Completed
stages and a committed vector index are reconciled before repeating work.
"""
import json
import math
import time
import uuid

from lancedb.index import BTree, LabelList

from .execution import atomic_json
from .metrics import peak_memory


def job_directory(vectors, job_id):
    if str(uuid.UUID(job_id)) != job_id:
        raise ValueError("invalid index job identity")
    directory = vectors.root / "index-jobs" / job_id
    directory.mkdir(parents=True, exist_ok=True)
    return directory


def run_index(vectors, recipe, job_id, input_version, partitions=None,
              max_iterations=30, sample_rate=128):
    directory = job_directory(vectors, job_id)
    table = vectors.table(recipe)
    rows = table.count_rows()
    if rows < 256:
        raise ValueError("IVF_FLAT requires at least 256 vectors")
    partitions = int(partitions or min(4096, max(1, int(math.sqrt(rows))), rows // 32))
    if not 1 <= partitions <= min(8192, rows // 32):
        raise ValueError("each partition requires at least 32 training rows")
    if not 1 <= max_iterations <= 100 or not 32 <= sample_rate <= 256:
        raise ValueError("index training settings exceed bounds")
    training_rate = min(sample_rate, 262144 // partitions)
    specification = {"recipe_id": recipe["id"], "input_version": int(input_version),
                     "rows": rows, "partitions": partitions, "max_iterations": max_iterations,
                     "sample_rate": sample_rate}
    state_path = directory / "state.json"
    if state_path.exists():
        state = json.loads(state_path.read_text(encoding="utf-8"))
        if state["specification"] != specification:
            raise ValueError("index checkpoint belongs to a different input or request")
    else:
        state = {"specification": specification, "completed_stages": []}
        atomic_json(state_path, state)
    result_path = directory / "result.json"
    if result_path.exists():
        result = json.loads(result_path.read_text(encoding="utf-8"))
        table.checkout(result["version"])
        if table.count_rows() != rows:
            raise ValueError("committed index receipt does not match its dataset")
        return result
    input_tag = "index_input_" + job_id
    tags = table.tags.list()
    if input_tag not in tags:
        table.tags.create(input_tag, int(input_version))
    elif tags[input_tag]["version"] != int(input_version):
        raise ValueError("index input tag changed")
    started = time.perf_counter()
    last_progress = [0.0]

    def progress(phase, completed=None, total=None, unit="", force=True):
        now = time.monotonic()
        if not force and now - last_progress[0] < .2:
            return
        atomic_json(directory / "progress.json", {
            "phase": phase, "completed_units": completed, "total_units": total,
            "unit": unit, "updated_at_ms": int(time.time() * 1000),
        })
        last_progress[0] = now

    def checkpoint(stage):
        state["completed_stages"].append(stage)
        state["last_version"] = table.version
        atomic_json(state_path, state)

    if "compact" not in state["completed_stages"]:
        progress("compacting")
        table.to_lance().optimize.compact_files(target_rows_per_fragment=32768, max_rows_per_group=1024)
        table.checkout_latest()
        checkpoint("compact")
    if "scalar" not in state["completed_stages"]:
        progress("scalar_indexes", 0, 2, "indexes")
        table.create_index("object_id", config=BTree(), replace=True)
        progress("scalar_indexes", 1, 2, "indexes")
        table.create_index("source_ids", config=LabelList(), replace=True)
        checkpoint("scalar")
    dataset = table.to_lance()
    # A process can die after Lance's atomic commit but before our checkpoint.
    # The task UUID identifies that exact committed vector index.
    committed = any(str(segment.uuid) == job_id for index in dataset.describe_indices() for segment in index.segments)
    if not committed:
        progress("vector_index")
        def callback(event):
            progress(event.stage, event.completed, event.total, event.unit,
                     force=event.event != "progress")
        dataset.create_index(
            "vector", "IVF_FLAT", name="vector_idx", metric="cosine", replace=True,
            num_partitions=partitions, index_uuid=job_id,
            # Full kmeans on a bounded sample. Lance 9's streaming coreset
            # training collapsed a clustered 1M-vector probe into a 470k-row
            # partition; full training avoids that measured query regression.
            max_iterations=max_iterations, sample_rate=training_rate,
            shuffle_partition_batches=64, shuffle_partition_concurrency=1,
            progress_callback=callback,
        )
        table.checkout_latest()
    stats = table.index_stats("vector_idx")
    if stats is None or stats.num_unindexed_rows != 0 or stats.num_indexed_rows != rows:
        raise ValueError("index did not cover all input vectors")
    details = table.to_lance().stats.index_stats("vector_idx")
    sizes = [p["size"] for index in details["indices"] for p in index["partitions"]]
    version = vectors.project_sources(recipe, [{"object_id": -1, "source_ids": []}])
    result = {"version": version, "rows": rows, "partitions": partitions,
              "index_type": "IVF_FLAT", "indexed_rows": stats.num_indexed_rows,
              "unindexed_rows": stats.num_unindexed_rows,
              "training_sample_rate": training_rate,
              "training_rows_target": min(rows, partitions * training_rate),
              "nonempty_partitions": sum(n > 0 for n in sizes),
              "largest_partition_rows": max(sizes, default=0),
              "build_revision": 2,
              "elapsed_ms": int((time.perf_counter() - started) * 1000),
              "peak_rss_bytes": peak_memory("cpu")["peak_rss_bytes"]}
    atomic_json(result_path, result)
    progress("ready_to_publish", rows, rows, "vectors")
    return result


def release_input(vectors, recipe, job_id):
    job_directory(vectors, job_id)
    table = vectors.table(recipe)
    tag = "index_input_" + job_id
    if tag in table.tags.list():
        table.tags.delete(tag)
    return {"released": True}
