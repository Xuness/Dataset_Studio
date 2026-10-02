"""Archive-only online-v3 reconstruction using the ordinary maintenance lifecycle."""

from pathlib import Path
import uuid

from . import ONLINE_VERSION
from .library import MediaLibrary
from .online import Publisher, initialize
from .schema import FACTS, MAX_BATCH_METADATA_BYTES, NORMALIZER, utc
from ..online_schema import settings
from ..online_storage import MODULUS, connect, row_digest
from ..raw_codec import decode
from ..util import FileLock, IntegrityError, atomic_json, contained, file_hash, read_json


def build(media, output, *, through=None, chunk_rows=1024, stop=None, reference_index=None):
    from ..archive_rebuild import checked_output, comparison_lease, journal_snapshot, reference_state

    media, output = checked_output(media, output)
    archive = MediaLibrary.archive(media)
    if not 1 <= chunk_rows <= 4096:
        raise ValueError("Publication chunk size must be 1–4096")
    output.mkdir(parents=True, exist_ok=True)
    marker = output / "ONLINE-BUILD.json"
    with FileLock(output / ".archive-build.lock", timeout=1):
        previous = read_json(marker) if marker.exists() else None
        if previous and through is not None and through != previous["sequence"]:
            raise IntegrityError("Cannot change a reconstruction's fixed archive prefix")
        reference = None
        if reference_index is not None:
            reference_index = str(Path(reference_index).resolve())
            reference, state = reference_state(reference_index)
            if (reference["library_id"] != archive.info["library_id"] or reference["schema_version"] != ONLINE_VERSION
                    or reference["site"] != "pixiv" or previous and previous.get("reference_index") != reference_index):
                raise IntegrityError("Reconstruction reference identity differs")
            if previous is None and through is None:
                through = int(state["served_seq"])
        snapshot = journal_snapshot(media, previous["sequence"] if previous else through)
        identity = dict(library_id=archive.info["library_id"], site="pixiv", source="canonical-archive-v2",
                        media_root=str(media), normalization_version=NORMALIZER, schema_version=ONLINE_VERSION, **snapshot)
        if previous:
            if any(previous.get(k) != v for k, v in identity.items()):
                raise IntegrityError("Archive identity or fixed prefix changed during reconstruction")
            plan = previous
            if plan["state"] in {"built", "verified"}:
                if not contained(output, plan["file"]).is_file():
                    raise IntegrityError("Prepared database is missing")
                return plan
            if plan["state"] != "building":
                raise IntegrityError("Reconstruction is not writable")
        else:
            if any(p.name != ".archive-build.lock" for p in output.iterdir()):
                raise IntegrityError("Reconstruction requires an empty independent directory")
            plan = dict(**identity, generation=str(uuid.uuid4()), file="online.sqlite", state="building", created_at=utc())
            if reference is not None:
                plan.update(reference_index=reference_index, reference_generation=reference["generation"])
            atomic_json(marker, plan)
        comparison_lease(plan)
        pointer = initialize(archive.info, output, generation=plan["generation"], activate=False)
        Publisher(archive, index=output, pointer=pointer).sync(through=plan["sequence"], chunk_rows=chunk_rows, stop=stop)
        if journal_snapshot(media, plan["sequence"]) != snapshot:
            raise IntegrityError("Archive prefix changed during reconstruction")
        plan.update(state="built", built_at=utc())
        atomic_json(marker, plan)
        return plan


def signatures(db, sequence, *, maximum_raw_bytes=MAX_BATCH_METADATA_BYTES):
    """Bounded, order-independent signatures; later interval closures are invisible."""
    queries = {}
    for name in FACTS:
        columns = [row[1] for row in db.execute('PRAGMA table_info("' + name + '")')]
        if "commit_seq" in columns or "first_seq" in columns:
            field = "commit_seq" if "commit_seq" in columns else "first_seq"
            queries[name] = (f"SELECT * FROM {name} WHERE {field}<=?", (sequence,))
        else:
            parent, child = {"work_tags": ("work_observations", "observation_id"),
                             "animation_frames": ("media_entries", "media_id"),
                             "discovery_members": ("discovery_snapshots", "snapshot_id")}[name]
            queries[name] = (f"SELECT c.* FROM {name} c JOIN {parent} p USING({child}) WHERE p.commit_seq<=?", (sequence,))
    queries["tags"] = ("SELECT DISTINCT t.* FROM tags t JOIN work_tags wt USING(tag_id) JOIN work_observations w USING(observation_id) WHERE w.commit_seq<=?", (sequence,))
    for name in ("work_versions", "media_asset_versions"):
        fields = [row[1] for row in db.execute('PRAGMA table_info("' + name + '")')]
        selected = ",".join("CASE WHEN valid_until>? THEN NULL ELSE valid_until END" if f == "valid_until" else f for f in fields)
        queries[name] = (f"SELECT {selected} FROM {name} WHERE valid_from<=?", (sequence, sequence))
    queries["changes"] = ("SELECT * FROM changes WHERE seq<=?", (sequence,))
    result = {}
    for name, (query, args) in queries.items():
        count = hashed = 0
        for row in db.execute(query, args):
            hashed = (hashed + row_digest(row)) % MODULUS
            count += 1
        result[name] = dict(rows=count, digest=format(hashed, "x"))
    for body, size, sha in db.execute("SELECT raw_zlib,raw_bytes,raw_sha256 FROM captures WHERE commit_seq<=?", (sequence,)):
        decode(body, size, sha, maximum_bytes=maximum_raw_bytes)
    return result


