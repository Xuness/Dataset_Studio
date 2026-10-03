"""Checks that need no serving writes, before making a result durable."""

import pyarrow as pa

from .failures import InvalidReceipt
from ..media_lake.schema import MAX_BATCH_METADATA_BYTES, arrow_schema, check_rows
from ..util import IntegrityError, stable_id


def records(records, library_id, *, detail=None):
    try:
        check_rows(records)
        size = sum(pa.Table.from_pylist(rows, schema=arrow_schema(name)).nbytes for name, rows in records.items())
        if size > MAX_BATCH_METADATA_BYTES:
            raise IntegrityError("Canonical metadata exceeds its byte budget")
        for row in records.get("captures", []):
            if row["capture_id"] != stable_id("capture-v1", library_id, row["request_receipt_id"]):
                raise IntegrityError("Capture identity mismatch")
        for name, field, prefix in (("work_observations", "work_id", "work-observation-v1"),
                                    ("author_observations", "author_id", "author-observation-v1")):
            for row in records.get(name, []):
                if row["observation_id"] != stable_id(prefix, row["capture_id"], row[field], row["normalizer_version"]):
                    raise IntegrityError("Observation identity mismatch")
        manifests = {r["manifest_id"]: r for r in records.get("media_manifests", [])}
        entries, frames = {}, {}
        for frame in records.get("animation_frames", []):
            frames.setdefault(frame["media_id"], []).append(frame)
        for row in records.get("media_entries", []):
            if row["media_id"] != stable_id("media-entry-v1", row["manifest_id"], row["slot_key"]):
                raise IntegrityError("Media identity mismatch")
            expected = "page:" + str(row["ordinal"]) if row["kind"] == "image" else "animation:0"
            if row["slot_key"] != expected or row["ordinal"] < 0:
                raise IntegrityError("Media slot mismatch")
            members = frames.get(row["media_id"], [])
            if sorted(f["ordinal"] for f in members) != list(range(len(members))) or len({f["file_name"] for f in members}) != len(members):
                raise IntegrityError("Animation frame sequence is invalid")
            if row["kind"] == "ugoira" and not members and manifests.get(row["manifest_id"], {}).get("complete"):
                raise IntegrityError("A complete animation requires frames")
            entries.setdefault(row["manifest_id"], []).append(row)
        details = {r["observation_id"]: r for r in records.get("work_observations", [])}
        if detail:
            details[detail["observation_id"]] = detail
        for row in manifests.values():
            if row["manifest_id"] != stable_id("media-manifest-v1", row["capture_id"], row["work_id"],
                                               row["detail_observation_id"] or "", row["normalizer_version"]):
                raise IntegrityError("Manifest identity mismatch")
            members = entries.get(row["manifest_id"], [])
            if len(members) != row["item_count"] or any(m["work_id"] != row["work_id"] for m in members):
                raise IntegrityError("Manifest members disagree")
            if row["complete"]:
                if not members or sorted(m["ordinal"] for m in members) != list(range(len(members))):
                    raise IntegrityError("A complete manifest requires contiguous members")
                if row["expected_count"] is not None and row["expected_count"] != len(members):
                    raise IntegrityError("Complete manifest count disagrees")
                if row["kind"] == "ugoira" and (len(members) != 1 or members[0]["kind"] != "ugoira"):
                    raise IntegrityError("An animation requires exactly one member")
                basis = details.get(row["detail_observation_id"])
                if basis and row["kind"] == "image_pages" and basis["page_count"] is not None and basis["page_count"] != len(members):
                    raise IntegrityError("Detail and manifest count disagree")
        return sum(len(rows) for rows in records.values()), size
    except (IntegrityError, ValueError, TypeError, KeyError, OverflowError, pa.ArrowException) as error:
        raise InvalidReceipt("Canonical result failed preflight: " + type(error).__name__) from error
