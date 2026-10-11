"""Bounded, read-only media ranges, shared decode, independent model outcomes."""

import hashlib
from io import BytesIO
import json
import os
from pathlib import Path
import re
import time

import imagehash
# Import native SciPy extensions on the process main thread, before Torch starts
# its pools. Lazy first import inside the GPU executor can deadlock Windows DLL
# initialization (scipy.special._ufuncs); imagehash otherwise imports it lazily.
import scipy.fftpack
from PIL import Image, ImageOps

from .models import Models, ModelLoadError
from .vectors import Vectors
from .metrics import milliseconds, peak_memory

MAX_BYTES = 64 << 20
MAX_PIXELS = 32_000_000
MAX_DECODE_PIXELS = 64_000_000


def atomic_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_suffix(".tmp")
    with temp.open("w", encoding="utf-8") as stream:
        json.dump(value, stream, ensure_ascii=False, separators=(",", ":"))
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temp, path)


def decode(data):
    with Image.open(BytesIO(data)) as image:
        if getattr(image, "n_frames", 1) != 1:
            raise ValueError("unsupported_animation")
        if image.width * image.height > MAX_PIXELS:
            raise ValueError("pixel_limit")
        image.load()
        ImageOps.exif_transpose(image, in_place=True)
        if "A" in image.getbands() or "transparency" in image.info:
            rgba = image if image.mode == "RGBA" else image.convert("RGBA")
            if rgba is not image:
                image.close()
            white = Image.new("RGBA", rgba.size, (255, 255, 255, 255))
            white.alpha_composite(rgba)
            rgb = white.convert("RGB")
            white.close()
            rgba.close()
            return rgb
        return image.convert("RGB")


