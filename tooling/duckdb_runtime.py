"""Shared runtime selection for stdlib-only C API fixtures."""
import os
from pathlib import Path


def library_path():
    configured = os.environ.get("STUDIO_DUCKDB_LIBRARY") or os.environ.get("STUDIO_DUCKDB_DLL")
    return Path(configured) if configured else (
        Path(__file__).resolve().parents[1] / "vendor" / "duckdb" /
        ("duckdb.dll" if os.name == "nt" else "libduckdb.so")
    )
