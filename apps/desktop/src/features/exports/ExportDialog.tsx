import { useState } from "react";
import { FolderOpen } from "lucide-react";
import { Button, Dialog, Field, errorText, usePlatformFiles } from "@studio/ui";
import type { StudioClient } from "@studio/client";
import type { Job } from "@studio/contracts";
import { ScopePicker } from "../scopes/ScopePicker.js";
import type { ScopeOption } from "../scopes/scopes.js";
import "./exports.css";

export const EXPORT_OPERATOR = "core.export_files";
type Metadata = "none" | "tags" | "full";
type TagStyle = "comma_spaces" | "comma" | "space";
type Settings = {
  destination: string;
  metadata: Metadata;
  tagStyle: TagStyle;
  manifest: boolean;
};
const storageKey = "studio.export.settings";
const defaults: Settings = {
  destination: "",
  metadata: "none",
  tagStyle: "comma_spaces",
  manifest: true,
};
// Last-used options are a per-viewer convenience only.
function loadSettings(): Settings {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(storageKey) ?? "");
    return value && typeof value === "object"
      ? { ...defaults, ...(value as Partial<Settings>) }
      : defaults;
  } catch {
    return defaults;
  }
}
function storeSettings(settings: Settings) {
  try {
    localStorage.setItem(storageKey, JSON.stringify(settings));
  } catch {
    /* Storage can be unavailable; the dialog still works. */
  }
}

export function ExportDialog({
  client,
  projectId,
  options,
  defaultScope,
  onClose,
  onSubmitted,
}: {
  client: StudioClient;
  projectId: string;
  options: ScopeOption[];
  defaultScope: string;
  onClose: () => void;
  onSubmitted: (job: Job, count: number | null) => void;
}) {
  const files = usePlatformFiles();
  const [settings, setSettings] = useState(loadSettings);
  const [scopeId, setScopeId] = useState(
    options.find((o) => o.value === defaultScope)?.value ??
      options[0]?.value ??
      "",
  );
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const input = options.find((o) => o.value === scopeId);
  const update = (patch: Partial<Settings>) =>
    setSettings((old) => ({ ...old, ...patch }));
  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (!input) return;
    setPending(true);
    setError("");
    try {
      const job = await client.tools.submit(projectId, {
        idempotency_key: crypto.randomUUID(),
        scope: input.scope,
        run: {
          operator_id: EXPORT_OPERATOR,
          operator_version: 1,
          parameters_version: 1,
          parameters: {
            destination: settings.destination.trim(),
            metadata: settings.metadata,
            tag_style: settings.tagStyle,
            manifest: settings.manifest,
          },
        },
      });
      storeSettings(settings);
      onSubmitted(job, input.count);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setPending(false);
    }
  }
  return (
    <Dialog
      title="导出原图"
      onClose={() => {
        if (!pending) onClose();
      }}
    >
      <form className="export-dialog" onSubmit={(e) => void submit(e)}>
        <ScopePicker
          options={options}
          value={scopeId}
          onChange={setScopeId}
          label="导出范围"
        />
        <Field label="目标文件夹">
          <div className="path-field">
            <input
              aria-label="目标文件夹"
              value={settings.destination}
              onChange={(e) => update({ destination: e.target.value })}
              placeholder="已存在的文件夹，例如 D:\Exports\workset-a"
              required
              autoFocus
            />
            <Button
              type="button"
              title="选择文件夹"
              aria-label="选择文件夹"
              onClick={() =>
                void files.chooseDirectory().then((value) => {
                  if (value) update({ destination: value });
                })
              }
            >
              <FolderOpen size={15} />
            </Button>
          </div>
        </Field>
        <Field label="附带元数据">
          <select
            aria-label="附带元数据"
            value={settings.metadata}
            onChange={(e) => update({ metadata: e.target.value as Metadata })}
          >
            <option value="none">只导出原图</option>
            <option value="tags">标签：每张图一个同名 .txt</option>
            <option value="full">完整元数据：每张图一个同名 .json</option>
          </select>
        </Field>
        {settings.metadata === "tags" && (
          <Field label="标签格式">
            <select
              aria-label="标签格式"
              value={settings.tagStyle}
              onChange={(e) => update({ tagStyle: e.target.value as TagStyle })}
            >
              <option value="comma_spaces">
                long hair, 1girl（逗号，下划线转空格）
              </option>
              <option value="comma">
                long_hair, 1girl（逗号，保留下划线）
              </option>
              <option value="space">
                long_hair 1girl（空格，Danbooru 原样）
              </option>
            </select>
          </Field>
        )}
        <label className="check-row">
          <input
            type="checkbox"
            checked={settings.manifest}
            onChange={(e) => update({ manifest: e.target.checked })}
          />
          在目标文件夹写入
          manifest.jsonl（每行：文件名、图像身份、来源、导出状态）
        </label>
        <p className="dialog-hint">
          将导出「{input?.label ?? "尚未选择范围"}」的{" "}
          {input?.count?.toLocaleString("zh-CN") ?? "待确定数量的"}{" "}
          项。提交时固定成员，文件名为“序号_原文件名”，序号按图像身份的固定顺序，不是排名顺序。
          已有原图按大小和内容身份检查，标签或 JSON 按本次内容检查；一致时复用，
          冲突时将图片与元数据一起改名为
          _2、_3。已登记的数据湖目录不能作为目标。 单张超过 64
          MiB，或原图、元数据导出失败时，会记录在
          export-errors.jsonl；其余原图继续导出，任务会报告未完整导出的数量。
        </p>
        {error && (
          <p className="dialog-error" role="alert">
            {error}
          </p>
        )}
        <div className="dialog-actions">
          <Button type="button" onClick={onClose} disabled={pending}>
            取消
          </Button>
          <Button
            className="primary"
            type="submit"
            disabled={
              pending ||
              !input ||
              input.count === 0 ||
              !settings.destination.trim()
            }
          >
            {pending ? "正在提交…" : "开始导出"}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
