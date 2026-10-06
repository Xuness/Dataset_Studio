import { useState } from "react";

const presets = [2048, 1536, 1024, 768];
export function imageInputLabel(edge?: number | null) {
  return edge == null ? "原尺寸" : `最长边 ${edge} px`;
}

export function ImageInputSettings({
  maxEdge,
  disabled,
  onChange,
}: {
  maxEdge?: number | null | undefined;
  disabled: boolean;
  onChange: (edge: number | undefined) => void;
}) {
  const [custom, setCustom] = useState(false);
  const selection =
    maxEdge == null
      ? "original"
      : custom || !presets.includes(maxEdge)
        ? "custom"
        : String(maxEdge);
  return (
    <>
      <label>
        API 图片最长边
        <select
          aria-label="API 图片最长边"
          disabled={disabled}
          value={selection}
          onChange={(e) => {
            const selected = e.target.value;
            setCustom(selected === "custom");
            onChange(
              selected === "original"
                ? undefined
                : selected === "custom"
                  ? (maxEdge ?? 1536)
                  : Number(selected),
            );
          }}
        >
          <option value="original">原尺寸</option>
          {presets.map((edge) => (
            <option key={edge} value={edge}>
              {edge} px
            </option>
          ))}
          <option value="custom">自定义</option>
        </select>
      </label>
      {selection === "custom" && (
        <label>
          自定义最长边（px）
          <input
            aria-label="自定义图片最长边（px）"
            required
            type="number"
            min={128}
            max={8192}
            step={1}
            disabled={disabled}
            value={maxEdge ?? 1536}
            onChange={(e) => onChange(Number(e.target.value))}
          />
        </label>
      )}
      <p className="aesthetic-help">
        等比例缩小，不放大、不裁切；已满足尺寸的图片保留原文件。缩小后的普通图片使用
        JPEG 95，透明图片使用无损 PNG。具体 Token 用量以调用记录为准。
      </p>
    </>
  );
}
