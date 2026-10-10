"""Resumable byte-preserving consolidation, independent of logical commit size.

Containers are uncompressed USTAR files. They contain the original Parquet,
receipt, manifest and media TAR bytes, without reserializing any accepted file.
The journal publishes immutable virtual-file ranges in one transaction. Logical
versions and object offsets stay unchanged, including retained historical views.
"""

import argparse
from contextlib import nullcontext
import hashlib
import json
import os
from pathlib import Path
import shutil
import tarfile
import uuid

from . import CONTAINER_FEATURE, schema
from .library import PinterestLibrary, check_info, verify_manifest
from .online import Publisher
from .storage import COPY_BYTES, hash_stream
from ...canonical import canonical, utc
from ...config import Config
from ...online_storage import connect
from ...util import FileLock, IntegrityError, atomic_json, contained, digest, failpoint, file_hash, read_json, safe_managed_path, sync_directory, sync_file

TARGET_BYTES = 4 * 1024**3
TRIGGER_FILES = 4096
MAX_BATCHES = 1024
RESERVE_BYTES = 128 * 1024**2


def _check_stop(stop):
    if stop and stop():
        raise InterruptedError("Pinterest archive maintenance interrupted")


def _header(row):
    member = tarfile.TarInfo(row["path"])
    member.size, member.mode, member.mtime = row["length"], 0o644, 0
    return member.tobuf(format=tarfile.USTAR_FORMAT, encoding="ascii")


def _check_directory(directory, names):
    if directory.exists() and any(path.name not in names or not path.is_file() for path in directory.iterdir()):
        raise IntegrityError("Pinterest batch contains an unmanaged file or directory")


def _owned(lib, relative):
    # Validate the relative spelling, then inspect its lexical path. Passing the
    # resolved result to safe_managed_path would hide an intermediate junction.
    contained(lib.root, relative)
    return safe_managed_path(lib.root, lib.root / relative)


def _members(lib, identity, where="", parameters=()):
    with lib.journal() as db:
        for row in db.execute("SELECT * FROM archive_members WHERE container_id=? " + where + " ORDER BY offset", (identity, *parameters)):
            yield dict(row)


def _current(lib):
    with lib.journal() as db:
        if not db.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='archive_containers'").fetchone():
            return None
        row = db.execute("SELECT * FROM archive_containers WHERE state IN ('building','sealed','active') ORDER BY first_seq LIMIT 1").fetchone()
    return dict(row) if row else None


