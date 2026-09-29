"""Shallow restart recovery must not read historical payloads or stat sealed batches."""

import hashlib
import json
from pathlib import Path
import tracemalloc

import pytest

from studio_lake.util import IntegrityError


def committed(lib, number, *, padding="", files=None):
    identity = f"{number:032x}"
    directory = lib.root / "segments" / identity
    directory.mkdir()
    manifest = {"library_id": lib.info["library_id"], "batch_id": identity,
                "files": files or {}, "source": {"evidence": padding}}
    with lib.journal() as db, db:
        db.execute("INSERT INTO commits(batch_id,dedupe_key,manifest_json,committed_at) VALUES(?,?,?,'fixture')",
                   (identity, identity, json.dumps(manifest)))
    return directory


def test_shallow_recovery_memory_does_not_scale_with_historical_manifest_bytes(lib):
    # 16 MiB of historical metadata must not be loaded just to identify sealed batches.
    for number in range(64):
        committed(lib, number, padding="x" * (256 * 1024))
    tracemalloc.start()
    try:
        assert lib.recover() == []
        _, peak = tracemalloc.get_traced_memory()
    finally:
        tracemalloc.stop()
    assert peak < 4 * 1024**2, peak


def test_shallow_recovery_skips_sealed_directory_io_but_retains_incomplete_work(lib, monkeypatch):
    sealed = committed(lib, 1)
    pending = lib.root / "staging" / ("b" * 32)
    pending.mkdir()
    is_dir = Path.is_dir

    def guarded(path):
        if path == sealed:
            raise AssertionError("shallow recovery performed per-batch historical I/O")
        return is_dir(path)

    monkeypatch.setattr(Path, "is_dir", guarded)
    assert lib.recover() == [{"batch": pending.name, "status": "incomplete-retained", "path": str(pending)}]
    assert pending.exists()


def test_explicit_deep_recovery_still_checks_committed_file_hashes(lib):
    payload = b"sealed bytes"
    directory = committed(lib, 1, files={"evidence.bin": {
        "bytes": len(payload), "sha256": hashlib.sha256(payload).hexdigest(),
    }})
    (directory / "evidence.bin").write_bytes(payload)
    assert lib.recover(deep=True) == []
    (directory / "evidence.bin").write_bytes(b"broken bytes")
    assert lib.recover() == []
    with pytest.raises(IntegrityError, match="内容校验失败"):
        lib.recover(deep=True)
