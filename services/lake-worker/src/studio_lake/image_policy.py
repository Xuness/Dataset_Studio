"""Versioned, deterministic encoding recipes; legacy profiles keep their byte semantics."""

import hashlib
import io
import json
import re

from PIL import Image, ImageCms, ImageOps, __version__ as PILLOW_VERSION, features

from .ingest import prepare_image as prepare_legacy_image
from .png_compat import open_image
from .util import digest, typed_value


class ImagePolicyError(ValueError):
    pass


def integer(value, label, minimum, maximum):
    if isinstance(value, bool) or not isinstance(value, int) or not minimum <= value <= maximum:
        raise ImagePolicyError(f"{label} 必须是 {minimum}–{maximum} 的整数")
    return value


def boolean(value, label):
    if not isinstance(value, bool):
        raise ImagePolicyError(f"{label} 必须是布尔值")
    return value


def encoding(value):
    if not isinstance(value, dict) or value.get("version") != 1 or isinstance(value["version"], bool):
        raise ImagePolicyError("编码配置版本必须为 1")
    fmt = value.get("format")
    common = {"version", "format", "max_edge", "animation", "alpha", "background"}
    fields = {"webp": {"quality", "lossless", "method"}, "jpeg": {"quality", "optimize", "subsampling"}, "png": {"compress_level"}}
    if fmt not in fields or set(value) - common - fields[fmt]:
        raise ImagePolicyError("编码格式或参数不兼容")
    result = {"version": 1, "format": fmt, "max_edge": value.get("max_edge"),
              "animation": value.get("animation", "preserve"), "alpha": value.get("alpha", "preserve")}
    if result["max_edge"] is not None:
        integer(result["max_edge"], "最长边", 1, 32768)
    if result["animation"] not in {"preserve", "first_frame"}:
        raise ImagePolicyError("请选择保留动画原文件或明确提取首帧")
    if result["alpha"] not in {"preserve", "flatten", "reject"}:
        raise ImagePolicyError("透明通道策略无效")
    if fmt == "jpeg" and result["alpha"] == "preserve":
        raise ImagePolicyError("JPEG 需要选择透明背景合成或拒绝透明图片")
    if result["alpha"] == "flatten":
        background = value.get("background")
        if not isinstance(background, str) or not re.fullmatch(r"#[0-9a-fA-F]{6}", background):
            raise ImagePolicyError("背景颜色需为 #RRGGBB")
        result["background"] = background.upper()
    elif value.get("background") is not None:
        raise ImagePolicyError("仅背景合成策略接受背景颜色")
    if fmt == "webp":
        result["lossless"] = boolean(value.get("lossless", False), "无损编码")
        result["method"] = integer(value.get("method", 6), "WebP 编码力度", 0, 6)
        if result["lossless"]:
            if value.get("quality") is not None:
                raise ImagePolicyError("无损 WebP 不使用有损质量参数")
        else:
            result["quality"] = integer(value.get("quality", 95), "质量", 1, 100)
    elif fmt == "jpeg":
        result["quality"] = integer(value.get("quality", 95), "质量", 1, 100)
        result["optimize"] = boolean(value.get("optimize", True), "JPEG 优化")
        result["subsampling"] = value.get("subsampling", "444")
        if result["subsampling"] not in {"444", "420"}:
            raise ImagePolicyError("JPEG 色度采样必须为 444 或 420")
    else:
        result["compress_level"] = integer(value.get("compress_level", 6), "PNG 压缩级别", 0, 9)
    return result


def media_policy(value):
    if not isinstance(value, dict) or set(value) - {"profile", "allow_sample", "existing", "encoding"}:
        raise ImagePolicyError("保存策略字段无效")
    result = dict(value)
    profile = result.get("profile")
    if profile not in {"metadata_only", "original", "webp-2048-q95", "custom"}:
        raise ImagePolicyError("请明确选择保存策略")
    boolean(result.setdefault("allow_sample", False), "备用图片")
    if result.setdefault("existing", "keep") not in {"keep", "match_profile"}:
        raise ImagePolicyError("已有图片策略无效")
    if profile == "original" and result["allow_sample"]:
        raise ImagePolicyError("原图策略不能使用备用图片")
    if profile == "custom":
        result["encoding"] = encoding(result.get("encoding"))
    elif "encoding" in result:
        raise ImagePolicyError("仅自定义转码策略接受编码参数")
    return result


