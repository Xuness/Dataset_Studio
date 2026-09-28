"""Per-image disk footprints, separate from the decoded-memory admission budget."""

from dataclasses import dataclass

# Receipts, TAR headers and the small per-batch metadata files also need room.
OVERHEAD = 64 * 1024
DOWNLOAD_CHUNK = 4 * 1024**2
INITIAL_DOWNLOAD = 8 * 1024**2


@dataclass(frozen=True)
class StagingPlan:
    download_bytes: int
    output_bytes: int | None
    may_preserve_original: bool = True

    def peak(self, download_bytes=None):
        source = self.download_bytes if download_bytes is None else download_bytes
        output = source if self.output_bytes is None else self.output_bytes
        if self.may_preserve_original:
            output = max(source, output)
        # During encoding the original and output coexist. During publication a
        # verified output and its TAR copy coexist; these are different stages.
        return max(source + output, output * 2) + OVERHEAD


def estimate(policy, record, resources, *, source_bytes=None, may_preserve_original=True):
    maximum = resources.max_download_bytes
    hint = record.get("file_size")
    source = source_bytes if source_bytes is not None else (
        min(hint, maximum) if type(hint) is int and hint > 0 else min(maximum, INITIAL_DOWNLOAD)
    )
    if policy["profile"] not in {"custom", "webp-2048-q95"}:
        return StagingPlan(source, None)
    width = record.get("image_width", record.get("width"))
    height = record.get("image_height", record.get("height"))
    pixels = width * height if type(width) is int and type(height) is int and min(width, height) > 0 else None
    edge = (policy.get("encoding") or {}).get("max_edge") if policy["profile"] == "custom" else 2048
    # Metadata is only an estimate. The actual header and encoded bytes are checked
    # again before writing; a misleading size never authorizes unbounded disk use.
    pixels = min(pixels or resources.config["max_image_pixels"], resources.config["max_image_pixels"])
    if edge:
        pixels = min(pixels, edge * edge)
    return StagingPlan(source, pixels * 8, may_preserve_original)
