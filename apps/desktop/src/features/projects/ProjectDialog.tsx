import { useQuery } from "@tanstack/react-query";
import { useRef, useState } from "react";
import { FolderOpen } from "lucide-react";
import { Button, Dialog, Field, Brand } from "@studio/ui";
import type { Project, Source, Schema } from "@studio/contracts";
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
  const [name, setName] = useState(kind === "source" ? "数据湖" : "");
  const [directory, setDirectory] = useState("");
  const [media, setMedia] = useState("");
  const [sourceKind, setSourceKind] = useState("auto");
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const capture = useRef<AbortController | null>(null);
  const [captureProgress, setCaptureProgress] = useState<
    Schema["QueryResult"] | null
  >(null);
  const adapters = useQuery({
    queryKey: ["source-adapters"],
    queryFn: ({ signal }) => client.sourceAccess.adapters(signal),
    enabled: kind === "source",
  });
  const registration = adapters.data?.items.find(
    (item) => item.kind === sourceKind,
  );
  const needsPaths =
    sourceKind === "auto" ||
    registration?.descriptor.capabilities.relink === true;
  const probeBody = {
    kind: sourceKind,
    index_root: needsPaths ? directory : null,
    media_root: needsPaths ? media : null,
  };
  const probeKey = JSON.stringify(probeBody);
  const [checked, setChecked] = useState<{
    key: string;
    value: Schema["SourcePreflight"];
  } | null>(null);
  const currentProbe = checked?.key === probeKey ? checked.value : null;
  async function checkSource() {
    setPending(true);
    setError("");
    try {
      const value = await client.sourceAccess.probe(probeBody);
      setChecked({ key: probeKey, value });
      if (name === "数据湖") setName(value.descriptor.display_name);
    } catch (error) {
      setChecked(null);
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setPending(false);
    }
  }

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
    collection: "保存当前范围为工作集",
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
      if (kind === "source" && project) {
        const verified = await client.sourceAccess.probe(probeBody);
        await client.attachSource(project.id, {
          ...probeBody,
          kind: verified.kind,
          name: name === "数据湖" ? verified.descriptor.display_name : name,
        });
      }
      if (kind === "relink" && project && relinkSource)
        await client.relinkSource(project.id, relinkSource.id, {
          index_root: directory,
          media_root: media,
        });
      if (kind === "collection" && project) {
        capture.current = new AbortController();
        await client.createCollection(
          project.id,
          name,
          input?.scope,
          setCaptureProgress,
          capture.current.signal,
        );
      }
      onDone();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      capture.current = null;
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
          <Brand size={72} />
          <h2>Dataset Studio</h2>
          <p>Dataset Studio · 0.9.1</p>
          <p>项目、数据湖、工作集与持续保存的工作。</p>
          <Button onClick={onClose}>关闭</Button>
        </div>
      ) : (
        <form onSubmit={(e) => void submit(e)}>
          {kind === "source" && (
            <Field label="数据源类型">
              <select
                aria-label="数据源类型"
                value={sourceKind}
                onChange={(e) => {
                  setSourceKind(e.target.value);
                  setName(
                    adapters.data?.items.find((a) => a.kind === e.target.value)
                      ?.name ?? "数据湖",
                  );
                }}
              >
                <option value="auto">自动识别已转换数据湖</option>
                {adapters.data?.items.map((adapter) => (
                  <option key={adapter.kind} value={adapter.kind}>
                    {adapter.name}
                  </option>
                ))}
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
            (kind === "source" && needsPaths)) && (
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
          {(kind === "relink" || (kind === "source" && needsPaths)) && (
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
          {kind === "source" && (
            <>
              {adapters.error && (
                <p className="dialog-error" role="alert">
                  来源类型读取失败：{adapters.error.message}
                </p>
              )}
              <Button
                type="button"
                onClick={() => void checkSource()}
                disabled={pending || (needsPaths && (!directory || !media))}
              >
                检查数据湖
              </Button>
              {currentProbe && (
                <div role="status">
                  <p>
                    已识别 {currentProbe.descriptor.display_name}
                    ，数据湖身份与索引检查通过。
                  </p>
                  <small>
                    图片按唯一存储对象浏览；无图片的元数据记录保留在湖内。
                  </small>
                  <details>
                    <summary>身份与版本</summary>
                    <p>{currentProbe.source_id}</p>
                    <p>{currentProbe.revision}</p>
                  </details>
                </div>
              )}
            </>
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
              已登记更新服务的数据湖请使用设置中的“数据湖 API → 迁移数据湖位置”，并在搬动文件前准备迁移。
            </p>
          )}
          {kind === "collection" && (
            <p className="dialog-hint">
              将保存「{input?.label ?? "尚未选择范围"}」的{" "}
              {input?.count?.toLocaleString("zh-CN") ?? "待确定数量的"}{" "}
              项成员。之后修改选择或查询定义不会改变工作集。
              <br />
              要保存排名筛选结果，请使用“结果榜单”中的“保存筛选为工作集”。
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
          {pending && captureProgress && (
            <p role="status" className="dialog-hint">
              正在固定当前视图：已检查{" "}
              {captureProgress.processed.toLocaleString()} 项。
            </p>
          )}
          <div className="dialog-actions">
            <Button
              type="button"
              onClick={() => (pending ? capture.current?.abort() : onClose())}
              disabled={pending && kind !== "collection"}
            >
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
