"""Sequential copies and a single forward pack/object integrity pass."""

from contextlib import contextmanager
import hashlib
import io
import os
from pathlib import Path
import tarfile

import pyarrow.parquet as pq

from .util import IntegrityError, contained, file_hash, sync_directory

CHUNK = 8 * 1024**2


class WindowsAlignedReader(io.RawIOBase):
    """Read-only FILE_FLAG_NO_BUFFERING, using an aligned 8 MiB native buffer."""

    def __init__(self, path):
        import ctypes as c
        from ctypes import wintypes as w

        super().__init__()
        self.c, self.w = c, w
        self.k = k = c.WinDLL("kernel32", use_last_error=True)
        k.CreateFileW.argtypes = [w.LPCWSTR, w.DWORD, w.DWORD, c.c_void_p, w.DWORD, w.DWORD, w.HANDLE]
        k.CreateFileW.restype = w.HANDLE
        k.ReadFile.argtypes = [w.HANDLE, c.c_void_p, w.DWORD, c.POINTER(w.DWORD), c.c_void_p]
        k.ReadFile.restype = w.BOOL
        k.CloseHandle.argtypes = [w.HANDLE]
        k.VirtualAlloc.argtypes = [c.c_void_p, c.c_size_t, w.DWORD, w.DWORD]
        k.VirtualAlloc.restype = c.c_void_p
        k.VirtualFree.argtypes = [c.c_void_p, c.c_size_t, w.DWORD]
        self.handle, self.buffer = None, None
        self.handle = k.CreateFileW(str(Path(path).resolve()), 0x80000000, 1, None, 3, 0x28000000, None)
        if self.handle == c.c_void_p(-1).value:
            self.handle = None
            raise c.WinError(c.get_last_error())
        self.buffer = k.VirtualAlloc(None, CHUNK, 0x3000, 0x04)
        if not self.buffer:
            self.close()
            raise c.WinError(c.get_last_error())
        self.eof = False

    def readable(self):
        return True

    def readinto(self, target):
        if self.eof:
            return 0
        size = min(len(target), CHUNK) // 4096 * 4096
        if not size:
            raise ValueError("Use WindowsAlignedReader through an 8 MiB BufferedReader")
        done = self.w.DWORD()
        if not self.k.ReadFile(self.handle, self.buffer, size, self.c.byref(done), None):
            raise self.c.WinError(self.c.get_last_error())
        n = done.value
        target[:n] = memoryview((self.c.c_ubyte * n).from_address(self.buffer)).cast("B")
        self.eof = n < size
        return n

    def close(self):
        if getattr(self, "handle", None) is not None:
            self.k.CloseHandle(self.handle)
            self.handle = None
        if getattr(self, "buffer", None):
            self.k.VirtualFree(self.buffer, 0, 0x8000)
            self.buffer = None
        super().close()


@contextmanager
def sequential_reader(path, uncached=False):
    if uncached:
        if os.name != "nt":
            raise ValueError("Uncached reads currently require Windows")
        stream = io.BufferedReader(WindowsAlignedReader(path), buffer_size=CHUNK)
    else:
        stream = Path(path).open("rb", buffering=CHUNK)
    with stream:
        yield stream


def copy_verified(source, target, expected=None, *, uncached=False):
    """Hash during copy, flush, then rename; caller schedules destination read-back."""
    source, target = Path(source), Path(target)
    target.parent.mkdir(parents=True, exist_ok=True)
    partial = target.with_name(target.name + ".partial")
    h, size = hashlib.sha256(), 0
    with sequential_reader(source, uncached) as src, partial.open("wb", buffering=CHUNK) as dst:
        while block := src.read(CHUNK):
            if dst.write(block) != len(block):
                raise OSError("short copy write")
            h.update(block)
            size += len(block)
        dst.flush()
        os.fsync(dst.fileno())
    info = {"bytes": size, "sha256": h.hexdigest()}
    if expected is not None and info != expected:
        raise IntegrityError(f"Source copy hash/size mismatch: {source}")
    os.replace(partial, target)
    sync_directory(target.parent)
    return info


def verify_pack(path, objects, expected, *, uncached=False):
    pack_hash, position, count = hashlib.sha256(), 0, 0
    ordered = sorted(objects, key=lambda r: r["offset"])
    with sequential_reader(path, uncached) as stream:

        def consume(length, digest=None):
            nonlocal position
            if length < 0:
                raise IntegrityError("Overlapping or invalid object offsets")
            while length:
                block = stream.read(min(CHUNK, length))
                if not block:
                    raise IntegrityError("Truncated image pack")
                pack_hash.update(block)
                if digest is not None:
                    digest.update(block)
                position += len(block)
                length -= len(block)

        for row in ordered:
            if row["offset"] % 512 or row["length"] <= 0:
                raise IntegrityError("Invalid object range")
            consume(row["offset"] - 512 - position)
            header = stream.read(512)
            if len(header) != 512:
                raise IntegrityError("Truncated TAR header")
            pack_hash.update(header)
            position += 512
            try:
                member = tarfile.TarInfo.frombuf(header, "utf-8", "strict")
            except (tarfile.TarError, ValueError) as e:
                raise IntegrityError("Invalid TAR header") from e
            if not member.isfile() or member.name != row["member_name"] or member.size != row["length"]:
                raise IntegrityError("Object catalog and TAR header disagree")
            h = hashlib.sha256()
            consume(row["length"], h)
            if h.hexdigest() != row["sha256"] or member.name != f"{row['sha256']}.{row['stored_ext']}":
                raise IntegrityError("Object SHA-256/name mismatch")
            count += 1
        while block := stream.read(CHUNK):
            pack_hash.update(block)
            position += len(block)
    if {"bytes": position, "sha256": pack_hash.hexdigest()} != expected:
        raise IntegrityError(f"Pack SHA-256/size mismatch: {path}")
    return {"objects": count, "bytes": position}


def verify_segment(directory, manifest, *, uncached=False):
    result = {"objects": 0, "bytes": 0}
    for name, expected in manifest["files"].items():
        path = contained(directory, name)
        if not path.is_file() or path.stat().st_size != expected["bytes"]:
            raise IntegrityError(f"Segment file size mismatch: {path}")
        if name != "images.tar" and file_hash(path) != expected["sha256"]:
            raise IntegrityError(f"Segment file SHA-256 mismatch: {path}")
    objects = pq.ParquetFile(directory / "objects.parquet").read().to_pylist()
    if len(objects) != manifest["counts"]["objects"]:
        raise IntegrityError("Object count mismatch")
    if "images.tar" in manifest["files"]:
        result = verify_pack(
            directory / "images.tar", objects, manifest["files"]["images.tar"], uncached=uncached
        )
    elif objects:
        raise IntegrityError("Objects without an image pack")
    return result
