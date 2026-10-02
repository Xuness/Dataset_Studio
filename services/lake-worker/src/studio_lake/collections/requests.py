"""Idempotency ledger; secret request fingerprints are DPAPI-protected too."""

import hmac

from .model import identity
from ..media_lake.schema import canonical, utc
from ..updates.credentials import protect
from ..updates.sites import UpdateError
from ..util import digest


def begin(db, operation, request_key, arguments, subject_id=None, *, secret=False):
    identity(request_key)
    fingerprint = digest(canonical(arguments).encode())
    old = db.execute("SELECT * FROM collection_requests WHERE request_key=?", (request_key,)).fetchone()
    if old:
        stored = old["request_hash"]
        if stored.startswith("dpapi:"):
            stored = protect(bytes.fromhex(stored[6:]), False).decode("ascii")
        if old["operation"] != operation or not hmac.compare_digest(stored, fingerprint):
            raise UpdateError("COLLECTION_IDEMPOTENCY_CONFLICT", "Request key already belongs to a different collection operation")
        return dict(old), True
    stored = "dpapi:" + protect(fingerprint.encode("ascii"), True).hex() if secret else fingerprint
    at = utc()
    db.execute("INSERT INTO collection_requests VALUES(?,?,?,'pending',?,NULL,?,?)", (request_key, operation, stored, subject_id, at, at))
    return dict(request_key=request_key, state="pending", subject_id=subject_id, result_json=None), False


def succeed(db, request_key, subject_id, result=None):
    db.execute("UPDATE collection_requests SET state='succeeded',subject_id=?,result_json=?,updated_at=? WHERE request_key=?",
               (subject_id, canonical(result) if result is not None else None, utc(), request_key))
