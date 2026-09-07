import { useProjectSession } from "../features/projects/useProjectSession.js";
import { useProjectQueries } from "../features/query/useProjectQueries.js";
import { QueryPanel } from "../features/query/QueryPanel.js";
import { scopeOptions } from "../features/scopes/scopes.js";
import { ProjectDialog } from "../features/projects/ProjectDialog.js";
import type { DialogKind } from "../features/projects/ProjectDialog.js";
import { useEffect, useRef, useState, useCallback } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Images,
  FolderOpen,
  FolderPlus,
  Plus,
  ChevronDown,
  ChevronRight,
  MousePointer2,
  PanelLeft,
  PanelRight,
  ListTodo,
  Home,
  Database,
  FileText,
  X,
  Layers,
  Info,
  RotateCw,
  Search,
  Link2,
} from "lucide-react";
import { Button } from "@studio/ui";
import type {
  Project,
  Asset,
  AssetKey,
  Source,
  QueryResult,
  ScopeOperation,
  ScopeRef,
} from "@studio/contracts";
import type { StudioClient } from "@studio/client";
import {
  connectEngine,
  chooseDirectory,
  onNativeClose,
} from "../platform/connection.js";
import { Browser } from "../features/browser/Browser.js";
import type { Scope } from "../features/browser/Browser.js";
import { AssetImage } from "../features/browser/AssetImage.js";
import { Tasks } from "../features/tasks/Tasks.js";
import { MetadataInspector } from "../features/metadata/MetadataInspector.js";

