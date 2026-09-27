import type { Schema } from "@studio/contracts";

export type Encoding = Schema["LakeImageEncoding"];
export type ImagePolicy = Schema["LakeImagePolicy"];
export type ImageFields = {
  profile: "" | Schema["LakeImageProfile"];
  encoding: Encoding;
  existing: "keep" | "match_profile";
  allowSample: boolean;
};
export const defaultEncoding: Encoding = {
  version: 1,
  format: "webp",
  max_edge: 2048,
  quality: 95,
  lossless: false,
  method: 6,
  animation: "preserve",
  alpha: "preserve",
};
function integer(
  n: number | undefined,
  label: string,
  min: number,
  max: number,
) {
  if (!Number.isInteger(n) || n! < min || n! > max)
    throw new Error(`${label}需为 ${min} 至 ${max} 的整数`);
  return n!;
}
export function normalizedEncoding(e: Encoding): Encoding {
  if (e.version !== 1 || !["webp", "jpeg", "png"].includes(e.format))
    throw new Error("编码配置版本或格式无效");
  if (
    !["preserve", "first_frame"].includes(e.animation) ||
    !["preserve", "flatten", "reject"].includes(e.alpha)
  )
    throw new Error("动画或透明通道策略无效");
  const result: Encoding = {
    version: 1,
    format: e.format,
    max_edge:
      e.max_edge == null ? null : integer(e.max_edge, "最长边", 1, 32768),
    animation: e.animation,
    alpha: e.alpha,
  };
  if (e.alpha === "flatten") {
    if (!/^#[0-9a-fA-F]{6}$/.test(e.background ?? ""))
      throw new Error("请选择有效的背景颜色");
    result.background = e.background!.toUpperCase();
  }
  if (e.format === "webp") {
    result.lossless = e.lossless ?? false;
    result.method = integer(e.method ?? 6, "编码力度", 0, 6);
    if (!result.lossless)
      result.quality = integer(e.quality ?? 95, "质量", 1, 100);
  } else if (e.format === "jpeg") {
    if (e.alpha === "preserve")
      throw new Error("JPEG 请选择合成背景或拒绝透明图片");
    result.quality = integer(e.quality ?? 95, "质量", 1, 100);
    result.optimize = e.optimize ?? true;
    result.subsampling = e.subsampling ?? "444";
    if (!["444", "420"].includes(result.subsampling))
      throw new Error("JPEG 色度采样无效");
  } else
    result.compress_level = integer(
      e.compress_level ?? 6,
      "PNG 压缩级别",
      0,
      9,
    );
  return result;
}
export function imagePolicy(d: ImageFields): ImagePolicy {
  if (!d.profile) throw new Error("请明确选择图片保存策略");
  return {
    profile: d.profile,
    existing: d.existing,
    allow_sample:
      !["original", "metadata_only"].includes(d.profile) && d.allowSample,
    ...(d.profile === "custom"
      ? { encoding: normalizedEncoding(d.encoding) }
      : {}),
  };
}
export function fieldsForPolicy(p: ImagePolicy): ImageFields {
  return {
    profile: p.profile,
    existing: p.existing === "match_profile" ? "match_profile" : "keep",
    allowSample: p.allow_sample ?? false,
    encoding: p.encoding ?? defaultEncoding,
  };
}
export function encodingForFormat(
  e: Encoding,
  format: Encoding["format"],
): Encoding {
  return normalizedEncoding({
    ...defaultEncoding,
    format,
    max_edge: e.max_edge ?? null,
    animation: e.animation,
    alpha: format === "jpeg" && e.alpha === "preserve" ? "flatten" : e.alpha,
    background: e.background ?? "#FFFFFF",
    quality: e.quality ?? 95,
  });
}
export function imagePolicyLabel(p: ImagePolicy | null | undefined) {
  if (!p || p.profile === "metadata_only") return "仅元数据";
  if (p.profile === "original") return "原图";
  if (p.profile === "webp-2048-q95") return "WebP · 2048 / Q95";
  const e = p.encoding;
  if (!e) return "自定义 · 配置缺失";
  return `${e.format.toUpperCase()} · ${e.max_edge ?? "原尺寸"} / ${e.format === "png" || e.lossless ? "无损编码" : `Q${e.quality}`}`;
}
export type ImagePresets = {
  items: { id: string; name: string; media: ImagePolicy }[];
};
export const initialPresets: ImagePresets = { items: [] };
export function decodePresets(value: unknown): ImagePresets | null {
  try {
    const v = value as ImagePresets;
    if (!Array.isArray(v?.items) || v.items.length > 64) return null;
    const ids = new Set<string>();
    for (const item of v.items) {
      if (
        typeof item.id !== "string" ||
        ids.has(item.id) ||
        !item.id ||
        typeof item.name !== "string" ||
        !item.name.trim() ||
        item.name.length > 80
      )
        return null;
      if (
        !["metadata_only", "original", "webp-2048-q95", "custom"].includes(
          item.media.profile,
        )
      )
        return null;
      if (item.media.profile === "custom")
        normalizedEncoding(item.media.encoding!);
      ids.add(item.id);
    }
    return v;
  } catch {
    return null;
  }
}
