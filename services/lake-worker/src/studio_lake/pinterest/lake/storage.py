"""Pinterest batch files use the shared immutable byte-range reader."""

import json
from pathlib import Path

from ...archive_ranges import COPY_BYTES as COPY_BYTES, RangeReader as RangeReader, hash_stream as hash_stream, open_file as open_file
from ...util import IntegrityError


def open_batch_file(path):
    path = Path(path)
    if path.parent.parent.name == "segments":
        return open_file(path.parent.parent.parent, path.relative_to(path.parent.parent.parent).as_posix())
    return path.open("rb")


def json_file(path, maximum=32 * 1024**2):
    with open_batch_file(path) as source:
        size = source.seek(0, 2)
        if size > maximum:
            raise IntegrityError("Pinterest archive JSON exceeds its byte budget")
        source.seek(0)
        body = source.read(size)
    if len(body) != size:
        raise IntegrityError("Pinterest archive JSON is truncated")
    return json.loads(body)


def exists(path):
    try:
        with open_batch_file(path):
            return True
    except FileNotFoundError:
        return False