class Execution:
    def __init__(self, root, device):
        self.root = Path(root).resolve()
        self.models = Models(device)
        self.vectors = Vectors(self.root)
        self.prepared = None
        self.device = device

    def receipt_path(self, batch_id):
        if not re.fullmatch(r"[a-zA-Z0-9-]{1,80}", batch_id):
            raise ValueError("invalid batch identity")
        return self.root / "receipts" / f"{batch_id}.json"

    def prepare(self, request):
        batch_id = request["batch_id"]
        receipt = self.receipt_path(batch_id)
        if receipt.exists():
            return {"complete": json.loads(receipt.read_text(encoding="utf-8")), "bytes_read": 0}
        if self.prepared is not None:
            if self.prepared["request"]["batch_id"] == batch_id:
                return self.prepared["summary"]
            raise ValueError("another batch is already prepared")
        rows = request["items"]
        if not 1 <= len(rows) <= 512 or sum(r["length"] for r in rows) > MAX_BYTES:
            raise ValueError("media batch exceeds admitted range budget")
        recipes = request["recipes"]
        start = time.perf_counter()
        if hasattr(self.models, "timings"):
            self.models.timings = {}
        if self.device == "cuda":
            import torch
            torch.cuda.reset_peak_memory_stats()
        metrics = {"read_ms": 0.0, "sha256_ms": 0.0, "decode_ms": 0.0, "phash_ms": 0.0,
                   "decoded_images": 0, "batches": 1}
        existing = {r["id"]: self.vectors.existing(r, [i["object_id"] for i in rows]) for r in recipes}
        metrics["lookup_ms"] = milliseconds(start)
        results, encoded, accepted = [], {}, []
        bytes_read, opens = 0, 0
        current_path, current_file = None, None
        try:
            for row in rows:
                oid = row["object_id"]
                needs = [r["id"] for r in recipes if oid not in existing[r["id"]]]
                if not needs and row.get("hash_done", False):
                    accepted.append(row)
                    continue
                try:
                    tick = time.perf_counter()
                    relative = Path(row["pack"])
                    if relative.is_absolute() or not relative.parts or any(p in ("..", ".") for p in relative.parts):
                        raise ValueError("range_path_invalid")
                    root = Path(row["media_root"]).resolve(strict=True)
                    path = (root / relative).resolve(strict=True)
                    if not path.is_relative_to(root):
                        raise ValueError("range_outside_lake")
                    offset, length = int(row["offset"]), int(row["length"])
                    if offset < 0 or not 0 < length <= MAX_BYTES:
                        raise ValueError("range_limit")
                    if path != current_path:
                        if current_file:
                            current_file.close()
                        current_file = path.open("rb")
                        current_path = path
                        opens += 1
                    if offset + length > os.fstat(current_file.fileno()).st_size:
                        raise ValueError("range_past_end")
                    current_file.seek(offset)
                    data = current_file.read(length)
                    metrics["read_ms"] += milliseconds(tick)
                    bytes_read += len(data)
                    tick = time.perf_counter()
                    actual_sha = hashlib.sha256(data).hexdigest()
                    metrics["sha256_ms"] += milliseconds(tick)
                    if len(data) != length or actual_sha != row["sha256"]:
                        raise ValueError("sha256_mismatch")
                    encoded[oid] = data
                    accepted.append(row)
                except Exception as error:
                    accepted.append(row)
                    code = str(error)[:200]
                    for recipe_id in ["phash64-dct32-white-v1", *needs]:
                        results.append({"object_id": oid, "recipe_id": recipe_id, "status": "failed", "hash": None, "error": code})
        finally:
            if current_file:
                current_file.close()
        summary = {"complete": None, "bytes_read": bytes_read, "opens": opens,
                   "accepted": len(accepted)}
        self.prepared = {"request": request, "summary": summary, "rows": accepted,
                         "encoded": encoded, "results": results, "existing": existing,
                         "started": start, "metrics": metrics}
        return summary

    def finish(self, batch_id):
        receipt = self.receipt_path(batch_id)
        if receipt.exists():
            return json.loads(receipt.read_text(encoding="utf-8"))
        state = self.prepared
        if state is None or state["request"]["batch_id"] != batch_id:
            raise ValueError("batch is not prepared")
        try:
            rows, results = state["rows"], state["results"]
            recipes = state["request"]["recipes"]
            import numpy as np

            features = {r["id"]: [] for r in recipes}
            feature_rows = {r["id"]: [] for r in recipes}
            model_images = {r["id"]: 0 for r in recipes}
            model_ms = {r["id"]: 0.0 for r in recipes}
            model_batches = {r["id"]: 0 for r in recipes}
            metrics = state["metrics"]
            chunk, pixels = [], 0

            def consume():
                nonlocal chunk, pixels
                try:
                    for recipe in recipes:
                        rid = recipe["id"]
                        selected = [(row, image) for row, image in chunk if row["object_id"] not in state["existing"][rid]]
                        size = int(state["request"].get("micro_batch_sizes", {}).get(recipe.get("model"), 4))
                        if not 1 <= size <= 64:
                            raise ValueError("model micro batch must be between 1 and 64")
                        for start in range(0, len(selected), size):
                            micro = selected[start:start + size]
                            model_images[rid] += len(micro)
                            model_batches[rid] += 1
                            tick = time.perf_counter()
                            try:
                                values = self.models.encode_bounded(recipe, [image for _, image in micro])
                                features[rid].append(values)
                                feature_rows[rid].extend(row for row, _ in micro)
                            except ModelLoadError:
                                raise
                            except Exception as error:
                                for row, _ in micro:
                                    results.append({"object_id": row["object_id"], "recipe_id": rid, "status": "failed",
                                                    "hash": None, "error": str(error)[:200]})
                            finally:
                                model_ms[rid] += milliseconds(tick)
                finally:
                    for _, image in chunk:
                        image.close()
                    chunk, pixels = [], 0

            for row in rows:
                oid = row["object_id"]
                data = state["encoded"].pop(oid, None)
                if data is None:
                    continue
                try:
                    with Image.open(BytesIO(data)) as header:
                        area = header.width * header.height
                    if pixels and pixels + area > MAX_DECODE_PIXELS:
                        consume()
                    tick = time.perf_counter()
                    try:
                        image = decode(data)
                    finally:
                        metrics["decode_ms"] += milliseconds(tick)
                    metrics["decoded_images"] += 1
                    pixels += image.width * image.height
                    chunk.append((row, image))
                    tick = time.perf_counter()
                    hash_value = str(imagehash.phash(image, hash_size=8, highfreq_factor=4))
                    metrics["phash_ms"] += milliseconds(tick)
                    results.append({"object_id": oid, "recipe_id": "phash64-dct32-white-v1", "status": "success",
                                    "hash": hash_value, "error": None})
                except Exception as error:
                    needs = [r["id"] for r in recipes if oid not in state["existing"][r["id"]]]
                    for rid in ["phash64-dct32-white-v1", *needs]:
                        results.append({"object_id": oid, "recipe_id": rid, "status": "failed", "hash": None, "error": str(error)[:200]})
            consume()
            write_started = time.perf_counter()
            versions = {}
            for recipe in recipes:
                rid = recipe["id"]
                existing = state["existing"][rid]
                selected = feature_rows[rid]
                if selected:
                    # Storage and maintenance errors are batch failures, never
                    # per-image/model failures. Preserve the intent and recover
                    # already committed heads before attempting any new batch.
                    values = np.concatenate(features[rid], axis=0)
                    self.vectors.append(recipe, batch_id, selected, values)
                    existing.update(r["object_id"] for r in selected)
                for row in rows:
                    if row["object_id"] in existing:
                        results.append({"object_id": row["object_id"], "recipe_id": rid,
                                        "status": "success", "hash": None, "error": None})
                version = self.vectors.project_sources(recipe, rows)
                if version is not None:
                    versions[rid] = version
            selected_ids = {r["id"] for r in state["request"]["recipes"]}
            for recipe in state["request"].get("projection_recipes", []):
                if recipe["id"] not in selected_ids:
                    version = self.vectors.project_sources(recipe, rows)
                    if version is not None:
                        versions[recipe["id"]] = version
            metrics["write_ms"] = milliseconds(write_started)
            metrics["elapsed_ms"] = milliseconds(state["started"])
            metrics = {k: int(v) for k, v in metrics.items()}
            metrics.update(model_ms={k: int(v) for k, v in model_ms.items()}, model_images=model_images,
                           model_batches=model_batches, opens=state["summary"]["opens"], **peak_memory(self.device))
            for stage in ("load", "preprocess", "inference"):
                metrics[f"model_{stage}_ms"] = {k: int(v.get(stage, 0)) for k, v in getattr(self.models, "timings", {}).items()}
            response = {"batch_id": batch_id, "object_ids": [r["object_id"] for r in rows], "results": results,
                        "versions": versions, "bytes_read": state["summary"]["bytes_read"],
                        "model_images": model_images, "metrics": metrics,
                        "elapsed_ms": metrics["elapsed_ms"]}
            atomic_json(receipt, response)
            return response
        finally:
            self.prepared = None