def verify(output, *, maximum_raw_bytes=MAX_BATCH_METADATA_BYTES):
    from ..archive_rebuild import checked_output, journal_snapshot

    output = Path(output).resolve()
    plan = read_json(output / "ONLINE-BUILD.json")
    if plan.get("source") != "canonical-archive-v2" or plan["state"] not in {"built", "verified"}:
        raise IntegrityError("Reconstruction is not ready for verification")
    media, _ = checked_output(plan["media_root"], output)
    snapshot = journal_snapshot(media, plan["sequence"])
    if any(plan[k] != v for k, v in snapshot.items()):
        raise IntegrityError("Archive prefix changed after reconstruction")
    with FileLock(output / ".archive-build.lock", timeout=1), FileLock(output / ".online.lock"):
        path = contained(output, plan["file"])
        db = connect(path)
        try:
            state = settings(db)
            if (state["library_id"] != plan["library_id"] or state["generation"] != plan["generation"]
                    or int(state["served_seq"]) != plan["sequence"]):
                raise IntegrityError("Reconstructed state identity differs")
            if list(db.execute("PRAGMA integrity_check")) != [("ok",)] or list(db.execute("PRAGMA foreign_key_check")):
                raise IntegrityError("Reconstructed database failed integrity verification")
            if next(db.execute("SELECT count(*) FROM pending_publication"))[0]:
                raise IntegrityError("Reconstruction contains an unfinished publication")
            db.execute("INSERT INTO tag_index(tag_index) VALUES('integrity-check')")
            tables = signatures(db, plan["sequence"], maximum_raw_bytes=maximum_raw_bytes)
            db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
        finally:
            db.close()
        wal = Path(str(path) + "-wal")
        if wal.exists() and wal.stat().st_size:
            raise IntegrityError("Prepared database still has an active WAL")
        result = dict(checked_at=utc(), source=plan["source"], sequence=plan["sequence"], journal_digest=plan["journal_digest"],
                      tables=tables, raw_roundtrip_verified=True, image_payloads_rehashed=False, file_sha256=file_hash(path))
        atomic_json(output / "ONLINE-VERIFY.json", result)
        plan.update(state="verified", verified_at=utc())
        atomic_json(output / "ONLINE-BUILD.json", plan)
        return result


def compare(output):
    from ..archive_rebuild import comparison_lease, reference_state

    output = Path(output).resolve()
    plan = read_json(output / "ONLINE-BUILD.json")
    if plan["state"] != "verified" or not plan.get("reference_index"):
        raise IntegrityError("Comparison requires a verified preparation and a protected reference")
    comparison_lease(plan)
    reference = Path(plan["reference_index"])
    pointer, _ = reference_state(reference)
    proofs = []
    with FileLock(output / ".archive-build.lock", timeout=1):
        for path in (contained(output, plan["file"]), contained(reference, pointer["file"])):
            db = connect(path)
            try:
                db.execute("PRAGMA query_only=ON")
                db.execute("BEGIN")
                proofs.append(signatures(db, plan["sequence"]))
            finally:
                db.close()
        result = dict(checked_at=utc(), sequence=plan["sequence"], reference_generation=plan["reference_generation"],
                      equal=proofs[0] == proofs[1], tables={k: dict(rebuilt=proofs[0][k], existing=proofs[1][k], equal=proofs[0][k] == proofs[1][k]) for k in proofs[0]})
        atomic_json(output / "ONLINE-COMPARE.json", result)
        if not result["equal"]:
            raise IntegrityError("Reconstruction differs from the reference; preparation and lease retained")
        comparison_lease(plan, release=True)
        return result
