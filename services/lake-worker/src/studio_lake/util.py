from contextlib import AbstractContextManager
from datetime import datetime, timezone
from pathlib import Path
import base64
import decimal
import hashlib
import json
import math
import numbers
import os
import stat
import time
import uuid
from email.utils import parsedate_to_datetime


class IntegrityError(RuntimeError):
    pass


def retry_after_seconds(headers):
    value = headers.get("Retry-After")
    if not value:
        return None
    try:
        seconds = float(value)
        return max(0.0, seconds) if math.isfinite(seconds) else None
    except ValueError:
        try:
            return max(0.0, parsedate_to_datetime(value).timestamp() - time.time())
        except (ValueError, TypeError, OverflowError):
            return None


def now():
    return datetime.now(timezone.utc).isoformat(timespec="microseconds")


def digest(data: bytes):
    return hashlib.sha256(data).hexdigest()


def file_hash(path: Path):
    h = hashlib.sha256()
    with path.open("rb") as f:
        for data in iter(lambda: f.read(8 * 1024**2), b""):
            h.update(data)
    return h.hexdigest()


def stable_id(*parts):
    return digest(json.dumps(parts, ensure_ascii=False, separators=(",", ":")).encode())


def json_text(obj):
    return json.dumps(obj, ensure_ascii=False, indent=2, allow_nan=False)


def read_json(path: Path):
    return json.loads(path.read_text(encoding="utf-8"))


def sync_file(path: Path):
    with path.open("r+b") as f:
        f.flush()
        os.fsync(f.fileno())


def sync_directory(path: Path):
    if os.name != "nt":
        fd = os.open(path, os.O_RDONLY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)


def atomic_json(path: Path, obj):
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_name(path.name + "." + uuid.uuid4().hex + ".tmp")
    with temp.open("x", encoding="utf-8", newline="\n") as f:
        f.write(json_text(obj) + "\n")
        f.flush()
        os.fsync(f.fileno())
    for attempt in range(8):
        try:
            os.replace(temp, path)
            break
        except OSError as error:
            if os.name != "nt" or getattr(error, "winerror", None) not in {5,32,33} or attempt==7:
                raise
            time.sleep(min(0.01*2**attempt,0.1))
    sync_file(path)
    sync_directory(path.parent)


def contained(root: Path, relative: str):
    candidate = Path(relative)
    if candidate.is_absolute() or ".." in candidate.parts or ":" in relative:
        raise IntegrityError(f"不安全的相对路径: {relative!r}")
    resolved = (root / candidate).resolve()
    if resolved == root.resolve() or root.resolve() not in resolved.parents:
        raise IntegrityError(f"路径越界: {relative!r}")
    return resolved


def safe_managed_path(base: Path, path: Path):
    """Check the lexical/resolved boundary and reject links, including Windows junctions."""
    base, path = Path(base).absolute(), Path(path).absolute()
    if path == base or base not in path.parents or path.resolve() != path:
        raise IntegrityError(f"路径越界或经过链接: {path}")
    current = base
    for part in (None, *path.relative_to(base).parts):
        if part is not None:
            current /= part
        try:
            info = current.lstat()
        except FileNotFoundError:
            continue
        if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & 1024:
            raise IntegrityError(f"受管理目录包含链接: {current}")
    return path


def failpoint(name):
    if os.environ.get("DANBOORU_STORE_FAIL_AT") == name:
        if os.environ.get("DANBOORU_STORE_HARD_EXIT") == "1":
            if os.name == "nt":
                # CRT _exit can itself fault during native-library teardown.
                # TerminateProcess models abrupt process death without running DLL cleanup.
                import ctypes
                from ctypes import wintypes

                kernel = ctypes.WinDLL("kernel32", use_last_error=True)
                kernel.GetCurrentProcess.restype = wintypes.HANDLE
                kernel.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
                kernel.TerminateProcess.restype = wintypes.BOOL
                if not kernel.TerminateProcess(kernel.GetCurrentProcess(), 91):
                    raise ctypes.WinError(ctypes.get_last_error())
            os._exit(91)
        raise RuntimeError(f"injected failure: {name}")


