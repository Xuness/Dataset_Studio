"""Stable archive filenames backed by loose files or immutable container ranges."""

from contextlib import closing
import hashlib
import io
from pathlib import Path
import sqlite3

from .util import IntegrityError, contained, read_json

COPY_BYTES = 1024 * 1024
FEATURE = "archive-containers-v1"


class RangeReader(io.RawIOBase):
    """A seekable view that cannot escape one original file inside a container."""

    def __init__(self, source, offset, length):
        self.source, self.offset, self.length, self.position = source, offset, length, 0
        if offset < 0 or length < 0 or offset + length > source.seek(0, 2):
            source.close()
            raise IntegrityError("Archive container range is truncated")
        source.seek(offset)

    def readable(self):
        return True

    def seekable(self):
        return True

    def tell(self):
        return self.position

    def seek(self, offset, whence=0):
        position = offset + (0 if whence == 0 else self.position if whence == 1 else self.length if whence == 2 else -1)
        if whence not in (0, 1, 2) or position < 0:
            raise ValueError("Invalid archive seek")
        self.source.seek(self.offset + position)
        self.position = position
        return position

    def read(self, size=-1):
        size = max(0, self.length - self.position) if size < 0 else max(0, min(size, self.length - self.position))
        data = self.source.read(size)
        self.position += len(data)
        return data

    def readinto(self, buffer):
        data = self.read(len(buffer))
        buffer[:len(data)] = data
        return len(data)

    def close(self):
        self.source.close()
        super().close()


def location(root, relative):
    """Mappings are published once, never moved or retired; no stale range cache."""
    root = Path(root)
    contained(root, relative)
    info = read_json(root / "library.json")
    if FEATURE not in info.get("required_features", []):
        return None
    with closing(sqlite3.connect((root / "journal.sqlite").as_uri() + "?mode=ro", uri=True, timeout=30)) as db:
        row = db.execute("""SELECT c.path,m.offset,m.length,m.sha256,c.bytes FROM archive_members m
            JOIN archive_containers c USING(container_id) WHERE m.path=? AND c.state IN ('active','cleaned')""", (relative,)).fetchone()
    return row


def open_file(root, relative):
    """Open first, then fall back after ENOENT: concurrent reclamation is harmless."""
    path = contained(Path(root), relative)
    try:
        return path.open("rb")
    except FileNotFoundError:
        row = location(Path(root), relative)
        if row is None:
            raise
    container, offset, length, _, size = row
    source = contained(Path(root), container).open("rb")
    if source.seek(0, 2) != size:
        source.close()
        raise IntegrityError("Archive container size changed")
    return RangeReader(source, offset, length)


def hash_stream(source, *, stop=None):
    value = hashlib.sha256()
    while True:
        if stop and stop():
            raise InterruptedError("Archive maintenance interrupted")
        chunk = source.read(COPY_BYTES)
        if not chunk:
            return value.hexdigest()
        value.update(chunk)
