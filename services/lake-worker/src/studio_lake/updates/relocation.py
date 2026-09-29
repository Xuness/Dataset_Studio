"""Prepare -> verify -> writer switched -> reader acknowledged. Every step is repeatable."""

from contextlib import ExitStack
import json
from pathlib import Path
import uuid

from ..online_migrate import connect
from ..util import FileLock, atomic_json, contained, failpoint, now, read_json, safe_managed_path, same_directory
from . import locations
from .relocation_inventory import checkpoint, write_inventory, verify_inventory
from .sites import UpdateError


def row(state, identity):
    with state.db() as db:
        value = db.execute("SELECT * FROM lake_relocations WHERE id=?", (identity,)).fetchone()
    if value is None:
        raise UpdateError("NOT_FOUND", "Lake relocation not found")
    return dict(value)


def list_all(state):
    with state.db() as db:
        rows = db.execute("SELECT * FROM lake_relocations WHERE phase NOT IN ('complete','cancelled') ORDER BY created_at LIMIT 1000")
        return {"items": [dict(value) for value in rows]}


def evidence_path(state, identity):
    if not isinstance(identity, str) or len(identity) != 32 or any(c not in "0123456789abcdef" for c in identity):
        raise UpdateError("INVALID_INPUT", "Invalid relocation identity")
    return safe_managed_path(state.root, state.root / "relocations" / (identity + ".sqlite"))


def locks(stack, media, index):
    for root, name in ((index, ".daily-run.lock"), (media, ".writer.lock"),
                       (index, ".index.lock"), (index, ".online.lock")):
        stack.enter_context(FileLock(root / name, timeout=0))


def marker(index, value):
    path = index / "LAKE-RELOCATION.json"
    if path.exists() and read_json(path).get("id") != value["id"]:
        raise UpdateError("UPDATE_CONFLICT", "Another relocation owns this location")
    atomic_json(path, {"id": value["id"], "library_id": value["lake_id"]})


def prepare(state, lake_id):
    with FileLock(locations.directory(state, lake_id) / "gate.lock", timeout=10), state.db() as db:
        lake = state.lake(lake_id)
        db.execute("BEGIN IMMEDIATE")
        old = db.execute("SELECT id FROM lake_relocations WHERE lake_id=? AND phase NOT IN ('complete','cancelled')", (lake_id,)).fetchone()
        identity = old[0] if old else uuid.uuid4().hex
        if old is None:
            db.execute("INSERT INTO lake_relocations(id,lake_id,phase,old_media,old_index,created_at) VALUES(?,?,'draining',?,?,?)",
                       (identity, lake_id, lake["media"], lake["index_root"], now()))
    value = row(state, identity)
    if value["phase"] != "draining":
        return value
    try:
        with locations.drained(state, lake_id), ExitStack() as stack:
            value = row(state, identity)
            if value["phase"] != "draining":
                return value
            media, index = Path(value["old_media"]), Path(value["old_index"])
            # Offline roots without prior evidence cannot establish a freshness floor.
            if not (media / "journal.sqlite").is_file() or not (index / "ONLINE.json").is_file():
                raise UpdateError("SOURCE_CHANGED", "Restore the old lake to prepare a verified relocation checkpoint")
            lib = state.library(lake_id)
            stack.enter_context(FileLock(index / ".daily-run.lock", timeout=0))
            if not (index / "LAKE-RELOCATION.json").exists():
                lib.recover()
            for root, name in ((media, ".writer.lock"), (index, ".index.lock"), (index, ".online.lock")):
                stack.enter_context(FileLock(root / name, timeout=0))
            stamp = checkpoint(media, index, lake_id, lake["site"])
            path = evidence_path(state, identity)
            path.parent.mkdir(parents=True, exist_ok=True)
            write_inventory(path, media, index)
            marker(index, value)
            failpoint("relocation_prepared_files")
            with state.db() as db:
                db.execute("UPDATE lake_relocations SET phase='prepared',checkpoint=? WHERE id=? AND phase='draining'",
                           (json.dumps(stamp), identity))
            failpoint("relocation_prepared")
    except UpdateError as error:
        if error.code == "UPDATE_CONFLICT":
            return row(state, identity)
        raise
    except RuntimeError as error:
        if str(error).startswith("另一个进程正在使用此工作区"):
            return row(state, identity)
        raise
    return row(state, identity)


