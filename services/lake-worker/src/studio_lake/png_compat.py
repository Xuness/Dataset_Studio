"""Bounded PNG metadata recovery without changing Pillow's process-wide guards.

Only known metadata checksums may be repaired in a decoder copy. Original chunk
bytes and checksum mismatches remain in provenance; pixel/animation chunks are
always strict. Large ICC profiles are decoded separately and restored to info.
"""

from dataclasses import dataclass, field
import io
from pathlib import Path
import struct
import zlib

from PIL import Image, PngImagePlugin

from .util import digest, typed_value

SIGNATURE = b"\x89PNG\r\n\x1a\n"
MAX_ICC_BYTES = 4 * 1024**2
MAX_METADATA_BYTES = 16 * 1024**2
MAX_CHUNKS = 100_000
MAX_RECOVERY_CHUNKS = 128
METADATA_CRC = {b"iCCP", b"eXIf", b"tEXt", b"zTXt", b"iTXt"}
CRITICAL = {b"IHDR", b"PLTE", b"IDAT", b"IEND"}
ANIMATION = {b"acTL", b"fcTL", b"fdAT"}


class PngCompatibilityError(ValueError):
    pass


@dataclass
class Inspection:
    width: int = 0
    height: int = 0
    frames: int = 1
    metadata_bytes: int = 0
    icc: bytes | None = None
    edits: list = field(default_factory=list)
    changes: list = field(default_factory=list)

    @property
    def memory_bytes(self):
        # Include both the retained original metadata and the restored ICC.
        return self.metadata_bytes + len(self.icc or b"")


def read_exact(source, count):
    data = source.read(count)
    if len(data) != count:
        raise PngCompatibilityError("PNG chunk is truncated")
    return data


def icc_profile(payload):
    name, separator, compressed = payload.partition(b"\0")
    if not separator or not 1 <= len(name) <= 79 or not compressed or compressed[0] != 0:
        raise PngCompatibilityError("PNG ICC header is invalid")
    try:
        decoder = zlib.decompressobj()
        profile = decoder.decompress(compressed[1:], MAX_ICC_BYTES + 1)
    except zlib.error:
        raise PngCompatibilityError("PNG ICC compression is invalid") from None
    if len(profile) > MAX_ICC_BYTES or decoder.unconsumed_tail:
        raise PngCompatibilityError("PNG ICC exceeds the 4 MiB decompression budget")
    if not decoder.eof or decoder.unused_data:
        raise PngCompatibilityError("PNG ICC compressed stream is incomplete or has trailing data")
    return profile


def scan(source, size):
    if source.read(8) != SIGNATURE:
        return None
    result = Inspection()
    offset, chunks = 8, 0
    seen_idat = seen_iend = seen_icc = seen_animation = False
    while offset < size:
        chunks += 1
        if chunks > MAX_CHUNKS:
            raise PngCompatibilityError("PNG exceeds the chunk-count budget")
        header = read_exact(source, 8)
        length, kind = struct.unpack(">I4s", header)
        end = offset + length + 12
        if length > 0x7FFFFFFF or end > size:
            raise PngCompatibilityError("PNG chunk extends beyond its file")
        if any(not (65 <= c <= 90 or 97 <= c <= 122) for c in kind) or kind[2] & 32:
            raise PngCompatibilityError("PNG chunk type is invalid")
        if not kind[0] & 32 and kind not in CRITICAL:
            raise PngCompatibilityError("PNG has an unknown critical chunk")
        if (offset == 8) != (kind == b"IHDR") or kind == b"IHDR" and length != 13:
            raise PngCompatibilityError("PNG IHDR is missing, duplicated or malformed")
        metadata = bool(kind[0] & 32) and kind not in ANIMATION
        if metadata:
            result.metadata_bytes += length + 12
            if result.metadata_bytes > MAX_METADATA_BYTES:
                raise PngCompatibilityError("PNG metadata exceeds the 16 MiB budget")
        payload = b""
        crc = zlib.crc32(kind)
        if metadata or kind in {b"IHDR", b"acTL"}:
            if kind == b"acTL" and length != 8:
                raise PngCompatibilityError("PNG animation control is malformed")
            payload = read_exact(source, length)
            crc = zlib.crc32(payload, crc)
        else:
            remaining = length
            while remaining:
                block = read_exact(source, min(remaining, 1024**2))
                crc = zlib.crc32(block, crc)
                remaining -= len(block)
        original_crc = read_exact(source, 4)
        expected, actual = struct.unpack(">I", original_crc)[0], crc & 0xFFFFFFFF
        actions = []
        replacement = None
        if expected != actual:
            if kind not in METADATA_CRC:
                raise PngCompatibilityError(f"PNG {kind.decode('ascii')} checksum mismatch")
            actions.append("metadata_crc_recomputed_for_decode")
            replacement = header + payload + struct.pack(">I", actual)
        if kind == b"IHDR":
            result.width, result.height = struct.unpack(">II", payload[:8])
            if not result.width or not result.height:
                raise PngCompatibilityError("PNG dimensions are invalid")
        elif kind == b"acTL":
            if seen_animation or seen_idat:
                raise PngCompatibilityError("PNG animation control is out of order")
            seen_animation = True
            result.frames = struct.unpack(">I", payload[:4])[0]
            if not 1 <= result.frames <= MAX_CHUNKS:
                raise PngCompatibilityError("PNG animation frame count is invalid")
        elif kind == b"IDAT":
            seen_idat = True
        elif kind == b"iCCP":
            if seen_icc:
                raise PngCompatibilityError("PNG has duplicate ICC profiles")
            seen_icc = True
            profile = icc_profile(payload)
            if len(profile) > PngImagePlugin.MAX_TEXT_CHUNK:
                result.icc = profile
                actions.append("icc_decoded_separately")
                replacement = b""
        elif kind == b"IEND":
            if length or end != size:
                raise PngCompatibilityError("PNG IEND is malformed or has trailing data")
            seen_iend = True
        if replacement is not None:
            if len(result.changes) >= MAX_RECOVERY_CHUNKS:
                raise PngCompatibilityError("PNG exceeds the metadata recovery budget")
            result.edits.append((offset, end, replacement))
            result.changes.append({"offset": offset, "type": kind.decode("ascii"),
                                   "actions": actions, "original_crc": expected, "computed_crc": actual,
                                   "original_chunk": typed_value(header + payload + original_crc)})
        offset = end
    if not seen_idat or not seen_iend:
        raise PngCompatibilityError("PNG pixel data or end marker is missing")
    return result


def inspect_path(path):
    path = Path(path)
    with path.open("rb") as source:
        return scan(source, path.stat().st_size)


def open_image(data):
    """Return a Pillow image and recovery evidence; caller owns/closes the image."""
    inspected = scan(io.BytesIO(data), len(data))
    recovery, decode_data = {}, data
    if inspected is not None and inspected.edits:
        parts, offset = [], 0
        view = memoryview(data)
        for start, end, replacement in inspected.edits:
            parts.extend((view[offset:start], replacement))
            offset = end
        parts.append(view[offset:])
        decode_data = b"".join(parts)
        recovery = {"source_decode_sha256": digest(decode_data), "source_png_compatibility": {
            "version": 1, "original_bytes_changed": False, "changes": inspected.changes,
            "icc_limit_bytes": MAX_ICC_BYTES, "metadata_limit_bytes": MAX_METADATA_BYTES}}
    image = Image.open(io.BytesIO(decode_data))
    if inspected is not None and inspected.icc is not None:
        image.info["icc_profile"] = inspected.icc
    return image, recovery
