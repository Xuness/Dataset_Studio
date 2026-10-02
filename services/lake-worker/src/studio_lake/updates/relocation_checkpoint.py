"""Bounded location checks; reconnecting a directory is not an archive integrity scan."""

from contextlib import ExitStack, closing
import sqlite3

import apsw

from ..online_schema import settings
from ..util import contained, file_hash, read_json
from .sites import UpdateError


def checkpoint(media, index, identity, site, *, allow_missing=False):
    result = {"version": 2, "library_id": identity, "site": site}
    with ExitStack() as stack:
        journal = None
        if media.is_dir():
            if read_json(media / "library.json").get("library_id") != identity:
                raise UpdateError("SOURCE_ID_MISMATCH", "Relocation media belongs to another lake")
            journal = stack.enter_context(closing(sqlite3.connect(
                (media / "journal.sqlite").as_uri() + "?mode=ro", uri=True)))
            # The primary-key tail lookup never reads historical manifest_json blobs.
            head = journal.execute("SELECT seq,batch_id FROM commits ORDER BY seq DESC LIMIT 1").fetchone()
            result.update(archive_seq=head[0] if head else 0, archive_batch=head[1] if head else None)
        elif not allow_missing:
            raise UpdateError("SOURCE_CHANGED", "Relocation media directory is unavailable")

        if not index.is_dir():
            if not allow_missing:
                raise UpdateError("SOURCE_CHANGED", "Relocation index directory is unavailable")
            return result
        pointer = read_json(index / "ONLINE.json")
        if (pointer.get("schema_version") != (3 if site == "pixiv" else 2) or pointer.get("library_id") != identity
                or pointer.get("site") != site):
            raise UpdateError("SOURCE_ID_MISMATCH", "Relocation lake identity or site differs")
        source = stack.enter_context(closing(apsw.Connection(
            str(contained(index, pointer["file"])), flags=apsw.SQLITE_OPEN_READONLY)))
        source.set_busy_timeout(5000)
        state = settings(source)
        if state.get("library_id") != identity or state.get("site") != site:
            raise UpdateError("SOURCE_ID_MISMATCH", "Online database identity or site differs from its pointer")
        served, minimum = int(state["served_seq"]), int(state["min_seq"])
        if state["generation"] != pointer["generation"] or not 0 <= minimum <= served:
            raise UpdateError("SOURCE_CHANGED", "Online projection version or retention range is invalid")
        publication = source.execute("SELECT seq,batch_id FROM publications WHERE seq<=? ORDER BY seq DESC LIMIT 1", (served,)).fetchone()
        if (publication[0] if publication else 0) != served:
            raise UpdateError("SOURCE_CHANGED", "Online publication does not match its serving version")
        if journal is not None:
            batch = journal.execute("SELECT batch_id FROM commits WHERE seq=?", (served,)).fetchone()
            if served > result["archive_seq"] or (served and (batch is None or batch[0] != publication[1])):
                raise UpdateError("SOURCE_CHANGED", "Online projection does not match its archive")
        result.update(generation=state["generation"], served_seq=served, min_seq=minimum,
                      producer_retired=False, producer_marker_digest=None, schema_version=pointer["schema_version"])
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


def verify_checkpoint(expected, actual):
    # Old prepared/verified records have archive_digest and an inventory database.
    # Keep their saved version floors, but do not restart their unbounded scans.
    for key in ("library_id", "site", "generation", "archive_seq", "archive_batch", "schema_version",
                "producer_retired", "producer_marker_digest"):
        if key in expected and actual.get(key) != expected[key]:
            raise UpdateError("SOURCE_CHANGED", "Relocation target differs from the saved lake version")
    if (("served_seq" in expected and actual["served_seq"] < expected["served_seq"])
            or ("min_seq" in expected and actual["min_seq"] > expected["min_seq"])):
        raise UpdateError("SOURCE_CHANGED", "Relocation target is stale or no longer retains the frozen versions")
    if "generation" not in expected and actual["served_seq"] != actual["archive_seq"]:
        raise UpdateError("SOURCE_CHANGED", "Reconnected index must include the latest archive commit")
