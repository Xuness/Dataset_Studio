"""Preserve a bounded set of legacy serving choices as explicit archive metadata.

These are historical associations, not new source facts or a change to publication
rules. A later observation or asset for the post makes the association obsolete.
"""

from contextlib import closing
import itertools
from pathlib import Path
import sqlite3

from .online_storage import MODULUS, connect, row_digest
from .util import FileLock, IntegrityError, atomic_json, contained, file_hash, now, read_json

RELATIVE = "source_manifests/legacy-online-bindings-v1.json"
MAX_BINDINGS = 10000


def load(media, identity, site):
    path = contained(Path(media), RELATIVE)
    if not path.exists():
        return None, None
    if path.stat().st_size > 8 * 1024**2:
        raise IntegrityError("历史关联清单超过读取预算")
    value = read_json(path)
    if (
        value.get("schema_version") != 1
        or value.get("kind") != "legacy_online_bindings"
        or value.get("library_id") != identity
        or value.get("site") != site
        or not isinstance(value.get("captured_sequence"), int)
        or value["captured_sequence"] < 0
        or not isinstance(value.get("bindings"), list)
        or len(value["bindings"]) > MAX_BINDINGS
    ):
        raise IntegrityError("历史关联清单身份或格式无效")
    seen = set()
    for row in value["bindings"]:
        if (
            type(row.get("post_id")) is not int
            or row["post_id"] in seen
            or not isinstance(row.get("observation_id"), str)
            or not isinstance(row.get("asset_id"), str)
            or row.get("reason") != "legacy_missing_source_md5"
        ):
            raise IntegrityError("历史关联清单含无效或重复条目")
        seen.add(row["post_id"])
    return value, file_hash(path)


def apply_to_preparation(db, media, build):
    manifest, fingerprint = load(media, build["library_id"], build["site"])
    if fingerprint != build.get("legacy_bindings_sha256"):
        raise IntegrityError("历史关联归档在重建后发生变化")
    if manifest is None or build["sequence"] < manifest["captured_sequence"]:
        return 0
    applied = 0
    with db:
        for row in manifest["bindings"]:
            post = row["post_id"]
            touched = next(
                db.execute(
                    "SELECT max(commit_seq) FROM (SELECT commit_seq FROM observations WHERE post_id=? "
                    "UNION ALL SELECT commit_seq FROM assets WHERE post_id=?)",
                    (post, post),
                )
            )[0]
            if touched is not None and touched > manifest["captured_sequence"]:
                continue
            current = next(
                db.execute(
                    "SELECT o.observation_id,o.md5,p.asset_id FROM post_versions p JOIN observations o "
                    "ON o.row_id=p.row_id WHERE p.post_id=? AND p.valid_from<=? "
                    "AND (p.valid_until IS NULL OR p.valid_until>?)",
                    (post, build["sequence"], build["sequence"]),
                ),
                None,
            )
            asset = next(
                db.execute("SELECT post_id,commit_seq FROM assets WHERE asset_id=?", (row["asset_id"],)), None
            )
            if (
                not current
                or current[0] != row["observation_id"]
                or current[1]
                or current[2] not in (None, row["asset_id"])
                or not asset
                or asset[0] != post
                or asset[1] > manifest["captured_sequence"]
            ):
                raise IntegrityError("历史关联不能由归档中的同帖观察与资产证明")
            db.execute(
                "UPDATE post_versions SET asset_id=? WHERE post_id=? AND valid_until IS NULL",
                (row["asset_id"], post),
            )
            applied += 1
    return applied


def post_rows(db, seq):
    return db.execute(
        "SELECT p.post_id,o.observation_id,p.asset_id,o.md5 FROM post_versions p JOIN observations o "
        "ON o.row_id=p.row_id WHERE p.valid_from<=? AND (p.valid_until IS NULL OR p.valid_until>?) "
        "ORDER BY p.post_id",
        (seq, seq),
    )


