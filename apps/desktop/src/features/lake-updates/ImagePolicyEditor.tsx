import { useState } from "react";
import type { StudioClient } from "@studio/client";
import { Button, DraftStatus, ErrorDetails } from "@studio/ui";
import { useLakePreference } from "./queries.js";
import { ImagePolicySummary } from "./ImagePolicySummary.js";
import {
  decodePresets,
  defaultEncoding,
  encodingForFormat,
  fieldsForPolicy,
  imagePolicy,
  imagePolicyLabel,
  initialPresets,
} from "./imagePolicy.js";
import type { Encoding, ImageFields } from "./imagePolicy.js";

export function ImagePolicyEditor({
  client,
  value: d,
  onChange,
  collection = false,
}: {
  client: StudioClient;
  value: ImageFields;
  onChange: (patch: Partial<ImageFields>) => void;
  collection?: boolean;
}) {
  const presets = useLakePreference(
    client,
    "studio.lake-updates.image-presets",
    initialPresets,
    decodePresets,
  );
  const [name, setName] = useState(""),
    [editing, setEditing] = useState("");
  const [error, setError] = useState<unknown>(null),
    [notice, setNotice] = useState("");
  const e = d.encoding ?? defaultEncoding;
  const selectedPreset = presets.value.items.find((p) => p.id === editing);
  let matchesPreset = false;
  if (selectedPreset) {
    try {
      matchesPreset =
        JSON.stringify(imagePolicy(d)) ===
        JSON.stringify(imagePolicy(fieldsForPolicy(selectedPreset.media)));
    } catch {
      // Incomplete edits still allow restoring the saved preset.
    }
  }
  function applyPreset(id: string) {
    const preset = presets.value.items.find((p) => p.id === id);
    setEditing(id);
    setName(preset?.name ?? "");
    setError(null);
    setNotice("");
    if (preset) onChange(fieldsForPolicy(preset.media));
  }
  const update = (patch: Partial<Encoding>) =>
    onChange({ encoding: { ...e, ...patch } });
  async function save(overwrite: boolean) {
    setError(null);
    setNotice("");
    try {
      if (!name.trim() || name.trim().length > 80)
        throw new Error("预设名称需为 1–80 个字符");
      if (!overwrite && presets.value.items.length >= 64)
        throw new Error("最多保存 64 个预设");
      if (
        presets.value.items.some(
          (p) => p.name === name.trim() && (!overwrite || p.id !== editing),
        )
      )
        throw new Error("已有同名预设");
      const item = {
        id: overwrite ? editing : crypto.randomUUID(),
        name: name.trim(),
        media: imagePolicy(d),
      };
      presets.controller.set({
        items: overwrite
          ? presets.value.items.map((p) => (p.id === editing ? item : p))
          : [...presets.value.items, item],
      });
      await presets.controller.flush();
      setEditing(item.id);
      setNotice("预设已保存；已创建任务仍使用各自固定的配置。");
    } catch (error) {
      setError(error);
    }
  }
  return (
    <>
      <div className="lake-fields">
        <label>
          保存策略
          <select
            aria-label="保存策略"
            value={d.profile}
            onChange={(event) => {
              const id = event.target.value;
              const preset = presets.value.items.find(
                (p) => `preset:${p.id}` === id,
              );
              if (preset) applyPreset(preset.id);
              else {
                setNotice("");
                onChange({
                  profile: id as ImageFields["profile"],
                  allowSample: false,
                });
              }
            }}
          >
            <option value="">请选择策略</option>
            <option value="metadata_only">仅元数据</option>
            <option value="original">保存原图</option>
            <option value="webp-2048-q95">WebP · 最长边 2048 / 质量 95</option>
            <option value="custom">自定义图片编码</option>
            {presets.value.items.length > 0 && (
              <optgroup label="我的预设">
                {presets.value.items.map((p) => (
                  <option key={p.id} value={`preset:${p.id}`}>
                    {p.name}
                  </option>
                ))}
              </optgroup>
            )}
          </select>
        </label>
        {selectedPreset && (
          <div className="lake-preset-status">
            <p role="status">
              预设「{selectedPreset.name}」 ·{" "}
              {matchesPreset
                ? "当前设置与预设一致"
                : "当前设置已修改，预设保持不变"}
            </p>
            <Button onClick={() => applyPreset(selectedPreset.id)}>
              应用预设
            </Button>
          </div>
        )}
        {d.profile === "custom" && (
          <>
            <label>
              编码格式
              <select
                aria-label="编码格式"
                value={e.format}
                onChange={(ev) =>
                  onChange({
                    encoding: encodingForFormat(
                      e,
                      ev.target.value as Encoding["format"],
                    ),
                  })
                }
              >
                <option value="webp">WebP</option>
                <option value="jpeg">JPEG</option>
                <option value="png">PNG（无损编码）</option>
              </select>
            </label>
            <label>
              尺寸
              <select
                aria-label="尺寸"
                value={e.max_edge == null ? "original" : "limit"}
                onChange={(ev) =>
                  update({
                    max_edge: ev.target.value === "original" ? null : 2048,
                  })
                }
              >
                <option value="original">保持原尺寸</option>
                <option value="limit">限制最长边</option>
              </select>
            </label>
            {e.max_edge != null && (
              <label>
                最长边（像素）
                <input
                  aria-label="最长边（像素）"
                  type="number"
                  min={1}
                  max={32768}
                  value={e.max_edge}
                  onChange={(ev) =>
                    update({ max_edge: Number(ev.target.value) })
                  }
                />
              </label>
            )}
            <p className="lake-hint">
              等比例缩小，不放大小图。无损编码仍会保留你明确选择的尺寸或首帧处理。
            </p>
            {e.format === "webp" && (
              <label className="lake-check">
                <input
                  type="checkbox"
                  checked={e.lossless ?? false}
                  onChange={(ev) => update({ lossless: ev.target.checked })}
                />
                无损 WebP
              </label>
            )}
            {(e.format === "jpeg" || (e.format === "webp" && !e.lossless)) && (
              <label>
                编码质量（1–100）
                <input
                  aria-label="编码质量"
                  type="number"
                  min={1}
                  max={100}
                  value={e.quality ?? 95}
                  onChange={(ev) =>
                    update({ quality: Number(ev.target.value) })
                  }
                />
              </label>
            )}
            <label>
              动画与多帧图片
              <select
                aria-label="动画与多帧图片"
                value={e.animation}
                onChange={(ev) =>
                  update({
                    animation: ev.target.value as Encoding["animation"],
                  })
                }
              >
                <option value="preserve">保留原文件（不缩放或转码）</option>
                <option value="first_frame">仅提取首帧并按当前设置转码</option>
              </select>
            </label>
            <label>
              透明通道
              <select
                aria-label="透明通道"
                value={e.alpha}
                onChange={(ev) =>
                  update({
                    alpha: ev.target.value as Encoding["alpha"],
                    background: e.background ?? "#FFFFFF",
                  })
                }
              >
                {e.format !== "jpeg" && (
                  <option value="preserve">保留透明通道</option>
                )}
                <option value="flatten">合成背景颜色</option>
                <option value="reject">不处理透明图片，列入待检查</option>
              </select>
            </label>
            {e.alpha === "flatten" && (
              <label>
                背景颜色
                <input
                  type="color"
                  aria-label="背景颜色"
                  value={e.background ?? "#FFFFFF"}
                  onChange={(ev) => update({ background: ev.target.value })}
                />
              </label>
            )}
          </>
        )}
        {!collection && d.profile && d.profile !== "metadata_only" && (
          <>
            <label>
              已有图片
              <select
                aria-label="已有图片"
                value={d.existing}
                onChange={(ev) =>
                  onChange({
                    existing: ev.target.value as ImageFields["existing"],
                  })
                }
              >
                <option value="keep">保留可复用图片，只补缺图</option>
                <option value="match_profile">补入符合所选策略的版本</option>
              </select>
            </label>
            <p className="lake-hint">
              补入版本会匹配完整编码配置；原有图片与历史任务保持可用。
            </p>
            {d.profile !== "original" && (
              <label className="lake-check">
                <input
                  type="checkbox"
                  checked={d.allowSample}
                  onChange={(ev) =>
                    onChange({ allowSample: ev.target.checked })
                  }
                />
                原件不可用时允许备用图片
              </label>
            )}
          </>
        )}
      </div>
      {d.profile === "custom" && (
        <details>
          <summary>编码高级参数</summary>
          <div className="lake-fields">
            {e.format === "webp" && (
              <label>
                WebP 编码力度（0 快速–6 较慢）
                <input
                  type="number"
                  min={0}
                  max={6}
                  value={e.method ?? 6}
                  onChange={(ev) => update({ method: Number(ev.target.value) })}
                />
              </label>
            )}
            {e.format === "png" && (
              <label>
                PNG 压缩级别（0–9）
                <input
                  aria-label="PNG 压缩级别"
                  type="number"
                  min={0}
                  max={9}
                  value={e.compress_level ?? 6}
                  onChange={(ev) =>
                    update({ compress_level: Number(ev.target.value) })
                  }
                />
              </label>
            )}
            {e.format === "jpeg" && (
              <>
                <label>
                  JPEG 色度采样
                  <select
                    value={e.subsampling ?? "444"}
                    onChange={(ev) => update({ subsampling: ev.target.value })}
                  >
                    <option value="444">4:4:4</option>
                    <option value="420">4:2:0</option>
                  </select>
                </label>
                <label className="lake-check">
                  <input
                    type="checkbox"
                    checked={e.optimize ?? true}
                    onChange={(ev) => update({ optimize: ev.target.checked })}
                  />
                  优化 JPEG 编码
                </label>
              </>
            )}
          </div>
        </details>
      )}
      <details>
        <summary>保存与管理预设</summary>
        <DraftStatus controller={presets.controller} quiet />
        <div className="lake-fields">
          <label>
            已有预设（选择即应用）
            <select
              aria-label="管理保存预设"
              value={editing}
              onChange={(ev) => applyPreset(ev.target.value)}
            >
              <option value="">新预设（保留当前设置）</option>
              {presets.value.items.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          </label>
          <label>
            预设名称
            <input
              aria-label="预设名称"
              value={name}
              maxLength={80}
              onChange={(ev) => setName(ev.target.value)}
            />
          </label>
        </div>
        {selectedPreset && (
          <section className="lake-summary" aria-label="所选预设参数">
            <strong>{selectedPreset.name}</strong>
            <p>{imagePolicyLabel(selectedPreset.media)}</p>
            <ImagePolicySummary policy={selectedPreset.media} />
            {selectedPreset.media.profile !== "metadata_only" && (
              <p className="lake-hint">
                {selectedPreset.media.existing === "match_profile"
                  ? "已有图片：补入符合策略的版本"
                  : "已有图片：保留可复用图片，只补缺图"}
                {selectedPreset.media.allow_sample
                  ? " · 允许备用图片"
                  : " · 不使用备用图片"}
              </p>
            )}
          </section>
        )}
        <div className="lake-actions">
          <Button
            disabled={!d.profile || !presets.editable}
            onClick={() => void save(false)}
          >
            另存为预设
          </Button>
          <Button
            disabled={!editing || !d.profile || !presets.editable}
            onClick={() => void save(true)}
          >
            用当前设置更新预设
          </Button>
          <Button
            disabled={!editing || !presets.editable}
            onClick={() => {
              presets.controller.set({
                items: presets.value.items.filter((p) => p.id !== editing),
              });
              setEditing("");
              setName("");
              setNotice("预设已移除，当前配置和已有任务保留。");
            }}
          >
            删除预设
          </Button>
        </div>
        <p className="lake-hint">
          选择已有预设会立即填入上方参数；修改后可点“应用预设”恢复保存值。
          预设包含保存方式、编码、已有图片与备用图选项，跨项目共享，不改变更新范围和预算。
        </p>
        {error != null && <ErrorDetails error={error} />}
        {notice && <p role="status">{notice}</p>}
      </details>
    </>
  );
}
