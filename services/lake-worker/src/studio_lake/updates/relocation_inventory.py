"""Streaming relocation evidence. Archive payloads are size checked; resumable files are hashed."""

import hashlib
from contextlib import closing
import json
from pathlib import Path
import sqlite3

import apsw

from ..online_schema import settings
from ..util import contained, file_hash, read_json, safe_managed_path
from .sites import UpdateError


def checkpoint(media, index, identity, site):
    pointer, info = read_json(index / "ONLINE.json"), read_json(media / "library.json")
    if pointer.get("schema_version") != 2 or pointer.get("library_id") != identity or pointer.get("site") != site or info.get("library_id") != identity:
        raise UpdateError("SOURCE_ID_MISMATCH", "Relocation lake identity or site differs")
    with closing(sqlite3.connect((media / "journal.sqlite").as_uri() + "?mode=ro", uri=True)) as journal:
        digest = hashlib.sha256()
        head = 0
        for seq, batch, manifest in journal.execute("SELECT seq,batch_id,manifest_json FROM commits ORDER BY seq"):
            head = seq
            digest.update(json.dumps([seq, batch, manifest], separators=(",", ":")).encode())
    source = apsw.Connection(str(contained(index, pointer["file"])), flags=apsw.SQLITE_OPEN_READONLY)
    source.set_busy_timeout(5000)
    try:
        state = settings(source)
        if state.get("library_id") != identity or state.get("site") != site:
            raise UpdateError("SOURCE_ID_MISMATCH", "Online database identity or site differs from its pointer")
        if state["generation"] != pointer["generation"] or int(state["served_seq"]) > head:
            raise UpdateError("SOURCE_CHANGED", "Online projection does not match its archive")
        result = {"library_id": identity, "site": site, "generation": state["generation"],
                  "archive_seq": head, "archive_digest": digest.hexdigest(),
                  "served_seq": int(state["served_seq"]), "min_seq": int(state["min_seq"])}
        retired = index / "PRODUCER-RETIRED.json"
        if retired.exists() or state.get("producer_index_retired") == "1":
            if not retired.is_file():
                raise UpdateError("SOURCE_CHANGED", "Producer retirement marker is missing")
            marker = read_json(retired)
            current = read_json(index / "CURRENT.json")
            if (marker.get("phase") != "complete" or marker.get("library_id") != identity
                    or marker.get("online_generation") != state["generation"]
                    or not current.get("retired") or current.get("index_version") != 0
                    or current.get("library_id") != identity or state.get("producer_index_retired") != "1"):
                raise UpdateError("SOURCE_CHANGED", "Producer retirement is incomplete or belongs to another lake")
            result.update(producer_retired=True, producer_marker_digest=file_hash(retired))
        return result
    finally:
        source.close()


def inventory_paths(media, index):
    # Canonical archive authority and all resumable staging/input files. Analytical
    # caches are rebuildable; online.sqlite is checked by identity/version separately.
    for area, root, names in (
        ("media", media, ("segments", "staging", "plans", "source_manifests")),
        ("index", index, ("updates", "metadata_staging", "pack_staging", "retirement")),
    ):
        for name in names:
            directory = safe_managed_path(root, root / name)
            if not directory.exists():
                continue
            stack = [directory]
            while stack:
                parent = stack.pop()
                for path in parent.iterdir():
                    safe_managed_path(root, path)
                    if path.is_dir():
                        stack.append(path)
                    elif path.is_file():
                        relative = path.relative_to(root).as_posix()
                        # Large immutable archive packs already carry archive hashes.
                        # Do not claim that a move performed a full bit-rot scan.
                        hashed = area == "index" or name != "segments" or path.suffix == ".json"
                        yield area, relative, path.stat().st_size, file_hash(path) if hashed else None
                    else:
                        raise UpdateError("SOURCE_CHANGED", "Relocation contains a non-regular file")
    if (index / "PRODUCER-RETIRED.json").exists():
        for name in ("PRODUCER-RETIRED.json", "CURRENT.json"):
            path = safe_managed_path(index, index / name)
            if not path.is_file():
                raise UpdateError("SOURCE_CHANGED", "Producer retirement evidence is missing")
            yield "index", name, path.stat().st_size, file_hash(path)


def write_inventory(path, media, index):
    with closing(sqlite3.connect(path)) as db, db:
        db.execute("CREATE TABLE IF NOT EXISTS files(area TEXT,path TEXT,bytes INTEGER,sha TEXT,PRIMARY KEY(area,path))")
        db.execute("DELETE FROM files")
        db.executemany("INSERT INTO files VALUES(?,?,?,?)", inventory_paths(media, index))
    with path.open("r+b") as file:
        import os
        os.fsync(file.fileno())


def verify_inventory(path, media, index):
    with closing(sqlite3.connect(path)) as db:
        for area, relative, size, digest in db.execute("SELECT * FROM files ORDER BY area,path"):
            root = media if area == "media" else index
            target = safe_managed_path(root, root / Path(relative))
            if not target.is_file() or target.stat().st_size != size or digest is not None and file_hash(target) != digest:
                raise UpdateError("SOURCE_CHANGED", "Relocation is missing or changed archive, input or staging files")
