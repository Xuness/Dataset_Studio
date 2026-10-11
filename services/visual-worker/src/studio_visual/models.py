"""Fixed local model recipes. Inference never downloads models or remote code."""

import gc
import hashlib
import importlib.metadata
import json
import time
from pathlib import Path

import numpy as np

KALOSCOPE_CODE_REVISION = "8315d80ad58347ea119ffe7e415af2e6a42a6463"
DECODE_RECIPE = "pillow-exif-rgb-white-static-v1"
MODELS = {
    "siglip2_naflex": ("siglip2-so400m-patch16-naflex", 1152, "model.safetensors"),
    "embeddinggemma2": ("embeddinggemma-2", 768, "model.safetensors"),
    "kaloscope3": ("Kaloscope3.0-preview", 256, "v1-artist-classifier/model.safetensors"),
}


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def normalized(value, dimension):
    value = np.asarray(value, dtype=np.float32)
    if value.ndim != 2 or value.shape[1] != dimension or not np.isfinite(value).all():
        raise ValueError(f"invalid {dimension}D model output")
    norms = np.linalg.norm(value, axis=1, keepdims=True)
    if not np.isfinite(norms).all() or (norms <= 1e-12).any():
        raise ValueError("zero or non-finite feature norm")
    return np.ascontiguousarray(value / norms, dtype=np.float32)


