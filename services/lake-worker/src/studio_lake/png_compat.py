"""Bounded PNG metadata recovery without changing Pillow's process-wide guards.

Known metadata recoveries apply only to a decoder copy. Original chunks and
bytes after IEND remain in provenance (inline when small, otherwise by length and
SHA-256); pixel, animation and colour-affecting chunks are always strict. Large or
recovered complete ICC profiles are restored to the individual image.
"""

from dataclasses import dataclass, field
import io
import hashlib
from pathlib import Path
import struct
import zlib

from PIL import Image, ImageCms, PngImagePlugin

from .util import digest, typed_value

SIGNATURE = b"\x89PNG\r\n\x1a\n"
MAX_ICC_BYTES = 4 * 1024**2
MAX_METADATA_BYTES = 16 * 1024**2
MAX_CHUNKS = 100_000
MAX_RECOVERY_CHUNKS = 128
# Larger removed bytes keep offset, length and SHA-256; the MD5-verified source is the original.
MAX_INLINE_EVIDENCE = 64 * 1024
METADATA_CRC = {b"iCCP", b"eXIf", b"tEXt", b"zTXt", b"iTXt"}
CRITICAL = {b"IHDR", b"PLTE", b"IDAT", b"IEND"}
ANIMATION = {b"acTL", b"fcTL", b"fdAT"}
# Ancillary chunks that change decoded pixels or colour are never removed for decoding.
RENDERING = {b"tRNS", b"gAMA", b"cHRM", b"sRGB", b"iCCP", b"sBIT", b"cICP", b"mDCv", b"cLLi"}


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


class Crc:
    def __init__(self, value):
        self.value = value

    def update(self, block):
        self.value = zlib.crc32(block, self.value)


def stream(source, count, *hashes):
    while count:
        block = read_exact(source, min(count, 1024**2))
        for h in hashes:
            h.update(block)
        count -= len(block)


def chunk_evidence(raw=None, size=None, sha256=None):
    """Inline small removed chunks; larger ones are identified by length and SHA-256."""
    if raw is not None and len(raw) <= MAX_INLINE_EVIDENCE:
        return {"original_chunk": typed_value(raw)}
    return {"original_chunk_bytes": len(raw) if raw is not None else size,
            "original_chunk_sha256": sha256 or digest(raw)}


def complete_icc(profile):
    """Require a complete declared profile before tolerating an unfinished wrapper."""
    if (len(profile) < 132 or struct.unpack(">I", profile[:4])[0] != len(profile)
            or profile[36:40] != b"acsp"):
        raise PngCompatibilityError("PNG incomplete ICC stream has an incomplete profile")
    count = struct.unpack(">I", profile[128:132])[0]
    table_end = 132 + count * 12
    if not count or table_end > len(profile):
        raise PngCompatibilityError("PNG incomplete ICC stream has an invalid tag table")
    for i in range(count):
        offset, size = struct.unpack(">II", profile[136 + i * 12:144 + i * 12])
        if offset < table_end or offset % 4 or size < 8 or offset + size > len(profile):
            raise PngCompatibilityError("PNG incomplete ICC stream has an incomplete tag")
    try:
        ImageCms.ImageCmsProfile(io.BytesIO(profile))
    except (OSError, ValueError, ImageCms.PyCMSError):
        raise PngCompatibilityError("PNG incomplete ICC stream has an unreadable profile") from None


def icc_profile(payload, checksum_valid):
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
    if decoder.unused_data:
        raise PngCompatibilityError("PNG ICC compressed stream has trailing data")
    incomplete = not decoder.eof
    if incomplete:
        # A complete profile may precede a missing compression end marker. Do not
        # combine this recovery with a damaged enclosing chunk checksum.
        if not checksum_valid:
            raise PngCompatibilityError("PNG incomplete ICC stream has an invalid chunk checksum")
        complete_icc(profile)
    return profile, incomplete


def edit(result, start, end, replacement, evidence):
    if len(result.changes) >= MAX_RECOVERY_CHUNKS:
        raise PngCompatibilityError("PNG exceeds the metadata recovery budget")
    result.edits.append((start, end, replacement))
    result.changes.append({"offset": start, **evidence})