def adopt(output):
    """Capture only proven legacy associations, then recheck the entire changed relation.

    The previous full verification is carried forward only after its database hash is
    confirmed and a transaction changes exclusively post_versions. An interrupted
    adoption stays unverified; ordinary verify/compare can validate it in full again.
    """
    from .archive_rebuild import (
        checked_output,
        comparison_lease,
        journal_snapshot,
        reference_state,
        verify_archive,
        compare_reference,
    )
    from .producer_retirement import maintenance_locks

    output = Path(output).resolve()
    build = read_json(output / "ONLINE-BUILD.json")
    media, output = checked_output(Path(build["media_root"]), output)
    index = Path(build["reference_index"])
    pointer, state = reference_state(index)
    if (
        build.get("source") != "canonical-archive-v1"
        or build["state"] not in {"built", "verified"}
        or pointer["library_id"] != build["library_id"]
        or pointer["site"] != build["site"]
        or read_json(media / "library.json")["library_id"] != build["library_id"]
        or pointer["generation"] != build["reference_generation"]
        or int(state["served_seq"]) != build["sequence"]
    ):
        raise IntegrityError("历史关联交接要求原参考代次与固定已验证水位")
    prior_adoption = output / "LEGACY-BINDINGS-ADOPTION.json"
    if build["state"] == "built":
        if not prior_adoption.exists() or read_json(prior_adoption).get("phase") != "prepared":
            raise IntegrityError("需要完整验证结果或可恢复的历史关联交接")
        with FileLock(output / ".archive-build.lock"), maintenance_locks(media, index, build["library_id"]):
            db = connect(contained(output, build["file"]), building=True)
            try:
                apply_to_preparation(db, media, build)
                db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
            finally:
                db.close()
        # After interruption there is no shortcut based on the previous file hash.
        verify_archive(output)
        compare_reference(output)
        receipt = read_json(prior_adoption)
        receipt.update(
            phase="complete",
            completed_at=now(),
            resumed_with_full_verification=True,
            file_sha256=read_json(output / "ONLINE-VERIFY.json")["file_sha256"],
        )
        atomic_json(prior_adoption, receipt)
        return receipt
    with FileLock(output / ".archive-build.lock"), maintenance_locks(media, index, build["library_id"]):
        proof = read_json(output / "ONLINE-VERIFY.json")
        compared = read_json(output / "ONLINE-COMPARE.json")
        if prior_adoption.exists() and read_json(prior_adoption).get("phase") == "complete":
            if load(media, build["library_id"], build["site"])[1] != build.get("legacy_bindings_sha256"):
                raise IntegrityError("历史关联归档在验证后发生变化")
            return read_json(prior_adoption)
        expected = {
            "objects",
            "observations",
            "assets",
            "raw_metadata",
            "source_schemas",
            "current_posts",
            "current_objects",
        }
        if (
            set(compared["tables"]) != expected
            or not all(compared["tables"][k]["equal"] for k in expected - {"current_posts"})
            or compared["tables"]["current_posts"]["equal"]
            or compared["sequence"] != build["sequence"]
            or compared["reference_generation"] != pointer["generation"]
            or not proof.get("raw_roundtrip_verified")
            or proof["sequence"] != build["sequence"]
            or proof["journal_digest"] != build["journal_digest"]
        ):
            raise IntegrityError("只有完整事实对照通过、仅当前帖子关联不同的库可以交接")
        snapshot = journal_snapshot(media, build["sequence"])
        if any(snapshot[k] != build[k] for k in snapshot):
            raise IntegrityError("归档提交前缀发生变化")
        path = contained(output, build["file"])
        wal = Path(str(path) + "-wal")
        if (wal.exists() and wal.stat().st_size) or file_hash(path) != proof["file_sha256"]:
            raise IntegrityError("交接前的完整验证文件已变化")
        bindings = []
        with (
            closing(sqlite3.connect(path.as_uri() + "?mode=ro", uri=True)) as prepared,
            closing(
                sqlite3.connect(contained(index, pointer["file"]).as_uri() + "?mode=ro", uri=True)
            ) as serving,
        ):
            for db in (prepared, serving):
                db.execute("PRAGMA cache_size=-524288")
                db.execute("BEGIN")
            for left, right in itertools.zip_longest(
                post_rows(prepared, build["sequence"]), post_rows(serving, build["sequence"])
            ):
                if left == right:
                    continue
                if (
                    not left
                    or not right
                    or left[:2] != right[:2]
                    or left[2] is not None
                    or right[2] is None
                    or left[3]
                    or right[3]
                    or len(bindings) >= MAX_BINDINGS
                ):
                    raise IntegrityError("存在无法解释为历史无 MD5 关联的差异，保留原库")
                asset = prepared.execute(
                    "SELECT post_id,commit_seq FROM assets WHERE asset_id=?", (right[2],)
                ).fetchone()
                if not asset or asset[0] != left[0] or asset[1] > build["sequence"]:
                    raise IntegrityError("历史关联资产不能从同帖归档证明")
                bindings.append(
                    {
                        "post_id": left[0],
                        "observation_id": left[1],
                        "asset_id": right[2],
                        "reason": "legacy_missing_source_md5",
                    }
                )
        manifest = {
            "schema_version": 1,
            "kind": "legacy_online_bindings",
            "library_id": build["library_id"],
            "site": build["site"],
            "reference_generation": pointer["generation"],
            "captured_sequence": build["sequence"],
            "captured_at": now(),
            "bindings": bindings,
        }
        archive_path = contained(media, RELATIVE)
        if archive_path.exists():
            raise IntegrityError("历史关联归档已存在；不覆盖既有交接，使用完整验证恢复")
        atomic_json(output / "ONLINE-VERIFY.before-bindings.json", proof)
        atomic_json(output / "ONLINE-COMPARE.before-bindings.json", compared)
        atomic_json(archive_path, manifest)
        build.update(state="built", legacy_bindings_sha256=file_hash(archive_path))
        atomic_json(output / "ONLINE-BUILD.json", build)
        receipt = {
            "phase": "prepared",
            "captured_at": now(),
            "rows": len(bindings),
            "source_manifest": RELATIVE,
            "source_sha256": build["legacy_bindings_sha256"],
            "prior_file_sha256": proof["file_sha256"],
            "library_id": build["library_id"],
        }
        atomic_json(prior_adoption, receipt)
        db = connect(path, building=True)
        try:
            if apply_to_preparation(db, media, build) != len(bindings):
                raise IntegrityError("历史关联回放数量与交接清单不一致")
            db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
        finally:
            db.close()
        # All six unchanged logical relations retain their full comparison proofs.
        # Recompute the changed relation in full on both sides, including its row count.
        signatures = []
        for target in (path, contained(index, pointer["file"])):
            with closing(sqlite3.connect(target.as_uri() + "?mode=ro", uri=True)) as db:
                db.execute("PRAGMA cache_size=-524288")
                count = hashed = 0
                for row in post_rows(db, build["sequence"]):
                    count += 1
                    hashed = (hashed + row_digest(row[:3])) % MODULUS
                signatures.append([count, format(hashed, "x")])
        if signatures[0] != signatures[1]:
            raise IntegrityError("历史关联回放后，完整帖子关系仍不一致")
        signature = path.stat()
        proof.update(
            file_signature=[signature.st_size, signature.st_mtime_ns],
            file_sha256=file_hash(path),
            legacy_bindings_sha256=build["legacy_bindings_sha256"],
            checked_at=now(),
            controlled_repair={
                "table": "post_versions",
                "rows": len(bindings),
                "prior_file_sha256": receipt["prior_file_sha256"],
                "unchanged_full_raw_verification_preserved": True,
            },
        )
        compared["tables"]["current_posts"] = {
            "rebuilt": signatures[0],
            "existing": signatures[1],
            "equal": True,
        }
        compared.update(
            equal=True,
            checked_at=now(),
            legacy_bindings_sha256=build["legacy_bindings_sha256"],
            prior_full_comparison="ONLINE-COMPARE.before-bindings.json",
            rechecked_tables=["current_posts"],
        )
        atomic_json(output / "ONLINE-VERIFY.json", proof)
        atomic_json(output / "ONLINE-COMPARE.json", compared)
        build.update(state="verified", verified_at=now())
        atomic_json(output / "ONLINE-BUILD.json", build)
        receipt.update(phase="complete", completed_at=now(), file_sha256=proof["file_sha256"])
        atomic_json(prior_adoption, receipt)
    comparison_lease(build, release=True)
    return receipt