def apply(state, identity, media_root, index_root):
    value = row(state, identity)
    media, index = Path(media_root), Path(index_root)
    if not media.is_absolute() or not index.is_absolute() or not media.is_dir() or not index.is_dir():
        raise UpdateError("INVALID_INPUT", "Relocation requires existing absolute directories")
    # Reject aliases, links and nested destinations that could merge two owners.
    for root in (media, index):
        safe_managed_path(root.parent, root)
    media, index = media.resolve(), index.resolve()
    for target in (media, index):
        for original in (Path(value["old_media"]), Path(value["old_index"])):
            if target != original and (original in target.parents or target in original.parents):
                raise UpdateError("INVALID_INPUT", "Relocation roots cannot contain old roots")
    if value["phase"] in {"writer_committed", "complete"}:
        if (value["media_root"], value["index_root"]) != (str(media), str(index)):
            raise UpdateError("UPDATE_CONFLICT", "Relocation already selected other roots")
        return value
    if value["phase"] not in {"prepared", "verified"}:
        raise UpdateError("UPDATE_CONFLICT", "Wait for lake operations to drain before moving files")
    if value["phase"] == "verified" and (value["media_root"], value["index_root"]) != (str(media), str(index)):
        raise UpdateError("UPDATE_CONFLICT", "Resume the selected relocation before changing its target")
    with locations.drained(state, value["lake_id"]), ExitStack() as stack:
        value = row(state, identity)
        if value["phase"] in {"writer_committed", "complete"}:
            if (value["media_root"], value["index_root"]) != (str(media), str(index)):
                raise UpdateError("UPDATE_CONFLICT", "Relocation already selected other roots")
            return value
        if value["phase"] not in {"prepared", "verified"} or value["phase"] == "verified" and (value["media_root"], value["index_root"]) != (str(media), str(index)):
            raise UpdateError("UPDATE_CONFLICT", "Relocation changed; refresh before continuing")
        locks(stack, media, index)
        expected = json.loads(value["checkpoint"])
        actual = checkpoint(media, index, value["lake_id"], expected["site"])
        if any(actual[key] != expected[key] for key in ("generation", "archive_seq", "archive_digest")) or actual["served_seq"] < expected["served_seq"] or actual["min_seq"] > expected["min_seq"]:
            raise UpdateError("SOURCE_CHANGED", "Relocation target is stale or no longer retains the frozen versions")
        for name in ("UPDATE-CONTROLLER.json", "cache_owner.json"):
            owner = read_json(index / name)
            if owner.get("library_id") != value["lake_id"] or (name == "UPDATE-CONTROLLER.json" and not same_directory(owner.get("root"), state.root)):
                raise UpdateError("UPDATE_CONFLICT", "Relocation target belongs to another controller or lake")
        verify_inventory(evidence_path(state, identity), media, index)
        existing_marker = index / "LAKE-RELOCATION.json"
        if existing_marker.exists() and read_json(existing_marker).get("id") != identity:
            raise UpdateError("UPDATE_CONFLICT", "Another relocation owns this location")
        with state.db() as db:
            db.execute("UPDATE lake_relocations SET phase='verified',media_root=?,index_root=? WHERE id=?",
                       (str(media), str(index), identity))
        marker(index, value)
        failpoint("relocation_verified")
        # Preserve all versions admitted before/during the move, including offline
        # old roots. This persistent floor requires an explicit retention decision.
        pointer = read_json(index / "ONLINE.json")
        source = connect(contained(index, pointer["file"]))
        try:
            with source:
                source.execute("INSERT OR REPLACE INTO leases VALUES(?,?,NULL,?,'relocation')",
                               ("relocation:" + identity, expected["min_seq"], identity))
        finally:
            source.close()
        atomic_json(media / "online-index.json", {"schema_version": 2, "library_id": value["lake_id"], "index_root": str(index)})
        failpoint("relocation_publisher")
        atomic_json(index / "cache_owner.json", {"library_id": value["lake_id"], "root": str(media)})
        atomic_json(index / "UPDATE-CONTROLLER.json", {"library_id": value["lake_id"], "root": str(state.root)})
        failpoint("relocation_owner")
        with state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            db.execute("UPDATE lakes SET media=?,index_root=? WHERE id=?", (str(media), str(index), value["lake_id"]))
            db.execute("UPDATE lake_relocations SET phase='writer_committed' WHERE id=?", (identity,))
        failpoint("relocation_writer")
    return row(state, identity)


def finish(state, identity):
    value = row(state, identity)
    if value["phase"] == "complete":
        return value
    if value["phase"] != "writer_committed":
        raise UpdateError("UPDATE_CONFLICT", "Reader location must acknowledge the switched writer first")
    with locations.drained(state, value["lake_id"]):
        value = row(state, identity)
        if value["phase"] == "complete":
            return value
        if value["phase"] != "writer_committed":
            raise UpdateError("UPDATE_CONFLICT", "Relocation changed; refresh before continuing")
        path = Path(value["index_root"]) / "LAKE-RELOCATION.json"
        if path.exists():
            if read_json(path).get("id") != identity:
                raise UpdateError("UPDATE_CONFLICT", "Relocation marker changed")
            path.unlink()
        failpoint("relocation_unfrozen")
        with state.db() as db:
            db.execute("UPDATE lake_relocations SET phase='complete' WHERE id=?", (identity,))
    return row(state, identity)


def cancel(state, identity):
    value = row(state, identity)
    if value["phase"] == "cancelled":
        return value
    if value["phase"] not in {"draining", "prepared"}:
        raise UpdateError("UPDATE_CONFLICT", "A switched relocation must be completed, not cancelled")
    with locations.drained(state, value["lake_id"]):
        value = row(state, identity)
        if value["phase"] == "cancelled":
            return value
        if value["phase"] not in {"draining", "prepared"}:
            raise UpdateError("UPDATE_CONFLICT", "A switched relocation must be completed, not cancelled")
        if value["checkpoint"]:
            expected = json.loads(value["checkpoint"])
            actual = checkpoint(Path(value["old_media"]), Path(value["old_index"]), value["lake_id"], expected["site"])
            if actual != expected:
                raise UpdateError("SOURCE_CHANGED", "Restore the frozen old roots before cancelling")
        path = Path(value["old_index"]) / "LAKE-RELOCATION.json"
        if path.exists():
            if read_json(path).get("id") != identity:
                raise UpdateError("UPDATE_CONFLICT", "Relocation marker changed")
            path.unlink()
        with state.db() as db:
            db.execute("UPDATE lake_relocations SET phase='cancelled' WHERE id=?", (identity,))
    return row(state, identity)