def _candidates(lib, *, target_bytes, trigger_files, force):
    publisher = Publisher(lib)
    db = connect(publisher.path)
    try:
        through = publisher._check(db)
    finally:
        db.close()
    cursor = lib.setting("pinterest_container_through", 0)
    selected, total, files, full = [], 1024, 0, False
    with lib.journal() as db:
        for commit in db.execute("SELECT seq,batch_id,manifest_json FROM commits WHERE seq>? AND seq<=? ORDER BY seq LIMIT ?", (cursor, through, MAX_BATCHES)):
            if commit["seq"] != cursor + len(selected) + 1:
                raise IntegrityError("Pinterest compaction requires a continuous archive prefix")
            manifest = json.loads(commit["manifest_json"])
            # Manifest bytes are bounded separately when freezing the plan.
            estimate = sum(512 + ((v["bytes"] + 511) // 512) * 512 for v in manifest["files"].values()) + 512 + len(commit["manifest_json"].encode()) + 4096
            if selected and total + estimate > target_bytes:
                full = True
                break
            selected.append(dict(commit))
            total += estimate
            files += len(manifest["files"]) + 1
            if total >= target_bytes or files >= trigger_files:
                break
    ready = force or full or total >= target_bytes or files >= trigger_files or len(selected) == MAX_BATCHES
    return selected if ready else []


def _plan(lib, commits, stop):
    identity = str(uuid.uuid4())
    rows, position = [], 0
    for commit in commits:
        _check_stop(stop)
        manifest = json.loads(commit["manifest_json"])
        directory = _owned(lib, "segments/" + commit["batch_id"])
        _check_directory(directory, {*manifest["files"], "manifest.json"})
        verify_manifest(lib, directory, manifest, hash_media=False)
        files = {**manifest["files"], "manifest.json": dict(bytes=(directory / "manifest.json").stat().st_size,
                                                           sha256=file_hash(directory / "manifest.json"))}
        for name, info in sorted(files.items()):
            relative = "segments/" + commit["batch_id"] + "/" + name
            safe_managed_path(lib.root, lib.root / relative)
            rows.append(dict(path=relative, container_id=identity, seq=commit["seq"], offset=position + 512,
                             length=info["bytes"], sha256=info["sha256"]))
            position += 512 + ((info["bytes"] + 511) // 512) * 512
    size = position + 1024
    if shutil.disk_usage(lib.root).free < size + RESERVE_BYTES:
        return dict(state="waiting_space", required_bytes=size + RESERVE_BYTES)
    with lib.journal() as db, db:
        db.executescript(schema.sql("compaction"))
    # Older readers reject the new physical capability before any mapping is activated.
    info = read_json(lib.root / "library.json")
    check_info(info)
    if CONTAINER_FEATURE not in info["required_features"]:
        info["required_features"].append(CONTAINER_FEATURE)
        atomic_json(lib.root / "library.json", info)
    lib.info = info
    container = dict(container_id=identity, path=f"containers/{identity[:2]}/{identity}.tar", state="building",
        first_seq=commits[0]["seq"], last_seq=commits[-1]["seq"], bytes=size, created_at=utc())
    with lib.journal() as db, db:
        db.execute("INSERT INTO archive_containers(container_id,path,state,first_seq,last_seq,bytes,created_at) VALUES(:container_id,:path,:state,:first_seq,:last_seq,:bytes,:created_at)", container)
        db.executemany("INSERT INTO archive_members(path,container_id,seq,offset,length,sha256) VALUES(:path,:container_id,:seq,:offset,:length,:sha256)", rows)
        db.execute("INSERT INTO settings VALUES('pinterest_container_through',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", (str(container["last_seq"]),))
    failpoint("pinterest_compact_after_plan")
    return container


def _build(lib, container, stop):
    final = _owned(lib, container["path"])
    partial = safe_managed_path(lib.root, final.with_suffix(".part"))
    if not final.exists():
        final.parent.mkdir(parents=True, exist_ok=True)
        allocated = partial.stat().st_size if partial.exists() else 0
        if shutil.disk_usage(lib.root).free < max(0, container["bytes"] - allocated) + RESERVE_BYTES:
            return False
        with partial.open("r+b" if partial.exists() else "x+b") as output:
            copied = []
            for row in _members(lib, container["container_id"], "AND copied=0"):
                _check_stop(stop)
                source_path = _owned(lib, row["path"])
                output.seek(row["offset"] - 512)
                output.write(_header(row))
                sha, remaining = hashlib.sha256(), row["length"]
                with source_path.open("rb") as source:
                    if source.seek(0, 2) != remaining:
                        raise IntegrityError("Pinterest compaction source length changed")
                    source.seek(0)
                    while remaining:
                        _check_stop(stop)
                        chunk = source.read(min(COPY_BYTES, remaining))
                        if not chunk:
                            raise IntegrityError("Pinterest compaction source is truncated")
                        output.write(chunk)
                        sha.update(chunk)
                        remaining -= len(chunk)
                        failpoint("pinterest_compact_during_copy")
                if sha.hexdigest() != row["sha256"]:
                    raise IntegrityError("Pinterest compaction source hash changed")
                output.write(b"\0" * (-row["length"] % 512))
                copied.append(row["path"])
                if len(copied) == 64:
                    output.flush()
                    os.fsync(output.fileno())
                    with lib.journal() as db, db:
                        db.executemany("UPDATE archive_members SET copied=1 WHERE path=?", ((p,) for p in copied))
                    copied.clear()
            output.seek(container["bytes"] - 1024)
            output.write(b"\0" * 1024)
            output.truncate()
            output.flush()
            os.fsync(output.fileno())
        partial.rename(final)
        sync_directory(final.parent)
        failpoint("pinterest_compact_after_rename")
    sha = _verify(lib, container, stop)
    with lib.journal() as db, db:
        db.execute("UPDATE archive_members SET copied=1 WHERE container_id=?", (container["container_id"],))
        db.execute("UPDATE archive_containers SET state='sealed',sha256=?,verified_at=? WHERE container_id=?", (sha, utc(), container["container_id"]))
    container.update(state="sealed", sha256=sha)
    failpoint("pinterest_compact_after_verify")
    return True


def _verify(lib, container, stop):
    whole, position = hashlib.sha256(), 0
    with _owned(lib, container["path"]).open("rb") as source:
        if source.seek(0, 2) != container["bytes"]:
            raise IntegrityError("Pinterest container size differs from its plan")
        source.seek(0)
        for row in _members(lib, container["container_id"]):
            _check_stop(stop)
            header = source.read(512)
            if row["offset"] != position + 512 or header != _header(row):
                raise IntegrityError("Pinterest container header or range differs")
            whole.update(header)
            sha, remaining = hashlib.sha256(), row["length"]
            while remaining:
                _check_stop(stop)
                chunk = source.read(min(COPY_BYTES, remaining))
                if not chunk:
                    raise IntegrityError("Pinterest container member is truncated")
                sha.update(chunk)
                whole.update(chunk)
                remaining -= len(chunk)
            if sha.hexdigest() != row["sha256"]:
                raise IntegrityError("Pinterest container member hash differs")
            padding = source.read(-row["length"] % 512)
            if any(padding):
                raise IntegrityError("Pinterest container padding differs")
            whole.update(padding)
            position = row["offset"] + row["length"] + len(padding)
        trailer = source.read(1025)
        if trailer != b"\0" * 1024 or position + 1024 != container["bytes"]:
            raise IntegrityError("Pinterest container trailer differs")
        whole.update(trailer)
    result = whole.hexdigest()
    if container.get("sha256") and result != container["sha256"]:
        raise IntegrityError("Pinterest verified container was changed")
    return result


def verify_containers(archive):
    """Deep archive verification also covers outer headers and original manifests."""
    with archive.journal() as db:
        if not db.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='archive_containers'").fetchone():
            return
        for row in db.execute("SELECT * FROM archive_containers WHERE state IN ('active','cleaned') ORDER BY first_seq"):
            _verify(archive, dict(row), None)


def _references(lib, container):
    """All versions keep their original paths; validate each path and object range."""
    if not 1 <= container["last_seq"] - container["first_seq"] + 1 <= MAX_BATCHES:
        raise IntegrityError("Invalid Pinterest compaction sequence range")
    with lib.journal() as journal:
        commits = list(journal.execute("SELECT seq,batch_id,manifest_json FROM commits WHERE seq BETWEEN ? AND ? ORDER BY seq",
                                      (container["first_seq"], container["last_seq"])))
        if len(commits) != container["last_seq"] - container["first_seq"] + 1 or len(commits) > MAX_BATCHES:
            raise IntegrityError("Pinterest compaction logical prefix changed")
        media, publications, count = {}, {}, 0
        for commit in commits:
            manifest = json.loads(commit["manifest_json"])
            rows = list(journal.execute("SELECT path,offset,length,sha256 FROM archive_members WHERE seq=? AND container_id=?", (commit["seq"], container["container_id"])))
            prefix = "segments/" + commit["batch_id"] + "/"
            expected = {prefix + name for name in manifest["files"]} | {prefix + "manifest.json"}
            if {r["path"] for r in rows} != expected:
                raise IntegrityError("Pinterest compaction omitted an accepted file")
            for row in rows:
                name = row["path"][len(prefix):]
                if name != "manifest.json" and (row["length"], row["sha256"]) != (manifest["files"][name]["bytes"], manifest["files"][name]["sha256"]):
                    raise IntegrityError("Pinterest compaction changed accepted evidence")
                if name == "media.tar":
                    media[row["path"]] = row["length"]
            publications[commit["seq"]] = digest(canonical(manifest).encode())
            count += len(rows)
    publisher = Publisher(lib)
    db = connect(publisher.path)
    try:
        through = publisher._check(db)
        if through < container["last_seq"]:
            raise IntegrityError("Publish Pinterest receipts before reclaiming their files")
        objects, batches = 0, 0
        for pack, offset, size in db.execute("SELECT pack_path,offset,length FROM objects WHERE first_seq BETWEEN ? AND ?", (container["first_seq"], container["last_seq"])):
            if offset < 0 or size <= 0 or pack not in media or offset + size > media[pack]:
                raise IntegrityError("A Pinterest object reference is not covered by the new container")
            objects += 1
        for seq, sha in db.execute("SELECT seq,manifest_sha256 FROM publications WHERE seq BETWEEN ? AND ?", (container["first_seq"], container["last_seq"])):
            if publications.get(seq) != sha:
                raise IntegrityError("Pinterest fixed publication differs from its accepted manifest")
            batches += 1
        if batches != len(publications):
            raise IntegrityError("Pinterest publication prefix is incomplete")
        leases = db.execute("SELECT count(*) FROM leases WHERE seq<=?", (container["last_seq"],)).fetchone()[0]
    finally:
        db.close()
    return dict(files=count, objects=objects, batches=batches, retained_leases=leases,
                generation=publisher.pointer["generation"], served_seq=through, logical_paths_unchanged=True, checked_at=utc())


def _activate(lib, container, stop):
    # A sealed container may have waited across a restart. Verify before the switch.
    _verify(lib, container, stop)
    proof = _references(lib, container)
    with lib.journal() as db, db:
        db.execute("UPDATE archive_containers SET state='active',activated_at=?,reference_json=? WHERE container_id=? AND state='sealed'",
                   (utc(), canonical(proof), container["container_id"]))
    sync_file(lib.root / "journal.sqlite")
    container["state"] = "active"
    failpoint("pinterest_compact_after_activate")


def _reclaim(lib, container, stop):
    # Never remove a source on the strength of a previous run's verification alone.
    _verify(lib, container, stop)
    proof = _references(lib, container)
    with lib.journal() as db:
        batches = [(r["batch_id"], json.loads(r["manifest_json"])) for r in db.execute(
            "SELECT batch_id,manifest_json FROM commits WHERE seq BETWEEN ? AND ? ORDER BY seq", (container["first_seq"], container["last_seq"]))]
    for batch, manifest in batches:
        _check_directory(safe_managed_path(lib.root, lib.root / "segments" / batch), {*manifest["files"], "manifest.json"})
    retained, removed = 0, 0
    for row in _members(lib, container["container_id"], "AND reclaimed=0"):
        _check_stop(stop)
        path = _owned(lib, row["path"])
        try:
            if path.exists():
                if path.stat().st_size != row["length"]:
                    raise IntegrityError("Pinterest loose file changed before reclamation")
                with path.open("rb") as source:
                    if hash_stream(source, stop=stop) != row["sha256"]:
                        raise IntegrityError("Pinterest loose file hash changed before reclamation")
                path.unlink()
                removed += 1
                failpoint("pinterest_compact_during_reclaim")
        except PermissionError:
            retained += 1
            continue
        with lib.journal() as db, db:
            db.execute("UPDATE archive_members SET reclaimed=1 WHERE path=?", (row["path"],))
    # Also retry directories left empty by a crash after the last file's receipt.
    directories = 0
    for batch, _ in batches:
        path = safe_managed_path(lib.root, lib.root / "segments" / batch)
        if path.exists():
            try:
                path.rmdir()  # Only empty, owned directories; no recursive deletion.
            except OSError:
                directories += 1
    if not retained and not directories:
        with lib.journal() as db, db:
            db.execute("UPDATE archive_containers SET state='cleaned',cleaned_at=?,reference_json=? WHERE container_id=?", (utc(), canonical(proof), container["container_id"]))
        container["state"] = "cleaned"
    failpoint("pinterest_compact_after_reclaim")
    return dict(state=container["state"], container_id=container["container_id"], bytes=container["bytes"],
                removed_files=removed, retained_files=retained, retained_directories=directories, references=proof)


def compact(lib, *, force=False, target_bytes=TARGET_BYTES, trigger_files=TRIGGER_FILES, stop=None):
    """Process at most one container; callers may drain explicitly or retry later."""
    if type(target_bytes) is not int or target_bytes < 1024 or type(trigger_files) is not int or trigger_files < 2:
        raise ValueError("Invalid Pinterest compaction bounds")
    try:
        with lib.writer_lock():
            _check_stop(stop)
            container = _current(lib)
            if container is None:
                commits = _candidates(lib, target_bytes=target_bytes, trigger_files=trigger_files, force=force)
                if not commits:
                    return dict(state="idle")
                container = _plan(lib, commits, stop)
                if container["state"] == "waiting_space":
                    return container
            if container["state"] == "building" and not _build(lib, container, stop):
                return dict(state="waiting_space", container_id=container["container_id"])
            if container["state"] == "sealed":
                _activate(lib, container, stop)
            return _reclaim(lib, container, stop)
    except InterruptedError:
        return dict(state="interrupted")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--media-root", required=True, type=Path)
    parser.add_argument("--index-root", required=True, type=Path)
    parser.add_argument("--force", action="store_true", help="Also consolidate a final partial container")
    parser.add_argument("--drain", action="store_true", help="Process all eligible containers")
    args = parser.parse_args()
    lib = PinterestLibrary(Config(args.media_root, args.index_root))
    access = nullcontext()
    marker = lib.cache / "UPDATE-CONTROLLER.json"
    if marker.exists():
        from ...updates import locations
        from ...updates.state import State
        owner = read_json(marker)
        if owner.get("library_id") != lib.info["library_id"]:
            raise IntegrityError("Pinterest archive controller identity differs")
        access = locations.access(State(Path(owner["root"])), lib.info["library_id"])
    with access, FileLock(lib.cache / ".daily-run.lock", timeout=0):
        while True:
            result = compact(lib, force=args.force)
            print(json.dumps(result, ensure_ascii=False), flush=True)
            if not args.drain or result["state"] != "cleaned":
                break


if __name__ == "__main__":
    main()