type ViewState = { scope: Scope; focus: Asset | null; view: "grid" | "image" };
const emptyView: ViewState = {
  scope: { kind: "all" },
  focus: null,
  view: "grid",
};
export function App() {
  const cache = useQueryClient();
  const engine = useQuery({
    queryKey: ["engine"],
    queryFn: connectEngine,
    retry: 1,
    staleTime: Infinity,
  });
  const reconnect = useCallback(() => void engine.refetch(), [engine.refetch]);
  const previous = useRef<StudioClient | null>(null);
  useEffect(() => {
    if (engine.data && previous.current !== engine.data) {
      previous.current?.dispose();
      cache.removeQueries({ queryKey: ["project"] });
      previous.current = engine.data;
    }
  }, [engine.data, cache]);
  useEffect(() => {
    if (!engine.data) return;
    const client = engine.data;
    let stopped = false;
    let unlisten: (() => void) | undefined;
    void onNativeClose(() => client.releaseProjectViews()).then((stop) => {
      if (stopped) stop();
      else unlisten = stop;
    });
    return () => {
      stopped = true;
      unlisten?.();
    };
  }, [engine.data]);
  if (!engine.data)
    return (
      <div className="connection-screen">
        <span className="brand-tile">Ds</span>
        <h1>Dataset Studio</h1>
        <p>{engine.error ? engine.error.message : "正在连接本机引擎…"}</p>
        {engine.error ? (
          <Button onClick={() => void engine.refetch()}>
            <RotateCw size={14} />
            重新连接
          </Button>
        ) : (
          <span className="connection-progress" />
        )}
      </div>
    );
  return (
    <Studio
      key={engine.data.connection.instance_id}
      client={engine.data}
      onReconnect={reconnect}
    />
  );
}
function Studio({
  client,
  onReconnect,
}: {
  client: StudioClient;
  onReconnect: () => void;
}) {
  const queryClient = useQueryClient();
  const health = useQuery({
    queryKey: ["engine-health", client.connection.instance_id],
    queryFn: () => client.health(),
    refetchInterval: 5000,
    retry: false,
  });
  useEffect(() => {
    if (health.isError) {
      const timer = setTimeout(onReconnect, 1200);
      return () => clearTimeout(timer);
    }
  }, [health.isError, health.errorUpdatedAt, onReconnect]);
  const [views, setViews] = useState<Record<string, ViewState>>({});
  const [dialog, setDialog] = useState<DialogKind>(null);
  const [menu, setMenu] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [saving, setBusy] = useState(false);
  const session = useProjectSession(client, setError);
  const { project, projects } = session;
  const busy = saving || session.pending;
  const [queryVisible, setQueryVisible] = useState(false);
  const [relinkTarget, setRelinkTarget] = useState<Source | null>(null);
  const [projectsVisible, setProjectsVisible] = useState(true);
  const [propertiesVisible, setPropertiesVisible] = useState(true);
  const [tasksVisible, setTasksVisible] = useState(false);
  const currentId = project?.id ?? "";
  const view = views[currentId] ?? emptyView;
  const sources = useQuery({
    queryKey: ["project", currentId, "sources"],
    queryFn: () => client.sources(currentId),
    enabled: !!project,
  });
  const selection = useQuery({
    queryKey: ["project", currentId, "selection"],
    queryFn: () => client.selection(currentId),
    enabled: !!project,
  });
  const collections = useQuery({
    queryKey: ["project", currentId, "collections"],
    queryFn: () => client.collections(currentId),
    enabled: !!project,
  });
  const jobs = useQuery({
    queryKey: ["project", currentId, "jobs"],
    queryFn: () => client.jobs(currentId),
    enabled: !!project,
  });
  const queryModel = useProjectQueries(client, currentId);
  const activeResultId = view.scope.kind === "result" ? view.scope.id : "";
  const activeResult = useQuery({
    queryKey: ["project", currentId, "query-result", activeResultId],
    queryFn: ({ signal }) =>
      client.queries.result(currentId, activeResultId, signal),
    enabled: !!project && !!activeResultId,
  });
  const results = queryModel.results.data?.items ?? [];
  const availableResults =
    activeResult.data && !results.some((r) => r.id === activeResult.data?.id)
      ? [...results, activeResult.data]
      : results;
  const inputs = project
    ? scopeOptions(
        project.id,
        selection.data,
        sources.data?.items ?? [],
        collections.data?.items ?? [],
        availableResults,
        queryModel.definitions.data?.items ?? [],
      )
    : [];
  const hasFixedInput = inputs.some(
    (option) => option.scope.target.kind !== "source" && option.count !== 0,
  );
  const hasTaskInput = inputs.some((option) => option.count !== 0);
  const defaultScope =
    view.scope.kind !== "all" && view.scope.kind !== "selection"
      ? view.scope.id
      : "selection";
  const selected = selection.data?.count ?? 0;
  function updateView(update: Partial<ViewState>) {
    setViews((old) => ({
      ...old,
      [currentId]: {
        ...(old[currentId] ?? emptyView),
        ...(update.scope ? { focus: null, view: "grid" as const } : {}),
        ...update,
      },
    }));
  }
  function activate(p: Project) {
    setMenu(null);
    setError("");
    return session.activate(p);
  }
  useEffect(() => {
    if (!currentId) return;
    const abort = new AbortController();
    void client.watch(
      currentId,
      (event) => {
        const prefix = ["project", currentId];
        if (event.kind === "project.closed") {
          void session.close();
          return;
        }
        if (event.kind.startsWith("job."))
          void queryClient.invalidateQueries({ queryKey: [...prefix, "jobs"] });
        else if (event.kind.startsWith("selection.")) {
          void queryClient.invalidateQueries({
            queryKey: [...prefix, "selection"],
          });
          void queryClient.invalidateQueries({
            queryKey: [...prefix, "assets"],
          });
        } else void queryClient.invalidateQueries({ queryKey: prefix });
      },
      abort.signal,
    );
    return () => abort.abort();
  }, [client, currentId, queryClient, session.close]);
  async function act(action: () => Promise<unknown>, pid = currentId) {
    setBusy(true);
    setError("");
    try {
      await action();
      if (pid)
        await queryClient.invalidateQueries({ queryKey: ["project", pid] });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      if (pid)
        void queryClient.invalidateQueries({ queryKey: ["project", pid] });
    } finally {
      setBusy(false);
    }
  }
  function pick(keys: AssetKey[], remove = false) {
    const revision = selection.data?.revision;
    if (revision === undefined || busy) return;
    void act(() =>
      client.changeSelection(currentId, {
        expected_revision: revision,
        add: remove ? [] : keys,
        remove: remove ? keys : [],
        clear: false,
      }),
    );
  }
  function operateScope(scope: ScopeRef, operation: ScopeOperation) {
    if (selection.data?.revision === undefined || busy) return;
    void act(() =>
      client.changeSelectionScope(currentId, {
        expected_revision: selection.data!.revision,
        scope,
        operation,
      }),
    );
  }
  function selectResult(result: QueryResult, operation: ScopeOperation) {
    operateScope(
      {
        project_id: currentId,
        target: { kind: "query_result", result_id: result.id },
      },
      operation,
    );
  }
  function operateViewScope(operation: ScopeOperation) {
    if (view.scope.kind === "result")
      operateScope(
        {
          project_id: currentId,
          target: { kind: "query_result", result_id: view.scope.id },
        },
        operation,
      );
    if (view.scope.kind === "collection")
      operateScope(
        {
          project_id: currentId,
          target: { kind: "workset", collection_id: view.scope.id },
        },
        operation,
      );
  }
  function clearSelection() {
    const revision = selection.data?.revision;
    if (revision === undefined) return;
    void act(() =>
      client.changeSelection(currentId, {
        expected_revision: revision,
        add: [],
        remove: [],
        clear: true,
      }),
    );
  }
  const activeJobs =
    jobs.data?.items.filter((j) =>
      ["waiting_input", "queued", "preparing", "running"].includes(j.status),
    ).length ?? 0;
  const menus: Record<
    string,
    { label: string; action: () => void; disabled?: boolean }[]
  > = {
    项目: [
      { label: "新建项目…", action: () => setDialog("new") },
      { label: "打开项目…", action: () => setDialog("open") },
      {
        label: "关闭当前项目",
        action: () => void session.close(),
        disabled: !project,
      },
    ],
    编辑: [
      { label: "清除当前选择", action: clearSelection, disabled: !selected },
      {
        label: "保存选择为工作集…",
        action: () => setDialog("collection"),
        disabled: !hasFixedInput,
      },
    ],
    视图: [
      {
        label: "图像网格",
        action: () => updateView({ view: "grid" }),
        disabled: !project,
      },
      {
        label: "单图查看",
        action: () => updateView({ view: "image" }),
        disabled: !view.focus,
      },
      {
        label: "刷新当前项目",
        action: () =>
          void queryClient.invalidateQueries({
            queryKey: ["project", currentId],
          }),
        disabled: !project,
      },
    ],
    工具: [
      {
        label: "项目查询",
        action: () => setQueryVisible((value) => !value),
        disabled: !project,
      },
      {
        label: "生成数据清单…",
        action: () => setDialog("manifest"),
        disabled: !hasTaskInput,
      },
    ],
    窗口: [
      { label: "项目面板", action: () => setProjectsVisible((v) => !v) },
      { label: "属性面板", action: () => setPropertiesVisible((v) => !v) },
      {
        label: "项目任务",
        action: () => setTasksVisible((v) => !v),
        disabled: !project,
      },
    ],
    帮助: [{ label: "关于 Dataset Studio", action: () => setDialog("about") }],
  };
  return (
    <div
      className="studio-app"
      onClick={() => {
        if (menu) setMenu(null);
      }}
    >
      <div className="menu-bar">
        <span className="brand-small">Ds</span>
        {Object.keys(menus).map((name) => (
          <div className="menu-anchor" key={name}>
            <button
              className={menu === name ? "menu-button open" : "menu-button"}
              onClick={(e) => {
                e.stopPropagation();
                setMenu(menu === name ? null : name);
              }}
            >
              {name}
            </button>
            {menu === name && (
              <div className="menu-popup">
                {menus[name]?.map((item) => (
                  <button
                    key={item.label}
                    disabled={item.disabled || busy}
                    onClick={() => {
                      item.action();
                      setMenu(null);
                    }}
                  >
                    {item.label}
                  </button>
                ))}
              </div>
            )}
          </div>
        ))}
        <span className="grow" />
        <span className="version-label">开发版 0.3</span>
      </div>
      <div className="options-bar">
        <button
          className="icon-button"
          title="项目起始页"
          onClick={() => void session.close()}
        >
          <Home size={17} />
        </button>
        <button
          disabled={!project}
          onClick={() => setQueryVisible((value) => !value)}
        >
          <Search size={14} />
          查询
        </button>
        <span className="option-separator" />
        <MousePointer2 size={16} />
        <span className="option-label">选择</span>
        <span className="option-value">{selected} 项</span>
        <button disabled={!selected || busy} onClick={clearSelection}>
          清除
        </button>
        <span className="option-separator" />
        <button
          disabled={!hasFixedInput || busy}
          onClick={() => setDialog("collection")}
        >
          <FolderPlus size={14} />
          存为工作集
        </button>
        <button
          disabled={!hasTaskInput || busy}
          onClick={() => setDialog("manifest")}
        >
          <FileText size={14} />
          生成清单
        </button>
        <span className="grow" />
        <button
          className="icon-button"
          title="项目面板"
          onClick={() => setProjectsVisible((v) => !v)}
        >
          <PanelLeft size={16} />
        </button>
        <button
          className="icon-button"
          title="属性面板"
          onClick={() => setPropertiesVisible((v) => !v)}
        >
          <PanelRight size={16} />
        </button>
      </div>
      <div className="document-tabs">
        {project ? (
          <div className="document-tab active">
            <Layers size={13} />
            <span>{project.name}</span>
            <small>{busy ? "保存中…" : "已保存"}</small>
            <button aria-label="关闭项目" onClick={() => void session.close()}>
              <X size={12} />
            </button>
          </div>
        ) : (
          <div className="document-tab active">
            <Home size={13} />
            <span>开始</span>
          </div>
        )}
        <span className="grow" />
      </div>
      {!project ? (
        <main className="start-screen">
          <div className="start-actions">
            <span className="brand-tile">Ds</span>
            <h1>Dataset Studio</h1>
            <p>打开项目，继续你的数据工作。</p>
            <Button className="primary" onClick={() => setDialog("new")}>
              <Plus size={15} />
              新建项目
            </Button>
            <Button onClick={() => setDialog("open")}>
              <FolderOpen size={15} />
              打开项目
            </Button>
          </div>
          <section className="recent-projects">
            <h2>最近使用的项目</h2>
            {projects.error ? (
              <p>{projects.error.message}</p>
            ) : projects.data?.items.length ? (
              projects.data.items.map((p) => (
                <button
                  className="recent-row"
                  key={p.id}
                  onClick={() => void session.openRecent(p.id)}
                  disabled={busy}
                >
                  <span className="recent-icon">
                    <Layers size={24} />
                  </span>
                  <span>
                    <strong>{p.name}</strong>
                    <small>{p.directory}</small>
                    {p.issue && (
                      <small className="source-offline">{p.issue}</small>
                    )}
                  </span>
                  <small className="recent-state">
                    {
                      {
                        open: "已打开",
                        background: "后台工作中",
                        draining: "正在关闭",
                        unavailable: "需要检查",
                        closed: "",
                      }[p.state]
                    }
                  </small>
                  <ChevronRight size={16} />
                </button>
              ))
            ) : (
              <div className="recent-empty">
                你的项目会保存在这里。
                <br />
                可以随时重新打开，继续之前的选择和整理。
              </div>
            )}
          </section>
        </main>
      ) : (
        <div
          className={
            "workspace " +
            (!projectsVisible ? "hide-projects " : "") +
            (!propertiesVisible ? "hide-properties" : "")
          }
        >
          <aside className="tool-rail">
            <button
              className="tool-button active"
              title="资料浏览"
              onClick={() => updateView({ view: "grid" })}
            >
              <Images size={19} />
            </button>
            <button
              className="tool-button"
              title="单图查看"
              disabled={!view.focus}
              onClick={() => updateView({ view: "image" })}
            >
              <MousePointer2 size={19} />
            </button>
            <button
              className={"tool-button " + (queryVisible ? "active" : "")}
              title="项目查询"
              onClick={() => setQueryVisible((value) => !value)}
            >
              <Search size={18} />
            </button>
            <div className="rail-divider" />
            <button
              className="tool-button"
              title="项目任务"
              onClick={() => setTasksVisible((v) => !v)}
            >
              <ListTodo size={19} />
            </button>
            <span className="grow" />
            <button
              className="tool-button"
              title="添加数据湖"
              onClick={() => setDialog("source")}
            >
              <Database size={18} />
            </button>
          </aside>
          <aside className="project-panel">
            <header className="panel-tabs">
              <strong>项目</strong>
              <span className="grow" />
              <button
                className="icon-button"
                title="添加数据湖"
                onClick={() => setDialog("source")}
              >
                <Plus size={14} />
              </button>
            </header>
            <div className="project-tree">
              <button
                className={
                  "tree-row root " + (view.scope.kind === "all" ? "active" : "")
                }
                onClick={() => updateView({ scope: { kind: "all" } })}
              >
                <Layers size={15} />
                <span>全部项目数据</span>
              </button>
              <button
                className={
                  "tree-row " +
                  (view.scope.kind === "selection" ? "active" : "")
                }
                onClick={() =>
                  updateView({
                    scope: { kind: "selection", name: "项目当前选择" },
                  })
                }
              >
                <MousePointer2 size={14} />
                <span>当前选择</span>
                <small>{selected}</small>
              </button>
              <div className="tree-heading">
                <ChevronDown size={12} />
                <span>数据湖</span>
              </div>
              {sources.data?.items.map((source) => (
                <SourceRow
                  key={source.id}
                  source={source}
                  onRelink={() => {
                    setRelinkTarget(source);
                    setDialog("relink");
                  }}
                  active={
                    view.scope.kind === "source" && view.scope.id === source.id
                  }
                  onClick={() =>
                    updateView({
                      scope: {
                        kind: "source",
                        id: source.id,
                        name: source.name,
                      },
                    })
                  }
                />
              ))}
              {!sources.data?.items.length && (
                <button
                  className="tree-add"
                  onClick={() => setDialog("source")}
                >
                  <Plus size={13} />
                  添加第一个数据湖
                </button>
              )}
              <div className="tree-heading">
                <ChevronDown size={12} />
                <span>工作集</span>
                <span className="grow" />
                <button
                  disabled={!selected}
                  title="保存选择为工作集"
                  className="icon-button"
                  onClick={() => setDialog("collection")}
                >
                  <Plus size={12} />
                </button>
              </div>
              {collections.data?.items.map((c) => (
                <button
                  key={c.id}
                  className={
                    "tree-row " +
                    (view.scope.kind === "collection" && view.scope.id === c.id
                      ? "active"
                      : "")
                  }
                  onClick={() =>
                    updateView({
                      scope: { kind: "collection", id: c.id, name: c.name },
                    })
                  }
                >
                  <FolderOpen size={14} />
                  <span>{c.name}</span>
                  <small>{c.count}</small>
                </button>
              ))}
              {!collections.data?.items.length && (
                <p className="tree-hint">选择资料后，可将它们保存为工作集。</p>
              )}
            </div>
            <div className="project-foot">
              <strong>{project.name}</strong>
              <span>
                {sources.data?.items.length ?? 0} 个数据湖 ·{" "}
                {collections.data?.items.length ?? 0} 个工作集
              </span>
            </div>
          </aside>
          <div className="data-workspace">
            {queryVisible && (
              <QueryPanel
                key={"query:" + project.id}
                client={client}
                projectId={project.id}
                sources={sources.data?.items ?? []}
                model={queryModel}
                onResult={(result, name) =>
                  updateView({ scope: { kind: "result", id: result.id, name } })
                }
                onSelect={selectResult}
                onClose={() => setQueryVisible(false)}
              />
            )}
            <Browser
              key={"browser:" + project.id}
              client={client}
              projectId={project.id}
              scope={view.scope}
              focus={view.focus}
              onFocus={(focus) => updateView({ focus })}
              onPick={pick}
              onScopeOperation={operateViewScope}
              selectionRevision={selection.data?.revision ?? 0}
              busy={busy}
              view={view.view}
              setView={(mode) => updateView({ view: mode })}
            />
          </div>
          <aside className="properties-panel">
            <header className="panel-tabs">
              <strong>属性</strong>
            </header>
            {view.focus ? (
              <div className="properties-scroll">
                <div className="property-preview">
                  <AssetImage
                    client={client}
                    projectId={project.id}
                    asset={view.focus}
                    edge={480}
                  />
                </div>
                <div className="property-section">
                  <h3>图像对象</h3>
                  <dl>
                    <dt>名称</dt>
                    <dd className="hash-value" title={view.focus.name}>
                      {view.focus.name}
                    </dd>
                    <dt>来源</dt>
                    <dd>{view.focus.source_name}</dd>
                    <dt>格式</dt>
                    <dd>{view.focus.extension.toUpperCase()}</dd>
                    <dt>存储大小</dt>
                    <dd>
                      {Number(view.focus.bytes)
                        ? (Number(view.focus.bytes) / 1024).toFixed(1) + " KB"
                        : "参考样本"}
                    </dd>
                  </dl>
                </div>
                {propertiesVisible && (
                  <MetadataInspector
                    key={
                      project.id +
                      ":" +
                      view.focus.key.source_id +
                      ":" +
                      view.focus.key.asset_id +
                      ":" +
                      client.connection.instance_id
                    }
                    client={client}
                    projectId={project.id}
                    asset={view.focus}
                  />
                )}
                <div className="property-section">
                  <h3>项目中的选择</h3>
                  <p>
                    当前共选择 <strong>{selected}</strong> 项
                  </p>
                  <Button
                    disabled={!selected || busy}
                    onClick={() => setDialog("collection")}
                  >
                    <FolderPlus size={14} />
                    保存为工作集
                  </Button>
                </div>
              </div>
            ) : (
              <div className="properties-empty">
                <MousePointer2 size={25} />
                <p>
                  点击一张图片
                  <br />
                  在这里查看对象属性
                </p>
              </div>
            )}
            <div className="properties-bottom">
              <Info size={13} />
              <span>项目保存选择和成果，数据湖提供来源。</span>
            </div>
          </aside>
        </div>
      )}
      {project && tasksVisible && (
        <Tasks
          client={client}
          projectId={project.id}
          onClose={() => setTasksVisible(false)}
          onError={setError}
        />
      )}
      {error && (
        <div className="error-banner" role="alert">
          <Info size={15} />
          <span>{error}</span>
          <button
            className="icon-button"
            aria-label="关闭错误提示"
            onClick={() => setError("")}
          >
            <X size={14} />
          </button>
        </div>
      )}
      <footer className="status-bar">
        <span className={health.error ? "online-dot offline" : "online-dot"} />
        <span>{health.error ? "本机引擎已断开" : "本机引擎已连接"}</span>
        {health.error && <button onClick={onReconnect}>重新连接</button>}
        {project && (
          <>
            <span className="status-divider" />
            <span>{busy ? "正在保存项目" : "项目已保存"}</span>
            <span className="status-divider" />
            <span>
              已选 {selected} 项
              {selection.data?.base_result
                ? " · 基于查询结果，已排除 " +
                  (selection.data.excluded_count ?? 0) +
                  " 项"
                : ""}
            </span>
          </>
        )}
        <span className="grow" />
        {project && (
          <button onClick={() => setTasksVisible((v) => !v)}>
            <ListTodo size={13} />
            项目任务{activeJobs ? " · " + activeJobs + " 项运行中" : ""}
          </button>
        )}
      </footer>
      {dialog && (
        <ProjectDialog
          pickDirectory={chooseDirectory}
          kind={dialog}
          client={client}
          project={project}
          scopeOptions={inputs}
          defaultScope={selected > 0 ? "selection" : defaultScope}
          relinkSource={relinkTarget}
          onClose={() => setDialog(null)}
          onCreated={activate}
          onDone={() => {
            setDialog(null);
            void queryClient.invalidateQueries({
              queryKey: ["project", currentId],
            });
            if (dialog === "manifest") setTasksVisible(true);
          }}
        />
      )}
    </div>
  );
}
function SourceRow({
  source,
  active,
  onClick,
  onRelink,
}: {
  source: Source;
  active: boolean;
  onClick: () => void;
  onRelink: () => void;
}) {
  return (
    <div className="source-tree-row">
      <button
        className={"tree-row " + (active ? "active" : "")}
        onClick={onClick}
        title={source.issue ?? source.name}
      >
        <Database size={14} />
        <span>{source.name}</span>
        {source.available ? (
          <small className="source-state">●</small>
        ) : (
          <small className="source-offline">离线</small>
        )}
      </button>
      {source.kind === "danbooru" && (
        <button
          className="icon-button"
          title={"重新关联 " + source.name}
          onClick={onRelink}
        >
          <Link2 size={12} />
        </button>
      )}
    </div>
  );
}
