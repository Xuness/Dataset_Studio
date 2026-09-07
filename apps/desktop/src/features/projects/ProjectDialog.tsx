import { useState } from "react";
import { FolderOpen } from "lucide-react";
import { Button, Dialog, Field } from "@studio/ui";
import type { Project, Source } from "@studio/contracts";
import type { StudioClient } from "@studio/client";
import { ScopePicker } from "../scopes/ScopePicker.js";
import type { ScopeOption } from "../scopes/scopes.js";
export type DialogKind =
  "new" | "open" | "source" | "relink" | "collection" | "about" | null;
export function ProjectDialog({
  kind,
  client,
  project,
  scopeOptions,
  defaultScope,
  relinkSource,
  onClose,
  onCreated,
  onDone,
  pickDirectory,
}: {
  kind: Exclude<DialogKind, null>;
  client: StudioClient;
  project: Project | null;
  scopeOptions: ScopeOption[];
  defaultScope: string;
  relinkSource: Source | null;
  onClose: () => void;
  onCreated: (p: Project) => Promise<void>;
  onDone: () => void;
  pickDirectory: () => Promise<string | null>;
}) {
  const [name, setName] = useState(kind === "source" ? "Danbooru" : "");
  const [directory, setDirectory] = useState("");
  const [media, setMedia] = useState("");
  const [sourceKind, setSourceKind] = useState("danbooru");
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const options =
    kind === "collection"
      ? scopeOptions.filter((o) => o.scope.target.kind !== "source")
      : scopeOptions;
  const [scopeId, setScopeId] = useState(
    options.find((o) => o.value === defaultScope)?.value ??
      options[0]?.value ??
      "",
  );
  const input = options.find((o) => o.value === scopeId);
  const titles = {
    new: "新建项目",
    open: "打开项目",
    source: "添加数据湖",
    relink: "重新关联数据湖位置",
    collection: "保存工作集",
    about: "关于 Dataset Studio",
  };
  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setPending(true);
    setError("");
    try {
      if (kind === "collection" && !input)
        throw new Error("请选择可用的数据范围。");
      if (kind === "new")
        await onCreated(
          await client.createProject({
            name,
            parent_directory: directory || null,
          }),
        );
      if (kind === "open") await onCreated(await client.openProject(directory));
      if (kind === "source" && project)
        await client.attachSource(project.id, {
          kind: sourceKind,
          name,
          index_root: sourceKind === "danbooru" ? directory : null,
          media_root: sourceKind === "danbooru" ? media : null,
        });
      if (kind === "relink" && project && relinkSource)
        await client.relinkSource(project.id, relinkSource.id, {
          index_root: directory,
          media_root: media,
        });
      if (kind === "collection" && project)
        await client.createCollection(project.id, name, input?.scope);
      onDone();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }
  return (
    <Dialog
      title={titles[kind]}
      onClose={() => {
        if (!pending) onClose();
      }}
    >
      {kind === "about" ? (
        <div className="about-content">
          <span className="brand-tile">Ds</span>
          <h2>Dataset Studio</h2>
          <p>项目数据范围层 · 0.3.0</p>
          <p>项目、数据湖、工作集与持续保存的工作。</p>
          <Button onClick={onClose}>关闭</Button>
        </div>
      ) : (
        <form onSubmit={(e) => void submit(e)}>
          {kind === "source" && (
            <Field label="数据源类型">
              <select
                value={sourceKind}
                onChange={(e) => {
                  setSourceKind(e.target.value);
                  setName(e.target.value === "demo" ? "参考资料" : "Danbooru");
                }}
              >
                <option value="danbooru">Danbooru 归档</option>
                <option value="demo">内置参考资料（用于验证）</option>
              </select>
            </Field>
          )}
          {["new", "source", "collection"].includes(kind) && (
            <Field
              label={
                kind === "collection"
                  ? "工作集名称"
                  : kind === "source"
                    ? "显示名称"
                    : "项目名称"
              }
            >
              <input
                autoFocus
                value={name}
                onChange={(e) => setName(e.target.value)}
                required
                maxLength={120}
              />
            </Field>
          )}
          {(kind === "new" ||
            kind === "open" ||
            kind === "relink" ||
            (kind === "source" && sourceKind === "danbooru")) && (
            <Field
              label={
                kind === "source" || kind === "relink"
                  ? "快速索引目录"
                  : kind === "open"
                    ? "项目目录"
                    : "保存位置（留空使用应用项目目录）"
              }
            >
              <div className="path-field">
                <input
                  value={directory}
                  onChange={(e) => setDirectory(e.target.value)}
                  required={kind !== "new"}
                  placeholder={
                    kind === "source" || kind === "relink"
                      ? "包含 CURRENT.json 的目录"
                      : ""
                  }
                />
                <Button
                  type="button"
                  onClick={() =>
                    void pickDirectory().then((value) => {
                      if (value) setDirectory(value);
                    })
                  }
                >
                  <FolderOpen size={15} />
                </Button>
              </div>
            </Field>
          )}
          {(kind === "relink" ||
            (kind === "source" && sourceKind === "danbooru")) && (
            <Field label="图片湖目录">
              <div className="path-field">
                <input
                  value={media}
                  onChange={(e) => setMedia(e.target.value)}
                  placeholder="包含 library.json 的目录"
                  required
                />
                <Button
                  type="button"
                  onClick={() =>
                    void pickDirectory().then((value) => {
                      if (value) setMedia(value);
                    })
                  }
                >
                  <FolderOpen size={15} />
                </Button>
              </div>
            </Field>
          )}
          {kind === "collection" && (
            <ScopePicker
              options={options}
              value={scopeId}
              onChange={(value) => {
                setScopeId(value);
              }}
              label="工作集成员范围"
            />
          )}
          {kind === "relink" && (
            <p className="dialog-hint">
              为「{relinkSource?.name}
              」选择同一个数据湖的新位置。此位置由当前应用登记的项目共享，调整会影响其中所有引用该数据湖的项目。
            </p>
          )}
          {kind === "collection" && (
            <p className="dialog-hint">
              保存所选范围的固定成员。之后修改选择或查询定义不会改变工作集。
            </p>
          )}
          {kind === "source" && (
            <p className="dialog-hint">
              数据湖作为整体加入项目。图片按需读取，选择与整理结果保存在项目内。
            </p>
          )}
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
                (kind === "collection" && (!input || input.count === 0))
              }
            >
              {pending
                ? "处理中…"
                : kind === "open"
                  ? "打开"
                  : kind === "source"
                    ? "加入项目"
                    : "保存"}
            </Button>
          </div>
        </form>
      )}
    </Dialog>
  );
}