def profile_id(policy):
    if policy["profile"] != "custom":
        return policy["profile"]
    payload = json.dumps(encoding(policy["encoding"]), sort_keys=True, separators=(",", ":"))
    return "studio-image-v1:" + hashlib.sha256(payload.encode()).hexdigest()


def prepare_image(data, policy):
    policy = media_policy(policy)
    if policy["profile"] != "custom":
        return prepare_legacy_image(data, policy["profile"])
    recipe = policy["encoding"]
    details = {"download_sha256": digest(data), "download_bytes": len(data),
               "storage_profile": profile_id(policy), "encoding": recipe, "processing_version": 5,
               "pillow_version": PILLOW_VERSION}
    source, recovery = open_image(data)
    details.update(recovery)
    with source:
        frames = getattr(source, "n_frames", 1)
        fmt = (source.format or "bin").lower()
        details.update(source_width=source.width, source_height=source.height, frames=frames,
                       source_format=fmt, source_image_info=typed_value(dict(source.info)))
        if frames > 1 and recipe["animation"] == "preserve":
            details.update(stored_width=source.width, stored_height=source.height,
                           animation_preserved=True, encoding_applied=False)
            return data, {"jpeg": "jpg", "tiff": "tif"}.get(fmt, fmt), details
        source.seek(0)
        im = ImageOps.exif_transpose(source)
        im.load()
        details["source_image_info"] = typed_value(dict(source.info))
        alpha = im.mode in {"RGBA", "RGBa", "LA", "La", "PA"} or "transparency" in im.info
        rgba = im.convert("RGBA") if alpha else None
        transparent = rgba is not None and rgba.getchannel("A").getextrema()[0] < 255
        if transparent and recipe["alpha"] == "reject":
            raise ImagePolicyError("transparent_image_rejected")
        icc = im.info.get("icc_profile")
        if icc and im.mode not in {"RGB", "RGBA", "P"}:
            profile = ImageCms.ImageCmsProfile(io.BytesIO(icc))
            if profile.profile.xcolor_space.strip() == "RGB" and im.mode in {"1", "L", "LA"}:
                # Some grayscale JPEG/PNG files embed an RGB profile. Expand the
                # channels before using that profile; L -> RGB CMS is invalid.
                im = rgba if rgba is not None else im.convert("RGB")
                details["color_profile_action"] = "expanded_grayscale_to_rgb"
            else:
                target = ImageCms.createProfile("sRGB")
                im = ImageCms.profileToProfile(im.convert("L") if im.mode == "LA" else im,
                                              profile, target, outputMode="RGB")
                if rgba is not None:
                    im.putalpha(rgba.getchannel("A"))
                icc = ImageCms.ImageCmsProfile(target).tobytes()
        elif rgba is not None:
            im = rgba
        else:
            im = im.convert("RGB")
        if alpha and recipe["alpha"] == "flatten":
            background = Image.new("RGBA", im.size, recipe["background"])
            im = Image.alpha_composite(background, im.convert("RGBA")).convert("RGB")
        elif recipe["format"] == "jpeg":
            im = im.convert("RGB")
        if recipe["max_edge"] is not None:
            im.thumbnail((recipe["max_edge"], recipe["max_edge"]), Image.Resampling.LANCZOS)
        options = {"icc_profile": icc} if icc else {}
        fmt = recipe["format"]
        if fmt == "webp":
            options.update(lossless=recipe["lossless"], quality=recipe.get("quality", 100), method=recipe["method"], exact=True)
        elif fmt == "jpeg":
            options.update(quality=recipe["quality"], optimize=recipe["optimize"], subsampling=0 if recipe["subsampling"] == "444" else 2)
        else:
            options["compress_level"] = recipe["compress_level"]
        output = io.BytesIO()
        im.save(output, format=fmt.upper(), **options)
        details.update(stored_width=im.width, stored_height=im.height, animation_preserved=False,
                       encoding_applied=True, frame_selected=0 if frames > 1 else None,
                       codec_version=features.version("webp" if fmt == "webp" else "jpg" if fmt == "jpeg" else "zlib"))
        return output.getvalue(), "jpg" if fmt == "jpeg" else fmt, details