class FileLock(AbstractContextManager):
    """OS-owned lock, automatically released when the process terminates."""

    def __init__(self, path: Path, timeout=30):
        self.path, self.timeout, self.handle = path, timeout, None

    def __enter__(self):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.handle = self.path.open("a+b")
        if self.path.stat().st_size == 0:
            self.handle.write(b"0")
            self.handle.flush()
        end = time.monotonic() + self.timeout
        while True:
            try:
                self.handle.seek(0)
                if os.name == "nt":
                    import msvcrt

                    msvcrt.locking(self.handle.fileno(), msvcrt.LK_NBLCK, 1)
                else:
                    import fcntl

                    fcntl.flock(self.handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                return self
            except OSError:
                if time.monotonic() >= end:
                    self.handle.close()
                    raise RuntimeError(f"另一个进程正在使用此工作区: {self.path}") from None
                time.sleep(0.1)

    def __exit__(self, *args):
        if self.handle:
            self.handle.seek(0)
            if os.name == "nt":
                import msvcrt

                msvcrt.locking(self.handle.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                import fcntl

                fcntl.flock(self.handle.fileno(), fcntl.LOCK_UN)
            self.handle.close()


def typed_value(value):
    """JSON representation of Arrow values; authoritative typed rows stay in Parquet."""
    if isinstance(value, bytes):
        return {"$type": "bytes", "base64": base64.b64encode(value).decode()}
    if isinstance(value, decimal.Decimal):
        return {"$type": "decimal", "value": str(value)}
    if isinstance(value, numbers.Rational) and not isinstance(value, numbers.Integral):
        return {
            "$type": "rational",
            "numerator": typed_value(value.numerator),
            "denominator": typed_value(value.denominator),
        }
    if isinstance(value, datetime):
        return {"$type": "datetime", "value": value.isoformat()}
    if isinstance(value, uuid.UUID):
        return {"$type": "uuid", "value": str(value)}
    if isinstance(value, float) and not math.isfinite(value):
        return {"$type": "float", "value": repr(value)}
    if isinstance(value, dict):
        return {k: typed_value(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [typed_value(v) for v in value]
    if hasattr(value, "isoformat"):
        return {"$type": type(value).__name__, "value": value.isoformat()}
    return value


def rows_fingerprint(table):
    # Physical Arrow buffers may contain unspecified bits in NULL slots or padding.
    # Hash typed logical scalars, including exact temporal integer units, instead.
    h = hashlib.sha256(table.schema.serialize().to_pybytes())
    for batch in table.to_batches(max_chunksize=8192):
        columns = logical_columns(batch)
        rows = zip(*columns) if columns else (() for _ in range(batch.num_rows))
        for row in rows:
            h.update(json.dumps(row, ensure_ascii=False, allow_nan=False, separators=(",", ":")).encode())
            h.update(b"\0")
    return h.hexdigest()


def logical_columns(table):
    """Bulk conversion with the same typed JSON values as arrow_value()."""
    import pyarrow as pa

    output = []
    for field, column in zip(table.schema, table.columns):
        t = field.type
        if (
            pa.types.is_timestamp(t)
            or pa.types.is_duration(t)
            or pa.types.is_time64(t)
            or pa.types.is_date64(t)
        ):
            values = column.cast(pa.int64()).to_pylist()
            name = str(t)
            values = [
                None if v is None else {"$type": "arrow-temporal", "arrow_type": name, "value": v}
                for v in values
            ]
        elif pa.types.is_time32(t) or pa.types.is_date32(t):
            values = column.cast(pa.int32()).to_pylist()
            name = str(t)
            values = [
                None if v is None else {"$type": "arrow-temporal", "arrow_type": name, "value": v}
                for v in values
            ]
        elif (
            pa.types.is_integer(t)
            or pa.types.is_boolean(t)
            or pa.types.is_string(t)
            or pa.types.is_large_string(t)
            or pa.types.is_null(t)
        ):
            values = column.to_pylist()
        elif pa.types.is_floating(t):
            values = [typed_value(v) for v in column.to_pylist()]
        else:
            # Nested temporal values, maps, decimals and extensions retain the
            # exact existing representation, including map order/duplicate keys.
            values = [arrow_value(v) for v in column]
        output.append(values)
    return output


def logical_rows(table):
    """Yield complete typed source records without per-cell primitive dispatch."""
    names = table.column_names
    for batch in table.to_batches(max_chunksize=8192):
        columns = logical_columns(batch)
        rows = zip(*columns) if columns else (() for _ in range(batch.num_rows))
        for row in rows:
            yield dict(zip(names, row))


def arrow_value(scalar):
    import pyarrow as pa

    if not scalar.is_valid:
        return None
    t = scalar.type
    if pa.types.is_timestamp(t) or pa.types.is_duration(t) or pa.types.is_time64(t) or pa.types.is_date64(t):
        return {"$type": "arrow-temporal", "arrow_type": str(t), "value": scalar.cast(pa.int64()).as_py()}
    if pa.types.is_time32(t) or pa.types.is_date32(t):
        return {"$type": "arrow-temporal", "arrow_type": str(t), "value": scalar.cast(pa.int32()).as_py()}
    if pa.types.is_list(t) or pa.types.is_large_list(t) or pa.types.is_fixed_size_list(t):
        return [arrow_value(x) for x in scalar.values]
    if pa.types.is_struct(t):
        return {f.name: arrow_value(scalar[i]) for i, f in enumerate(t)}
    if pa.types.is_map(t):
        return {"$type": "map", "entries": [[arrow_value(x[0]), arrow_value(x[1])] for x in scalar.values]}
    if pa.types.is_dictionary(t):
        return arrow_value(scalar.value)
    return typed_value(scalar.as_py())


def arrow_row(table, position):
    return {f.name: arrow_value(table.column(i)[position]) for i, f in enumerate(table.schema)}


def split_json_posts(body: bytes):
    """Keep each original JSON object substring, including unknown keys and number spelling."""
    text = body.decode("utf-8-sig")
    decoder = json.JSONDecoder(parse_float=decimal.Decimal)
    pos = 0
    while pos < len(text) and text[pos].isspace():
        pos += 1
    if pos == len(text) or text[pos] != "[":
        raise ValueError("API 响应应为 JSON 数组")
    pos += 1
    output = []
    while True:
        while pos < len(text) and text[pos].isspace():
            pos += 1
        if pos < len(text) and text[pos] == "]":
            if text[pos + 1 :].strip():
                raise ValueError("JSON 数组后有额外内容")
            return output
        start = pos
        value, pos = decoder.raw_decode(text, pos)
        if not isinstance(value, dict):
            raise ValueError("API 数组元素应为对象")
        output.append((text[start:pos], value))
        while pos < len(text) and text[pos].isspace():
            pos += 1
        if pos < len(text) and text[pos] == ",":
            pos += 1
            if text[pos:].lstrip().startswith("]"):
                raise ValueError("JSON 数组含尾随逗号")
        elif pos < len(text) and text[pos] == "]":
            continue
        else:
            raise ValueError("JSON 数组缺少分隔符")
