import { useRef, useState } from "react";
import { FolderOpen, FileText } from "lucide-react";
import { Button, Dialog, Field } from "@studio/ui";
import type { Project } from "@studio/contracts";
import type { StudioClient } from "@studio/client";
export type DialogKind =
  "new" | "open" | "source" | "collection" | "manifest" | "about" | null;
export function ProjectDialog({
  kind,
  client,
  project,
  selected,
  selectionRevision,
  onClose,
  onCreated,
  onDone,
  pickDirectory,
}: {
  kind: Exclude<DialogKind, null>;
  client: StudioClient;
  project: Project | null;
  selected: number;
  selectionRevision: number;
  onClose: () => void;
  onCreated: (p: Project) => void;
  onDone: () => void;
  pickDirectory: () => Promise<string | null>;
}) {
  const [name, setName] = useState(kind === "source" ? "Danbooru" : "");
  const [directory, setDirectory] = useState("");
  const [media, setMedia] = useState("");
  const [sourceKind, setSourceKind] = useState("danbooru");
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const idempotency = useRef(crypto.randomUUID());
  const titles = {
    new: "新建项目",
    open: "打开项目",
    source: "添加数据湖",
    collection: "保存工作集",
    manifest: "生成数据清单",
    about: "关于 Dataset Studio",
  };
  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setPending(true);
    setError("");
    try {
      if (kind === "new")
        onCreated(
          await client.createProject({
            name,
            parent_directory: directory || null,
          }),
        );
      if (kind === "open") onCreated(await client.openProject(directory));
      if (kind === "source" && project)
        await client.attachSource(project.id, {
          kind: sourceKind,
          name,
          index_root: sourceKind === "danbooru" ? directory : null,
          media_root: sourceKind === "danbooru" ? media : null,
        });
      if (kind === "collection" && project)
        await client.createCollection(project.id, name);
      if (kind === "manifest" && project)
        await client.submitJob(project.id, {
          idempotency_key: idempotency.current,
          selection_revision: selectionRevision,
          delay_ms: 0,
        });
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
          <p>项目数据层 · 0.2.0</p>
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
            (kind === "source" && sourceKind === "danbooru")) && (
            <Field
              label={
                kind === "source"
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
                    kind === "source" ? "包含 CURRENT.json 的目录" : ""
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
          {kind === "source" && sourceKind === "danbooru" && (
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
          {kind === "manifest" && (
            <div className="job-confirm">
              <div>
                <FileText size={28} />
                <strong>数据清单</strong>
              </div>
              <p>
                将当前选择的 <strong>{selected}</strong>{" "}
                个对象固定为输入，生成包含来源、内容身份和存储信息的清单。
              </p>
              <p>任务独立运行并保存进度，完成后可在项目任务中保存成果。</p>
            </div>
          )}
          {kind === "collection" && (
            <p className="dialog-hint">
              将当前选择的 {selected} 个对象保存到项目中。
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
            <Button className="primary" type="submit" disabled={pending}>
              {pending
                ? "处理中…"
                : kind === "manifest"
                  ? "创建任务"
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