def describe(model_root, device, cache_root):
    import torch

    if device not in ("cuda", "cpu"):
        raise ValueError("device must be cuda or cpu")
    if device == "cuda" and not torch.cuda.is_available():
        raise ValueError("CUDA requested but unavailable")
    root = Path(model_root).resolve(strict=True)
    if (root / "Embedding Model").is_dir():
        root /= "Embedding Model"
    cache_file = Path(cache_root) / "model-digests.json"
    try:
        cache = json.loads(cache_file.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        cache = {}
    packages = {name: importlib.metadata.version(name) for name in (
        "torch", "torchvision", "transformers", "Pillow", "ImageHash", "safetensors",
    )}
    recipes = []
    for model, (folder, dimension, weights) in MODELS.items():
        directory = root / folder
        path = directory / weights
        stat = path.stat()
        if stat.st_size < 1024 * 1024:
            raise ValueError(f"weights are missing or still a Git LFS pointer: {path}")
        files = [path]
        files += sorted(p for p in directory.glob("*.json") if p.is_file())
        if model == "kaloscope3":
            files += [path.parent / "config.json"]
        hashes = {}
        for file in files:
            stat = file.stat()
            key = str(file)
            stamp = [stat.st_size, stat.st_mtime_ns, stat.st_ctime_ns]
            entry = cache.get(key, {})
            if entry.get("stamp") != stamp:
                entry = {"stamp": stamp, "sha256": digest(file)}
                cache[key] = entry
            hashes[file.relative_to(directory).as_posix()] = entry["sha256"]
        spec = {
            "model": model, "dimension": dimension, "files": hashes,
            "adapter_version": 1, "decode": DECODE_RECIPE,
            "execution_backend": device,
            "forward_dtype": "bfloat16" if device == "cuda" else "float32",
            "storage_dtype": "float32", "normalization": "l2", "packages": packages,
            "input": {"siglip2_naflex": {"max_num_patches": 784},
                      "embeddinggemma2": {"soft_tokens": 280, "pooling": "masked_mean", "audio": False},
                      "kaloscope3": {"short_edge": 512, "center_crop": 512,
                                     "pooling": "cls_mean", "output": "projector",
                                     "architecture_revision": KALOSCOPE_CODE_REVISION}}[model],
        }
        recipe_id = hashlib.sha256(json.dumps(spec, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        recipes.append({"id": recipe_id, "model": model, "dimension": dimension,
                        "spec": spec, "path": str(directory), "weights": str(path)})
    cache_file.parent.mkdir(parents=True, exist_ok=True)
    cache_file.write_text(json.dumps(cache, indent=2), encoding="utf-8")
    return {"recipes": recipes, "device": device,
            "device_name": torch.cuda.get_device_name(0) if device == "cuda" else "CPU",
            "packages": packages, "hash_recipe": "phash64-dct32-white-v1"}


class Models:
    def __init__(self, device):
        import torch

        torch.set_num_threads(min(8, torch.get_num_threads()))
        self.device = device
        self.loaded = {}
        self.timings = {}

    def timed(self, recipe, stage, started):
        row = self.timings.setdefault(recipe["id"], {})
        row[stage] = row.get(stage, 0.0) + (time.perf_counter() - started) * 1000

    def _load(self, recipe):
        import torch
        from transformers import AutoImageProcessor, AutoModel, AutoProcessor, Siglip2VisionModel

        kind, path = recipe["model"], recipe["path"]
        if recipe["spec"]["execution_backend"] != self.device:
            raise ValueError("recipe belongs to another execution backend")
        for name, version in recipe["spec"]["packages"].items():
            if importlib.metadata.version(name) != version:
                raise ValueError("recipe belongs to another runtime version; register a new recipe")
        for name, expected in recipe["spec"]["files"].items():
            if digest(Path(path) / name) != expected:
                raise ValueError("local model files changed; register a new recipe before inference")
        dtype = torch.bfloat16 if self.device == "cuda" else torch.float32
        if kind == "siglip2_naflex":
            processor = AutoImageProcessor.from_pretrained(path, local_files_only=True)
            model = Siglip2VisionModel.from_pretrained(path, local_files_only=True, dtype=dtype)
        elif kind == "embeddinggemma2":
            processor = AutoProcessor.from_pretrained(path, local_files_only=True)
            model = AutoModel.from_pretrained(path, local_files_only=True, dtype=dtype, audio_config=None)
        elif kind == "kaloscope3":
            from .kaloscope import Kaloscope

            processor = None
            model = Kaloscope(recipe["weights"])
            # DINO rotary positions and image normalization retain their native
            # float32 buffers; use autocast for its forward, as in training.
        else:
            raise ValueError("unknown visual model")
        return model.eval().to(self.device), processor

    def encode(self, recipe, images):
        import torch
        from torchvision import transforms

        key = recipe["id"]
        if key not in self.loaded:
            started = time.perf_counter()
            try:
                self.loaded[key] = self._load(recipe)
            except torch.cuda.OutOfMemoryError:
                self.loaded.clear()
                gc.collect()
                torch.cuda.empty_cache()
                self.loaded[key] = self._load(recipe)
            finally:
                self.timed(recipe, "load", started)
        model, processor = self.loaded[key]
        kind = recipe["model"]
        with torch.inference_mode():
            started = time.perf_counter()
            if kind == "siglip2_naflex":
                inputs = processor(images=images, max_num_patches=784, return_tensors="pt")
                inputs = inputs.to(self.device, dtype=model.dtype)
                if self.device == "cuda": torch.cuda.synchronize()
                self.timed(recipe, "preprocess", started)
                started = time.perf_counter()
                output = model(**inputs).pooler_output
            elif kind == "embeddinggemma2":
                inputs = processor(images=[[image] for image in images], return_tensors="pt")
                inputs = inputs.to(self.device, dtype=model.dtype)
                if self.device == "cuda": torch.cuda.synchronize()
                self.timed(recipe, "preprocess", started)
                started = time.perf_counter()
                tokens = model(**inputs).last_hidden_state
                mask = inputs["attention_mask"].unsqueeze(-1).float()
                output = (tokens.float() * mask).sum(1) / mask.sum(1).clamp(min=1)
            else:
                spatial = transforms.Compose([transforms.Resize(512), transforms.CenterCrop(512)])
                tensors = [torch.from_numpy(np.array(spatial(i), copy=True)).permute(2, 0, 1) for i in images]
                inputs = torch.stack(tensors).to(self.device)
                if self.device == "cuda": torch.cuda.synchronize()
                self.timed(recipe, "preprocess", started)
                started = time.perf_counter()
                with torch.autocast(device_type="cuda", dtype=torch.bfloat16, enabled=self.device == "cuda"):
                    output = model(inputs)
        value = normalized(output.float().cpu().numpy(), recipe["dimension"])
        self.timed(recipe, "inference", started)
        return value

    def encode_bounded(self, recipe, images):
        """Split CUDA OOM batches without changing preprocessing or vector semantics."""
        import torch

        try:
            return self.encode(recipe, images)
        except torch.cuda.OutOfMemoryError:
            if len(images) <= 1:
                raise
            torch.cuda.empty_cache()
            half = len(images) // 2
            return np.concatenate((self.encode_bounded(recipe, images[:half]),
                                   self.encode_bounded(recipe, images[half:])), axis=0)
