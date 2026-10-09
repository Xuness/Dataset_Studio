"""Source-independent canonical JSON and timestamp primitives."""

from datetime import datetime, timezone
import json

from .util import IntegrityError


def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False)


def utc(value=None):
    date = datetime.now(timezone.utc) if value is None else datetime.fromisoformat(value.replace("Z", "+00:00"))
    if date.tzinfo is None:
        raise IntegrityError("A source timestamp requires a timezone")
    return date.astimezone(timezone.utc).isoformat(timespec="microseconds").replace("+00:00", "Z")
