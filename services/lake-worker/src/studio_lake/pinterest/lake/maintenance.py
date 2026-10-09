"""Pinterest archive reconstruction through the common preparation/verification lifecycle."""

from pathlib import Path
import uuid

from . import FEATURES, ONLINE_VERSION, SCHEMA_SET, schema
from .library import PinterestLibrary
from .online import Publisher, initialize
from .. import NORMALIZER
from ...canonical import utc
from ...online_schema import settings
from ...online_storage import MODULUS, connect, row_digest
from ...raw_codec import decode
from ...util import FileLock, IntegrityError, atomic_json, contained, file_hash, read_json


def build(media, output, *, through=None, chunk_rows=1024, stop=None, reference_index=None):
    from ...archive_rebuild import checked_output, comparison_lease, journal_snapshot, reference_state
    media, output = checked_output(media, output)
    archive = PinterestLibrary.archive(media)
    if not 1 <= chunk_rows <= 4096:
        raise ValueError("Invalid reconstruction read bound")
    output.mkdir(parents=True, exist_ok=True)
    marker = output / "ONLINE-BUILD.json"
    with FileLock(output / ".archive-build.lock", timeout=1):
        previous = read_json(marker) if marker.exists() else None
        if previous and through is not None and through != previous["sequence"]:
            raise IntegrityError("Cannot change a reconstruction's archive prefix")
        reference = None
        if reference_index is not None:
            reference_index = str(Path(reference_index).resolve())
            reference, state = reference_state(reference_index)
            if (reference["library_id"] != archive.info["library_id"] or reference.get("schema_set") != SCHEMA_SET
                    or reference.get("required_features") != list(FEATURES) or reference["schema_version"] != ONLINE_VERSION
                    or reference["site"] != "pinterest" or previous and previous.get("reference_index") != reference_index):
                raise IntegrityError("Pinterest reference identity differs")
            if previous is None and through is None:
                through = int(state["served_seq"])
        snapshot = journal_snapshot(media, previous["sequence"] if previous else through)
        identity = dict(library_id=archive.info["library_id"], site="pinterest", source="canonical-archive-v3",
            media_root=str(media), normalization_version=NORMALIZER, schema_version=ONLINE_VERSION,
            schema_set=SCHEMA_SET, required_features=list(FEATURES), **snapshot)
        if previous:
            if any(previous.get(k) != v for k, v in identity.items()):
                raise IntegrityError("Pinterest reconstruction identity or prefix changed")
            plan = previous
            if plan["state"] in ("built", "verified"):
                if not contained(output, plan["file"]).is_file():
                    raise IntegrityError("Prepared Pinterest database is missing")
                return plan
            if plan["state"] != "building":
                raise IntegrityError("Pinterest reconstruction is not writable")
        else:
            if any(p.name != ".archive-build.lock" for p in output.iterdir()):
                raise IntegrityError("Pinterest reconstruction requires an empty independent directory")
            plan = dict(**identity, generation=str(uuid.uuid4()), file="online.sqlite", state="building", created_at=utc())
            if reference:
                plan.update(reference_index=reference_index, reference_generation=reference["generation"])
            atomic_json(marker, plan)
        comparison_lease(plan)
        pointer = initialize(archive.info, output, generation=plan["generation"], activate=False)
        Publisher(archive, index=output, pointer=pointer).sync(through=plan["sequence"], stop=stop)
        if journal_snapshot(media, plan["sequence"]) != snapshot:
            raise IntegrityError("Pinterest archive prefix changed during reconstruction")
        plan.update(state="built", built_at=utc())
        atomic_json(marker, plan)
        return plan


def signatures(db, sequence, *, maximum_raw_bytes=schema.MAX_METADATA):
    result = {}
    for name in (*schema.FACTS, "changes"):
        columns = [row[1] for row in db.execute('PRAGMA table_info("' + name + '")')]
        field = "seq" if name == "changes" else "first_seq" if "first_seq" in columns else "commit_seq"
        count = hashed = 0
        for row in db.execute(f"SELECT * FROM {name} WHERE {field}<=?", (sequence,)):
            hashed = (hashed + row_digest(row)) % MODULUS
            count += 1
        result[name] = dict(rows=count, digest=format(hashed, "x"))
    for body, size, sha in db.execute("SELECT raw_zlib,raw_bytes,raw_sha256 FROM captures WHERE commit_seq<=?", (sequence,)):
        decode(body, size, sha, maximum_bytes=maximum_raw_bytes)
    return result


def verify(output, *, maximum_raw_bytes=schema.MAX_METADATA):
    from ...archive_rebuild import checked_output, journal_snapshot
    output = Path(output).resolve()
    plan = read_json(output / "ONLINE-BUILD.json")
    if plan.get("source") != "canonical-archive-v3" or plan["state"] not in ("built", "verified"):
        raise IntegrityError("Pinterest reconstruction is not ready")
    media, _ = checked_output(plan["media_root"], output)
    if any(plan[k] != v for k, v in journal_snapshot(media, plan["sequence"]).items()):
        raise IntegrityError("Pinterest reconstruction prefix changed")
    with FileLock(output / ".archive-build.lock", timeout=1), FileLock(output / ".online.lock"):
        path = contained(output, plan["file"])
        db = connect(path)
        try:
            state = settings(db)
            if (state["library_id"] != plan["library_id"] or state["generation"] != plan["generation"]
                    or int(state["served_seq"]) != plan["sequence"]):
                raise IntegrityError("Pinterest reconstructed identity differs")
            if list(db.execute("PRAGMA integrity_check")) != [("ok",)] or list(db.execute("PRAGMA foreign_key_check")):
                raise IntegrityError("Pinterest reconstruction failed database validation")
            tables = signatures(db, plan["sequence"], maximum_raw_bytes=maximum_raw_bytes)
            db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
        finally:
            db.close()
        wal = Path(str(path) + "-wal")
        if wal.exists() and wal.stat().st_size:
            raise IntegrityError("Pinterest preparation still has an active WAL")
        result = dict(checked_at=utc(), source=plan["source"], sequence=plan["sequence"], journal_digest=plan["journal_digest"],
            tables=tables, raw_roundtrip_verified=True, image_payloads_rehashed=False, file_sha256=file_hash(path))
        atomic_json(output / "ONLINE-VERIFY.json", result)
        plan.update(state="verified", verified_at=utc())
        atomic_json(output / "ONLINE-BUILD.json", plan)
        return result


def compare(output):
    from ...archive_rebuild import comparison_lease, reference_state
    output = Path(output).resolve()
    plan = read_json(output / "ONLINE-BUILD.json")
    if plan["state"] != "verified" or not plan.get("reference_index"):
        raise IntegrityError("Pinterest comparison requires a verified preparation and a protected reference")
    comparison_lease(plan)
    pointer, _ = reference_state(plan["reference_index"])
    proofs = []
    with FileLock(output / ".archive-build.lock", timeout=1):
        for path in (contained(output, plan["file"]), contained(Path(plan["reference_index"]), pointer["file"])):
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
            raise IntegrityError("Pinterest reconstruction differs from the reference")
        comparison_lease(plan, release=True)
        return result
