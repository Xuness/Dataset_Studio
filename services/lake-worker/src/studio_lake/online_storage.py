"""Online SQLite primitives independent of the retired native-index converter."""

import base64
from datetime import datetime, timezone
import hashlib
import json

import apsw

from .online_schema import TIME_COLUMNS
from .util import IntegrityError

MODULUS = 1 << 256


def connect(path, *, building=False):
    if tuple(map(int, apsw.sqlitelibversion().split("."))) < (3, 51, 3):
        raise IntegrityError("在线写入需要已修复 WAL-reset 的 SQLite 3.51.3 或更高版本")
    db = apsw.Connection(str(path))
    db.set_busy_timeout(30000)
    db.execute("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
    db.execute("PRAGMA cache_size=" + ("-2097152" if building else "-262144"))
    db.execute("PRAGMA synchronous=" + ("NORMAL" if building else "FULL"))
    db.execute("PRAGMA wal_autocheckpoint=16384; PRAGMA journal_size_limit=67108864;")
    return db


def scalar(value, name):
    if isinstance(value, bool):
        return int(value)
    if name in TIME_COLUMNS and value is not None:
        if isinstance(value, str):
            value = datetime.fromisoformat(value.replace("Z", "+00:00"))
        if value.tzinfo is None:
            value = value.replace(tzinfo=timezone.utc)
        return value.astimezone(timezone.utc).strftime("%Y-%m-%d %H:%M:%S.%f+00:00")
    return value


def row_digest(row):
    data = [{"blob": base64.b64encode(v).decode("ascii")} if isinstance(v, bytes) else v for v in row]
    return int.from_bytes(
        hashlib.sha256(
            json.dumps(data, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode()
        ).digest(),
        "big",
    )