def scan(source, size):
    if source.read(8) != SIGNATURE:
        return None
    result = Inspection()
    offset, chunks = 8, 0
    seen_idat = seen_iend = seen_icc = seen_animation = False
    retained_icc = None
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
        ancillary = bool(kind[0] & 32) and kind not in ANIMATION
        opaque = ancillary and kind not in RENDERING
        # Opaque chunks beyond the metadata budget are hashed, not held or decoded.
        oversized = opaque and result.metadata_bytes + length + 12 > MAX_METADATA_BYTES
        duplicate_icc = kind == b"iCCP" and seen_icc
        if ancillary and not oversized:
            result.metadata_bytes += length + 12
            if result.metadata_bytes > MAX_METADATA_BYTES:
                raise PngCompatibilityError("PNG metadata exceeds the 16 MiB budget")
        payload, whole = b"", None
        crc = zlib.crc32(kind)
        if (ancillary and not oversized) or kind in {b"IHDR", b"acTL"}:
            if kind == b"acTL" and length != 8:
                raise PngCompatibilityError("PNG animation control is malformed")
            payload = read_exact(source, length)
            crc = zlib.crc32(payload, crc)
        else:
            running = Crc(crc)
            whole = hashlib.sha256(header) if oversized else None
            stream(source, length, running, *([whole] if whole else []))
            crc = running.value
        original_crc = read_exact(source, 4)
        expected, actual = struct.unpack(">I", original_crc)[0], crc & 0xFFFFFFFF
        actions, extra = [], {}
        replacement = None
        if oversized:
            whole.update(original_crc)
            actions.append("oversized_opaque_chunk_removed_for_decode")
            replacement = b""
        elif duplicate_icc:
            # PNG allows one iCCP. The first is retained, as libpng does; later ones are evidence.
            actions.append("duplicate_icc_removed_for_decode")
            replacement = b""
            try:
                duplicate, _ = icc_profile(payload, expected == actual)
                extra = {"profile_sha256": digest(duplicate),
                         "matches_retained_profile": duplicate == retained_icc}
            except PngCompatibilityError as error:
                extra = {"profile_error": str(error)}
        elif expected != actual:
            if kind in METADATA_CRC:
                actions.append("metadata_crc_recomputed_for_decode")
                replacement = header + payload + struct.pack(">I", actual)
            elif opaque:
                actions.append("opaque_chunk_crc_mismatch_removed_for_decode")
                replacement = b""
            else:
                raise PngCompatibilityError(f"PNG {kind.decode('ascii')} checksum mismatch")
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
        elif kind == b"iCCP" and not duplicate_icc:
            seen_icc = True
            retained_icc, incomplete = icc_profile(payload, expected == actual)
            if incomplete:
                actions.append("incomplete_icc_stream_complete_profile_retained")
            if incomplete or len(retained_icc) > PngImagePlugin.MAX_TEXT_CHUNK:
                result.icc = retained_icc
                actions.append("icc_decoded_separately")
                replacement = b""
        elif kind == b"IEND":
            if length:
                raise PngCompatibilityError("PNG IEND is malformed")
            seen_iend = True
        if replacement is not None:
            original = (chunk_evidence(size=length + 12, sha256=whole.hexdigest()) if oversized
                        else chunk_evidence(header + payload + original_crc))
            edit(result, offset, end, replacement, {"type": kind.decode("ascii"),
                 "actions": actions, "original_crc": expected, "computed_crc": actual,
                 **original, **extra})
        offset = end
        if seen_iend:
            break
    if not seen_idat or not seen_iend:
        raise PngCompatibilityError("PNG pixel data or end marker is missing")
    if offset < size:
        # Bytes after IEND are opaque and never decoded; the download size limit bounds them.
        count = size - offset
        if count <= MAX_INLINE_EVIDENCE:
            trailing = read_exact(source, count)
            sha256, original = digest(trailing), {"original_bytes": typed_value(trailing)}
        else:
            hashed = hashlib.sha256()
            stream(source, count, hashed)
            sha256, original = hashed.hexdigest(), {}
        edit(result, offset, size, b"", {"type": "trailing_data",
             "actions": ["trailing_data_retained_outside_decode"], "bytes": count,
             "sha256": sha256, **original})
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
            "version": 3, "original_bytes_changed": False, "changes": inspected.changes,
            "icc_limit_bytes": MAX_ICC_BYTES, "metadata_limit_bytes": MAX_METADATA_BYTES,
            "inline_evidence_limit_bytes": MAX_INLINE_EVIDENCE}}
    image = Image.open(io.BytesIO(decode_data))
    if inspected is not None and inspected.icc is not None:
        image.info["icc_profile"] = inspected.icc
    return image, recovery
