import type { ImagePolicy } from "./imagePolicy.js";
export function ImagePolicySummary({
  policy,
}: {
  policy: ImagePolicy | undefined | null;
}) {
  const e = policy?.encoding;
  if (policy?.profile !== "custom" || !e) return null;
  const properties: [string, string][] = [
    [
      "最长边",
      e.max_edge == null ? "保持原尺寸" : `${e.max_edge} 像素 · 不放大`,
    ],
    [
      "动画",
      e.animation === "preserve" ? "保留原文件，不缩放或转码" : "仅提取首帧",
    ],
    [
      "透明通道",
      e.alpha === "preserve"
        ? "保留"
        : e.alpha === "reject"
          ? "透明图片列入待检查"
          : `合成背景 ${e.background}`,
    ],
  ];
  if (e.format === "webp")
    properties.push(["WebP 编码力度", String(e.method ?? 6)]);
  if (e.format === "png")
    properties.push(["PNG 压缩级别", String(e.compress_level ?? 6)]);
  if (e.format === "jpeg")
    properties.push(
      ["JPEG 色度采样", e.subsampling === "420" ? "4:2:0" : "4:4:4"],
      ["JPEG 优化", e.optimize ? "启用" : "关闭"],
    );
  return (
    <dl className="wb-property-list">
      {properties.map(([label, value]) => (
        <div key={label} className="lake-property-pair">
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}
