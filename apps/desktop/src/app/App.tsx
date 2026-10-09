import { sourceSupports } from "@studio/client";
import { useProjectSession } from "../features/projects/useProjectSession.js";
import { useProjectQueries } from "../features/query/useProjectQueries.js";
import { scopeOptions } from "../features/scopes/scopes.js";
import { ProjectDialog } from "../features/projects/ProjectDialog.js";
import type { DialogKind } from "../features/projects/ProjectDialog.js";
import {
  useEffect,
  useRef,
  useState,
  useCallback,
  Suspense,
  useSyncExternalStore,
  lazy,
} from "react";
import type { RefObject } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  FolderOpen,
  FolderPlus,
  FolderOutput,
  Plus,
  ChevronDown,
  ChevronRight,
  MousePointer2,
  PanelLeft,
  PanelRight,
  Home,
  Database,
  FileText,
  X,
  Layers,
  Info,
  RotateCw,
  Search,
  Undo2,
  Redo2,
  Filter,
  LocateFixed,
  SlidersHorizontal,
} from "lucide-react";
import {
  Button,
  Workbench,
  WorkbenchPreferences,
  useWorkbenchLayout,
  DraftStatus,
  Brand,
  CopyButton,
  assetTitle,
  ErrorDetails,
  isJobActive,
  MoreMenu,
  WorkbenchStatusTarget,
  NotificationStack,
  NotifyProvider,
  CommandPalette,
  jobPresentation,
  browseScopeIdentity,
} from "@studio/ui";
import { EditorTabs } from "./EditorTabs.js";
import type { Command, Notice, Notify, WorkbenchLayout } from "@studio/ui";
import { MenuBar } from "./MenuBar.js";
import type { MenuItems } from "./MenuBar.js";
import type { ModuleContext, BrowseScope } from "@studio/ui";
import type {
  Project,
  Asset,
  AssetKey,
  Source,
  QueryResult,
  ScopeOperation,
  ScopeRef,
} from "@studio/contracts";
import type { StudioClient, ObjectTarget } from "@studio/client";
import {
  connectEngine,
  chooseDirectory,
  onNativeClose,
} from "../platform/connection.js";
import { modules, moduleViews } from "./modules.js";
import { useWorkspaceState, useLayoutState } from "./workspaceState.js";
import { AssetImage } from "../features/browser/AssetImage.js";
import { Tasks } from "../features/tasks/Tasks.js";
import { TaskActivity } from "../features/tasks/TaskActivity.js";
import { MetadataInspector } from "../features/metadata/MetadataInspector.js";
import { SettingsDialog } from "../features/settings/SettingsDialog.js";
import { settingsPages } from "../features/settings/pages.js";
import type { SettingsPageId } from "../features/settings/pages.js";
import {
  ManagementPanel,
  InspectorTabs,
} from "../features/management/ManagementPanel.js";
import type { ManagementMode } from "../features/management/ManagementPanel.js";
import { WorksetTree } from "../features/management/WorksetTree.js";
import {
  ExportDialog,
  EXPORT_OPERATOR,
} from "../features/exports/ExportDialog.js";
import { DetachedSources } from "../features/management/DetachedSources.js";
import { CollectionMembersDialog } from "../features/worksets/CollectionMembersDialog.js";
import type { CollectionMembersIntent } from "../features/worksets/CollectionMembersDialog.js";
import { LakeActivity } from "../features/lake-updates/LakeActivity.js";
import {
  useLakePreference,
  useLakeStatus,
  useCollectionStatus,
} from "../features/lake-updates/queries.js";
import type { LakeInvocation } from "../features/lake-updates/LakeWorkspace.js";
const LakeWorkspace = lazy(
  () => import("../features/lake-updates/LakeWorkspace.js"),
);
const applicationInitial = { open: false, active: false };
function decodeApplication(value: unknown): typeof applicationInitial | null {
  if (!value || typeof value !== "object") return null;
  const v = value as typeof applicationInitial;
  return typeof v.open === "boolean" && typeof v.active === "boolean"
    ? v
    : null;
}

type ViewState = {
  scope: BrowseScope;
  focus: Asset | null;
  view: "grid" | "image";
};
const studioWorkbenchDefaults: WorkbenchLayout = {
  panels: {
    project: "left",
    inspector: "right",
    "browser.filters": "right",
    "browser.locate": "right",
  },
  active: {},
  leftWidth: 224,
  rightWidth: 380,
  bottomHeight: 260,
};
export function App() {
  const [closeError, setCloseError] = useState("");
  const [statusHost, setStatusHost] = useState<HTMLElement | null>(null);
  // Studio owns the notice stack; the provider sits here so feature modules
  // can raise notices without re-nesting Studio's render tree.
  const notifyHost = useRef<Notify>(() => {});
  const notify = useCallback<Notify>((n) => notifyHost.current(n), []);
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
    void onNativeClose(() => client.releaseProjectViews(), setCloseError).then(
      (stop) => {
        if (stopped) stop();
        else unlisten = stop;
      },
    );
    return () => {
      stopped = true;
      unlisten?.();
    };
  }, [engine.data]);
  if (!engine.data)
    return (
      <div className="connection-shell">
        <MenuBar />
        <div className="connection-screen">
          <Brand size={64} />
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
      </div>
    );
  return (
    <WorkbenchStatusTarget.Provider value={statusHost}>
      <NotifyProvider value={notify}>
        <Studio
          key={engine.data.connection.instance_id}
          client={engine.data}
          onReconnect={reconnect}
          onStatusHost={setStatusHost}
          notifyHost={notifyHost}
        />
      </NotifyProvider>
      {closeError && (
        <div className="error-banner" role="alert">
          <ErrorDetails error={closeError} compact />
          <button onClick={() => setCloseError("")}>关闭提示</button>
        </div>
      )}
    </WorkbenchStatusTarget.Provider>
  );
}
function Studio({
  client,
  onReconnect,
  onStatusHost,
  notifyHost,
}: {
  client: StudioClient;
  onReconnect: () => void;
  onStatusHost: (host: HTMLElement | null) => void;
  notifyHost: RefObject<Notify>;
}) {
  const queryClient = useQueryClient();
  const application = useLakePreference(
    client,
    "studio.application-workspace",
    applicationInitial,
    decodeApplication,
  );
  const lakesActive = application.value.active;
  const lakeStatus = useLakeStatus(client, lakesActive);
  const collectionStatus = useCollectionStatus(
    client,
    !!lakeStatus.data?.configured,
  );
  const [lakeInvocation, setLakeInvocation] = useState<LakeInvocation | null>(
    null,
  );
  function openLakes(
    jobId?: string,
    lakeId?: string,
    view?: "jobs" | "preparations",
    family?: "update" | "collection",
  ) {
    if (!application.editable) return;
    application.controller.set({ open: true, active: true });
    if (jobId || lakeId || view)
      setLakeInvocation((old) => ({
        sequence: (old?.sequence ?? 0) + 1,
        ...(jobId ? { jobId } : {}),
        ...(lakeId ? { lakeId } : {}),
        ...(view ? { view } : {}),
        ...(family ? { family } : {}),
      }));
    else setLakeInvocation(null);
  }
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
  const [dialog, setDialog] = useState<DialogKind>(null);
  // Scope option value the export dialog opens with; null when closed.
  const [exportScope, setExportScope] = useState<string | null>(null);
  const [memberEdit, setMemberEdit] = useState<
    (CollectionMembersIntent & { projectId: string }) | null
  >(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [settingsPage, setSettingsPage] = useState<SettingsPageId | null>(null);
  const [error, setError] = useState("");
  const [operationNotice, setOperationNotice] = useState("");
  const [managementAction, setManagementAction] = useState({
    mode: "details" as ManagementMode,
    sequence: 0,
  });
  const [taskFocus, setTaskFocus] = useState<string | null>(null);
  const [saving, setBusy] = useState(false);
  const session = useProjectSession(client, setError);
  const { project, projects } = session;
  const busy = saving || session.pending;
  const [relinkTarget, setRelinkTarget] = useState<Source | null>(null);
  const currentId = project?.id ?? "";
  useEffect(() => {
    setTaskFocus(null);
    setOperationNotice("");
  }, [currentId]);
  const currentIdRef = useRef(currentId);
  currentIdRef.current = currentId;
  const workspace = useWorkspaceState(client, currentId);
  const projectInfo = useQuery({
    queryKey: ["project", currentId, "info"],
    queryFn: () => client.project(currentId),
    enabled: !!currentId,
  });
  useEffect(() => {
    if (
      projectInfo.data &&
      projectInfo.data.id === currentId &&
      projectInfo.data.name !== project?.name
    )
      session.updateCurrent(projectInfo.data);
  }, [projectInfo.data, currentId, project?.name, session.updateCurrent]);
  const layout = useLayoutState(client);
  const studioWorkbench = useWorkbenchLayout(
    client,
    "studio",
    studioWorkbenchDefaults,
  );
  const [resourcesOpen, setResourcesOpen] = useState(false);
  useEffect(
    () => setResourcesOpen(false),
    [currentId, workspace.value.moduleId],
  );
  const { projectsVisible, propertiesVisible, tasksVisible } = layout.value;
  const queryVisible =
    workspace.value.moduleId === "core.browser" &&
    workspace.value.panels.includes("core.query");
  const [invocation, setInvocation] = useState<{
    projectId: string;
    sequence: number;
    args: Record<string, string>;
  } | null>(null);
  useSyncExternalStore(client.edits.subscribe, client.edits.getSnapshot);
  // Include application preferences: a saved project must not mask a pending layout write.
  const draftState = client.edits.status();
  const focusQuery = useQuery({
    queryKey: ["project", currentId, "asset", workspace.value.focusKey],
    queryFn: ({ signal }) =>
      client.asset(currentId, workspace.value.focusKey!, signal),
    enabled: !!currentId && !!workspace.value.focusKey && workspace.editable,
    retry: false,
  });
  const view: ViewState = {
    scope: workspace.value.scope,
    view: workspace.value.view,
    focus: focusQuery.data ?? null,
  };
  function setProjectsVisible(value: boolean | ((old: boolean) => boolean)) {
    layout.update({
      projectsVisible:
        typeof value === "function" ? value(projectsVisible) : value,
    });
  }
  function setPropertiesVisible(value: boolean | ((old: boolean) => boolean)) {
    const visible =
      typeof value === "function" ? value(propertiesVisible) : value;
    if (visible) revealStudioPanel("inspector");
    layout.update({
      propertiesVisible: visible,
    });
  }
  function revealStudioPanel(id: string) {
    studioWorkbench.update((old) => {
      const saved = old.panels[id];
      const position = saved && saved !== "hidden" ? saved : "right";
      return {
        ...old,
        panels: { ...old.panels, [id]: position },
        active: { ...old.active, [position]: id },
      };
    });
  }
  function setTasksVisible(value: boolean | ((old: boolean) => boolean)) {
    layout.update({
      tasksVisible: typeof value === "function" ? value(tasksVisible) : value,
    });
  }
  function setQueryVisible(value: boolean | ((old: boolean) => boolean)) {
    const show = typeof value === "function" ? value(queryVisible) : value;
    if (show) revealStudioPanel("core.query");
    if (workspace.editable)
      workspace.controller?.set((v) => ({
        ...v,
        ...(show ? { moduleId: "core.browser" } : {}),
        panels: show
          ? [...v.panels.filter((id) => id !== "core.query"), "core.query"]
          : v.panels.filter((id) => id !== "core.query"),
      }));
  }
  function activateView(id: string, args: Record<string, string> = {}) {
    if (id === "app.lakes") {
      openLakes();
      return;
    }
    application.controller.set((old) => ({ ...old, active: false }));
    if (id === "core.resources") {
      setSettingsPage("cache");
      return;
    }
    if (!workspace.editable) return;
    if (moduleViews.get(id)?.kind !== "view") {
      setError("该功能视图尚不可用。");
      return;
    }
    workspace.controller?.set((v) => ({
      ...v,
      moduleId: id,
      openViews: [...new Set([...v.openViews, id])],
    }));
    setInvocation((old) => ({
      projectId: currentId,
      sequence: (old?.sequence ?? 0) + 1,
      args,
    }));
  }
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
  const selectionHistory = useQuery({
    queryKey: ["project", currentId, "selection-history"],
    queryFn: ({ signal }) => client.management.history(currentId, signal),
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
    refetchInterval: (query) =>
      query.state.data?.items.some(isJobActive) ? 2000 : 5000,
    refetchIntervalInBackground: true,
  });
  const [notices, setNotices] = useState<Notice[]>([]);
  const jobStatuses = useRef({
    projectId: "",
    statuses: new Map<string, string>(),
  });
  useEffect(() => {
    const items = jobs.data?.items;
    if (!items) return;
    const seen = jobStatuses.current;
    // The first page of a project only seeds state: notify on transitions
    // observed in this session, never for jobs that finished earlier.
    const seeding = seen.projectId !== currentId;
    if (seeding) seen.statuses = new Map();
    seen.projectId = currentId;
    const finished: Notice[] = [];
    for (const job of items) {
      const previous = seen.statuses.get(job.id);
      seen.statuses.set(job.id, job.status);
      if (
        seeding ||
        !previous ||
        !isJobActive({ status: previous }) ||
        isJobActive(job)
      )
        continue;
      const p = jobPresentation(job);
      const exported = job.operator === EXPORT_OPERATOR;
      finished.push({
        id: job.id + ":" + job.status,
        tone: p.succeeded
          ? "success"
          : job.status === "failed"
            ? "error"
            : "info",
        title: exported
          ? p.succeeded
            ? "原图导出完成"
            : job.status === "failed"
              ? "原图导出未完成"
              : "原图导出已取消"
          : p.title,
        detail: p.succeeded
          ? exported
            ? job.total.toLocaleString("zh-CN") + " 项已写入目标文件夹"
            : p.count
          : exported && job.error
            ? job.error
            : p.detail,
        action:
          exported && p.succeeded
            ? {
                label: "打开文件夹",
                run: () =>
                  void client.revealExport(currentId, job.id).catch(() => {}),
              }
            : {
                label: "查看任务",
                run: () => {
                  setTaskFocus(job.id);
                  setTasksVisible(true);
                },
              },
      });
    }
    if (finished.length)
      setNotices((old) =>
        [
          ...old.filter((n) => !finished.some((f) => f.id === n.id)),
          ...finished,
        ].slice(-4),
      );
  }, [jobs.data, currentId]);
  useEffect(() => setNotices([]), [currentId]);
  const notify = useCallback<Notify>((notice) => {
    const id = notice.id ?? crypto.randomUUID();
    setNotices((old) =>
      [...old.filter((n) => n.id !== id), { ...notice, id }].slice(-4),
    );
  }, []);
  useEffect(() => {
    notifyHost.current = notify;
  }, [notifyHost, notify]);
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
  function editCollectionMembers(
    operation: "add" | "remove",
    keys?: AssetKey[],
  ) {
    const parent = activeResult.data?.spec.input_scope?.target;
    const collectionId =
      view.scope.kind === "collection"
        ? view.scope.id
        : parent?.kind === "workset"
          ? parent.collection_id
          : undefined;
    setMemberEdit({
      projectId: currentId,
      operation,
      ...(keys ? { keys } : {}),
      ...(operation === "remove" && collectionId ? { collectionId } : {}),
      scopeValue: selected > 0 ? "selection" : defaultScope,
    });
  }
  function targetForScope(scope: BrowseScope): ObjectTarget {
    if (scope.kind === "source") return { kind: "source", id: scope.id };
    if (scope.kind === "collection") return { kind: "workset", id: scope.id };
    if (scope.kind === "result") return { kind: "query_result", id: scope.id };
    if (scope.kind === "selection")
      return { kind: "selection", id: "selection" };
    return { kind: "project", id: currentId };
  }
  // Back/forward history of browse scopes, kept per project for this session.
  const scopeTrail = useRef({
    projectId: "",
    navigating: false,
    back: [] as BrowseScope[],
    forward: [] as BrowseScope[],
  });
  const [, setTrailRevision] = useState(0);
  if (scopeTrail.current.projectId !== currentId)
    scopeTrail.current = {
      projectId: currentId,
      navigating: false,
      back: [],
      forward: [],
    };
  const sameScope = (a: BrowseScope, b: BrowseScope) =>
    JSON.stringify(browseScopeIdentity(a)) ===
    JSON.stringify(browseScopeIdentity(b));
  function stepScope(direction: "back" | "forward") {
    const trail = scopeTrail.current;
    const next = trail[direction].pop();
    if (!next || !workspace.editable) return;
    (direction === "back" ? trail.forward : trail.back).push(
      workspace.value.scope,
    );
    trail.navigating = true;
    try {
      updateView({ scope: next });
    } finally {
      trail.navigating = false;
    }
    setTrailRevision((n) => n + 1);
  }
  function updateView(update: Partial<ViewState>) {
    if (!workspace.editable) return;
    const trail = scopeTrail.current;
    if (
      update.scope &&
      !trail.navigating &&
      !sameScope(update.scope, workspace.value.scope)
    ) {
      trail.back = [...trail.back, workspace.value.scope].slice(-50);
      trail.forward = [];
      setTrailRevision((n) => n + 1);
    }
    if (update.focus)
      queryClient.setQueryData(
        ["project", currentId, "asset", update.focus.key],
        update.focus,
      );
    workspace.controller?.set((v) => ({
      ...v,
      ...(update.scope
        ? {
            scope: update.scope,
            ...(v.inspectorTab === "management"
              ? { managementTarget: targetForScope(update.scope) }
              : {}),
            focusKey: null,
            view: "grid" as const,
            position: v.position
              ? {
                  scopeKey: "",
                  cursor: null,
                  pageNumber: 1,
                  pageSize: v.position.pageSize,
                  anchor: null,
                  version: null,
                }
              : null,
            moduleId: "core.browser",
            panels: queryVisible
              ? v.panels
              : v.panels.filter((id) => id !== "core.query"),
          }
        : {}),
      ...(update.view ? { view: update.view, moduleId: "core.browser" } : {}),
      ...("focus" in update ? { focusKey: update.focus?.key ?? null } : {}),
    }));
  }
  useEffect(() => {
    if (!workspace.editable || !workspace.controller) return;
    const scope = workspace.value.scope;
    const items =
      scope.kind === "source"
        ? sources.data?.items
        : scope.kind === "collection"
          ? collections.data?.items
          : undefined;
    if (!items || (scope.kind !== "source" && scope.kind !== "collection"))
      return;
    const current = items.find((item) => item.id === scope.id);
    if (!current) {
      workspace.controller.set((old) => ({
        ...old,
        scope: { kind: "all" },
        focusKey: null,
        position: null,
        view: "grid",
      }));
      setOperationNotice(
        scope.kind === "source"
          ? "数据湖已取消关联，已返回项目数据。"
          : "工作集已删除，已返回项目数据。",
      );
    } else if (current.name !== scope.name)
      workspace.controller.set((old) => ({
        ...old,
        scope: { ...scope, name: current.name },
      }));
  }, [
    workspace.editable,
    workspace.controller,
    workspace.value.scope,
    sources.data,
    collections.data,
  ]);
  useEffect(() => {
    if (!workspace.editable || !workspace.controller) return;
    const value = workspace.value;
    if (
      moduleViews.get(value.moduleId)?.kind !== "view" ||
      value.panels.some((id) => moduleViews.get(id)?.kind !== "panel")
    ) {
      workspace.controller.set((v) => ({
        ...v,
        moduleId:
          moduleViews.get(v.moduleId)?.kind === "view"
            ? v.moduleId
            : "core.browser",
        panels: v.panels.filter((id) => moduleViews.get(id)?.kind === "panel"),
      }));
      setError("部分已保存功能暂不可用，已恢复到可用视图。");
    }
  }, [workspace.controller, workspace.editable, workspace.value]);
  useEffect(() => {
    if (
      focusQuery.error &&
      workspace.editable &&
      workspace.controller &&
      workspace.value.focusKey
    ) {
      workspace.controller.set((v) => ({ ...v, focusKey: null, view: "grid" }));
      setError(
        "已保存的焦点对象暂不可用，已返回网格：" + focusQuery.error.message,
      );
    }
  }, [
    focusQuery.error,
    workspace.controller,
    workspace.editable,
    workspace.value.focusKey,
  ]);
  useEffect(() => {
    if (
      workspace.value.moduleId === "core.browser" &&
      workspace.value.scope.kind === "result" &&
      activeResult.data &&
      activeResult.data.id === workspace.value.scope.id &&
      activeResult.data.state !== "ready" &&
      workspace.editable &&
      workspace.controller
    ) {
      workspace.controller.set((v) => ({
        ...v,
        scope: { kind: "all" },
        focusKey: null,
        position: null,
        view: "grid",
      }));
      setError("已保存的查询结果已释放或尚未完成，已返回项目数据。");
    }
  }, [
    activeResult.data,
    workspace.controller,
    workspace.editable,
    workspace.value.moduleId,
    workspace.value.scope,
  ]);
  function activate(p: Project) {
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
        if (event.kind.startsWith("draft.")) return;
        if (event.kind.startsWith("job.")) {
          void queryClient.invalidateQueries({ queryKey: [...prefix, "jobs"] });
          void queryClient.invalidateQueries({
            queryKey: [...prefix, "artifacts"],
          });
          void queryClient.invalidateQueries({
            queryKey: [...prefix, "fields"],
          });
        } else if (event.kind.startsWith("selection.")) {
          void queryClient.invalidateQueries({
            queryKey: [...prefix, "selection"],
          });
          void queryClient.invalidateQueries({
            queryKey: [...prefix, "selection-members"],
          });
          void queryClient.invalidateQueries({
            queryKey: [...prefix, "selection-history"],
          });
        } else if (event.kind.startsWith("preset.")) {
          void queryClient.invalidateQueries({
            queryKey: [...prefix, "presets"],
          });
        } else void queryClient.invalidateQueries({ queryKey: prefix });
      },
      abort.signal,
    );
    return () => abort.abort();
  }, [client, currentId, queryClient, session.close]);
  async function act(
    action: () => Promise<unknown>,
    pid = currentId,
    changed: "project" | "selection" = "project",
  ) {
    setBusy(true);
    setError("");
    const refresh = () =>
      changed === "selection"
        ? Promise.all(
            ["selection", "selection-members", "selection-history"].map(
              (kind) =>
                queryClient.invalidateQueries({
                  queryKey: ["project", pid, kind],
                }),
            ),
          )
        : queryClient.invalidateQueries({ queryKey: ["project", pid] });
    try {
      await action();
      if (pid) await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      if (pid) void refresh();
    } finally {
      setBusy(false);
    }
  }
  function pick(keys: AssetKey[], remove = false) {
    const revision = selection.data?.revision;
    if (revision === undefined || busy) return;
    void act(
      () =>
        client.changeSelection(currentId, {
          expected_revision: revision,
          add: remove ? [] : keys,
          remove: remove ? keys : [],
          clear: false,
        }),
      currentId,
      "selection",
    );
  }
  function operateScope(scope: ScopeRef, operation: ScopeOperation) {
    if (selection.data?.revision === undefined || busy) return;
    void act(
      () =>
        client.changeSelectionScope(currentId, {
          expected_revision: selection.data!.revision,
          scope,
          operation,
        }),
      currentId,
      "selection",
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
    void act(
      () =>
        client.changeSelection(currentId, {
          expected_revision: revision,
          add: [],
          remove: [],
          clear: true,
        }),
      currentId,
      "selection",
    );
  }
  const historyBusy = useRef(false);
  function restoreSelection(action: "undo" | "redo") {
    if (
      !currentId ||
      lakesActive ||
      busy ||
      historyBusy.current ||
      selection.data?.revision === undefined ||
      !(action === "undo"
        ? selectionHistory.data?.undo_steps
        : selectionHistory.data?.redo_steps)
    )
      return;
    historyBusy.current = true;
    void act(
      async () => {
        const result = await client.management.restore(
          currentId,
          action,
          selection.data!.revision,
        );
        queryClient.setQueryData(
          ["project", currentId, "selection"],
          result.selection,
        );
        queryClient.setQueryData(
          ["project", currentId, "selection-history"],
          result,
        );
        setOperationNotice(
          action === "undo" ? "已撤销选择操作。" : "已重做选择操作。",
        );
      },
      currentId,
      "selection",
    ).finally(() => {
      historyBusy.current = false;
    });
  }
  const historyKeyboard = useRef(restoreSelection);
  historyKeyboard.current = restoreSelection;
  useEffect(() => {
    const keyboard = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        event.isComposing ||
        event.altKey ||
        !(event.ctrlKey || event.metaKey)
      )
        return;
      if (
        event.target instanceof Element &&
        event.target.closest(
          "input,textarea,select,[contenteditable=true],[role=dialog],[role=menu]",
        )
      )
        return;
      if (document.querySelector("[role=dialog]")) return;
      const key = event.key.toLowerCase();
      if (key === "z" || (key === "y" && !event.shiftKey)) {
        event.preventDefault();
        historyKeyboard.current(
          key === "y" || event.shiftKey ? "redo" : "undo",
        );
      }
    };
    document.addEventListener("keydown", keyboard);
    return () => document.removeEventListener("keydown", keyboard);
  }, []);
  function openManagement(
    target: ObjectTarget,
    mode: ManagementMode = "details",
  ) {
    if (!workspace.editable) return;
    setManagementAction((old) => ({ mode, sequence: old.sequence + 1 }));
    workspace.controller?.set((old) => ({
      ...old,
      inspectorTab: "management",
      managementTarget: target,
    }));
    setPropertiesVisible(true);
  }
  function setInspectorTab(tab: "properties" | "management") {
    if (!workspace.editable) return;
    workspace.controller?.set((old) => ({
      ...old,
      inspectorTab: tab,
      managementTarget: old.managementTarget ?? targetForScope(old.scope),
    }));
  }
  function browseManaged(target: ObjectTarget) {
    void act(async () => {
      const item = await client.management.details(currentId, target);
      if (target.kind === "source")
        updateView({
          scope: { kind: "source", id: target.id, name: item.object.name },
        });
      if (target.kind === "workset")
        updateView({
          scope: { kind: "collection", id: target.id, name: item.object.name },
        });
      if (target.kind === "selection")
        updateView({ scope: { kind: "selection", name: "当前选择" } });
      if (target.kind === "artifact")
        activateView("core.artifacts", { artifactId: target.id });
      if (target.kind === "query_result") {
        const result = await client.queries.result(currentId, target.id);
        if (result.state === "ready")
          updateView({
            scope: { kind: "result", id: target.id, name: item.object.name },
          });
        else setQueryVisible(true);
      }
      if (target.kind === "query") {
        setQueryVisible(true);
        setInvocation((old) => ({
          projectId: currentId,
          sequence: (old?.sequence ?? 0) + 1,
          args: { queryId: target.id },
        }));
      }
      if (target.kind === "job") {
        setTaskFocus(target.id);
        setTasksVisible(true);
      }
    });
  }
  function objectChanged(target: ObjectTarget, action: string) {
    if (currentIdRef.current !== currentId) return;
    if (target.kind === "project")
      void queryClient.invalidateQueries({ queryKey: ["projects"] });
    if (action === "remove") {
      setOperationNotice(
        target.kind === "source"
          ? "已取消数据湖与当前项目的关联。"
          : "对象已删除，相关引用已更新。",
      );
      if (target.kind === "workset" || target.kind === "job")
        openManagement({ kind: "project", id: currentId });
      if (
        target.kind === "source" &&
        workspace.value.focusKey?.source_id === target.id
      )
        workspace.controller?.set((old) => ({
          ...old,
          focusKey: null,
          view: "grid",
        }));
    }
  }
  const managementTarget =
    workspace.value.managementTarget ?? targetForScope(view.scope);
  const inspectorHeader = (
    <InspectorTabs
      tab={workspace.value.inspectorTab}
      onChange={setInspectorTab}
    />
  );
  const managementContent = currentId ? (
    <ManagementPanel
      key={currentId + managementTarget.kind + managementTarget.id}
      client={client}
      projectId={currentId}
      target={managementTarget}
      mode={managementAction.mode}
      sequence={managementAction.sequence}
      onNavigate={openManagement}
      onBrowse={browseManaged}
      onReuse={(run) => {
        activateView("core.tools", {
          operatorId: run.operator_id,
          reuseRun: JSON.stringify(run),
        });
        setInspectorTab("properties");
      }}
      onChanged={objectChanged}
      onSettings={() => setSettingsPage("editing")}
    />
  ) : null;
  const browserTarget: ScopeRef["target"] | null =
    view.scope.kind === "result"
      ? { kind: "query_result", result_id: view.scope.id }
      : view.scope.kind === "collection"
        ? { kind: "workset", collection_id: view.scope.id }
        : view.scope.kind === "selection" && selection.data
          ? { kind: "selection", revision: selection.data.revision }
          : null;
  const browserScopeSources = useQuery({
    queryKey: [
      "project",
      currentId,
      "browser-sources",
      browserTarget,
      browserTarget?.kind === "workset"
        ? collections.data?.items.find(
            (c) => c.id === browserTarget.collection_id,
          )?.count
        : null,
    ],
    queryFn: ({ signal }) =>
      client.sourceAccess.requirements(
        currentId,
        { project_id: currentId, target: browserTarget! },
        [],
        signal,
      ),
    enabled: !!project && !!browserTarget,
  });
  // Results, worksets and selections use their own sources, not every lake
  // attached to the project. An unrelated Pixiv lake must not change Booru order.
  const browserSourceIds =
    view.scope.kind === "source"
      ? [view.scope.id]
      : view.scope.kind === "all"
        ? (sources.data?.items.map((source) => source.id) ?? [])
        : (browserScopeSources.data?.sources.map(
            (source) => source.source_id,
          ) ?? []);
  const browserSourcesPending =
    sources.isPending ||
    (!!browserTarget && browserScopeSources.isPending) ||
    (view.scope.kind === "selection" && selection.isPending);
  const browserSourcesError =
    sources.error ??
    (browserTarget ? browserScopeSources.error : null) ??
    (view.scope.kind === "selection" ? selection.error : null);
  const booruBrowse =
    browserSourceIds.length > 0 &&
    browserSourceIds.every((id) => {
      const source = sources.data?.items.find((source) => source.id === id);
      return !!source && sourceSupports(source, "post_order");
    });
  const browserOrder = booruBrowse
    ? workspace.value.booruOrder
    : workspace.value.order.startsWith("post_id_")
      ? "asset_key_asc"
      : workspace.value.order;
  const moduleContext: ModuleContext = {
    client,
    projectId: currentId,
    sources: sources.data?.items ?? [],
    inputOptions: inputs,
    defaultInput: selected > 0 ? "selection" : defaultScope,
    browser: {
      order: browserOrder,
      postOrderAllowed: booruBrowse,
      rankedBrowse: workspace.value.rankedBrowse,
      onRankedBrowse: (rankedBrowse) => {
        if (workspace.editable)
          workspace.controller?.set((v) => {
            const previous = v.rankedBrowse;
            const changed =
              !previous ||
              previous.scopeKey !== rankedBrowse.scopeKey ||
              previous.sort !== rankedBrowse.sort ||
              previous.descending !== rankedBrowse.descending ||
              previous.startPostId !== rankedBrowse.startPostId ||
              previous.startRank !== rankedBrowse.startRank ||
              previous.startRating !== rankedBrowse.startRating;
            return {
              ...v,
              rankedBrowse,
              position: changed ? null : v.position,
            };
          });
      },
      onOrder: (order) => {
        if (workspace.editable)
          workspace.controller?.set((v) => ({
            ...v,
            ...(booruBrowse ? { booruOrder: order } : { order }),
            position: null,
          }));
      },
      scope: view.scope,
      focus: view.focus,
      focusPending: !!workspace.value.focusKey && focusQuery.isPending,
      onFocus: (focus) => updateView({ focus }),
      onInspect: (focus) => {
        updateView({ focus });
        setInspectorTab("properties");
        setPropertiesVisible(true);
      },
      onScope: (scope) => updateView({ scope }),
      onPick: pick,
      onScopeOperation: operateViewScope,
      selectionRevision: selection.data?.revision ?? 0,
      collectionRevision:
        view.scope.kind === "collection"
          ? (collections.data?.items.find(
              (c) => view.scope.kind === "collection" && c.id === view.scope.id,
            )?.revision ?? 0)
          : 0,
      onCollectionMembers: editCollectionMembers,
      busy,
      view: view.view,
      setView: (mode) => updateView({ view: mode }),
      position: workspace.value.position,
      onPosition: (position) => {
        if (workspace.editable)
          workspace.controller?.set((v) => ({ ...v, position }));
      },
      thumbnailSize: layout.value.thumbnailSize,
      onThumbnailSize: (thumbnailSize) => layout.update({ thumbnailSize }),
      navigation: {
        back: scopeTrail.current.back.length ? () => stepScope("back") : null,
        forward: scopeTrail.current.forward.length
          ? () => stepScope("forward")
          : null,
      },
    },
    onResult: (result, name) => {
      if (currentIdRef.current === result.project_id)
        updateView({ scope: { kind: "result", id: result.id, name } });
    },
    onSelect: selectResult,
    onJob: (job, options) => {
      if (
        currentIdRef.current === job.project_id &&
        options?.revealTasks !== false
      )
        setTasksVisible(true);
    },
    activateView,
    openSettings: setSettingsPage,
    openPanel: (id) => {
      revealStudioPanel(id);
      if (workspace.editable)
        workspace.controller?.set((v) => ({
          ...v,
          moduleId: id === "core.query" ? "core.browser" : v.moduleId,
          panels: [...v.panels.filter((p) => p !== id), id],
        }));
    },
    togglePanel: (id) => {
      if (!workspace.value.panels.includes(id)) revealStudioPanel(id);
      if (workspace.editable)
        workspace.controller?.set((v) => ({
          ...v,
          panels: v.panels.includes(id)
            ? v.panels.filter((p) => p !== id)
            : [...v.panels, id],
        }));
    },
    closePanel: (id) => {
      if (workspace.editable)
        workspace.controller?.set((v) => ({
          ...v,
          panels: v.panels.filter((p) => p !== id),
        }));
    },
    panels:
      workspace.value.moduleId === "core.browser" ? workspace.value.panels : [],
    panelHeight: () => layout.value.queryHeight,
    resizePanel: (id, height) => {
      if (id === "core.query") layout.update({ queryHeight: height });
    },
    inspector: {
      visible: propertiesVisible,
      width: layout.value.propertiesWidth,
      setVisible: setPropertiesVisible,
      resize: (propertiesWidth) => layout.update({ propertiesWidth }),
    },
    management: {
      tab: workspace.value.inspectorTab,
      open: openManagement,
      showProperties: () => setInspectorTab("properties"),
      header: inspectorHeader,
      content: managementContent,
    },
    invocation:
      invocation?.projectId === currentId
        ? { sequence: invocation.sequence, args: invocation.args }
        : null,
  };
  const activeSurface = moduleViews.get(workspace.value.moduleId);
  const ActiveModule = activeSurface?.Component;
  const openViews = [
    ...new Set([...workspace.value.openViews, workspace.value.moduleId]),
  ].filter(
    (id) => moduleViews.get(id)?.kind === "view" && id !== "core.resources",
  );
  function closeView(ids: string | string[]) {
    const closing = new Set(typeof ids === "string" ? [ids] : ids);
    if (closing.delete("app.lakes"))
      application.controller.set({ open: false, active: false });
    if (!closing.size || !workspace.editable) return;
    workspace.controller?.set((value) => {
      const remaining = openViews.filter((viewId) => !closing.has(viewId));
      if (!remaining.length) remaining.push("core.browser");
      return {
        ...value,
        openViews: remaining,
        moduleId: closing.has(value.moduleId)
          ? remaining[remaining.length - 1]!
          : value.moduleId,
      };
    });
  }
  const tabOrder = [
    ...(project ? openViews : []),
    ...(application.value.open ? ["app.lakes"] : []),
  ];
  const activeTab = lakesActive ? "app.lakes" : workspace.value.moduleId;
  const projectWorkbenchShown =
    !!project && !lakesActive && !activeSurface?.ownsWorkbench;
  function resetStudioLayout() {
    studioWorkbench.reset();
    layout.update({ projectsVisible: true, propertiesVisible: true });
  }
  function refreshProject() {
    void queryClient.invalidateQueries({ queryKey: ["project", currentId] });
  }
  const shortcuts = useRef<(event: KeyboardEvent) => void>(() => {});
  shortcuts.current = (event) => {
    const key = event.key.toLowerCase();
    const ctrl = (event.ctrlKey || event.metaKey) && !event.altKey;
    // F5 / Ctrl+R would reload the WebView and drop unsaved editor state.
    if (key === "f5" || (ctrl && !event.shiftKey && key === "r")) {
      event.preventDefault();
      if (project && !lakesActive && !document.querySelector("dialog[open]"))
        refreshProject();
      return;
    }
    if (
      event.altKey &&
      !event.ctrlKey &&
      (event.key === "ArrowLeft" || event.key === "ArrowRight") &&
      browsing &&
      !(event.target as Element | null)?.closest?.(
        "input,textarea,select,[contenteditable=true]",
      )
    ) {
      event.preventDefault();
      stepScope(event.key === "ArrowLeft" ? "back" : "forward");
      return;
    }
    if (!ctrl || event.defaultPrevented || event.isComposing) return;
    if (document.querySelector("dialog[open],[aria-modal=true]")) return;
    if (key === "tab" && tabOrder.length > 1) {
      event.preventDefault();
      const index = tabOrder.indexOf(activeTab);
      activateView(
        tabOrder[
          (index + (event.shiftKey ? -1 : 1) + tabOrder.length) %
            tabOrder.length
        ]!,
      );
    } else if (key === "p") {
      event.preventDefault();
      setPaletteOpen(true);
    } else if (key === "w" && !event.shiftKey && tabOrder.length) {
      event.preventDefault();
      closeView(activeTab);
    } else if (
      key === " " &&
      project &&
      !(event.target as Element | null)?.closest?.(
        "input,textarea,select,[contenteditable=true]",
      )
    ) {
      event.preventDefault();
      setResourcesOpen((v) => !v);
    }
  };
  const browsing =
    !!project && !lakesActive && workspace.value.moduleId === "core.browser";
  const mouseHistory = useRef<(event: MouseEvent) => void>(() => {});
  mouseHistory.current = (event) => {
    // Mouse back/forward buttons step through browse scopes; elsewhere they
    // must not navigate the WebView itself.
    if (event.button !== 3 && event.button !== 4) return;
    event.preventDefault();
    if (browsing && !document.querySelector("dialog[open]"))
      stepScope(event.button === 3 ? "back" : "forward");
  };
  useEffect(() => {
    const keyboard = (event: KeyboardEvent) => shortcuts.current(event);
    const mouse = (event: MouseEvent) => mouseHistory.current(event);
    const block = (event: MouseEvent) => {
      if (event.button === 3 || event.button === 4) event.preventDefault();
    };
    document.addEventListener("keydown", keyboard);
    document.addEventListener("mouseup", mouse);
    document.addEventListener("mousedown", block);
    return () => {
      document.removeEventListener("keydown", keyboard);
      document.removeEventListener("mouseup", mouse);
      document.removeEventListener("mousedown", block);
    };
  }, []);
  function paletteCommands(): Command[] {
    const commands: Command[] = Object.entries(menus).flatMap(
      ([group, items]) =>
        items.map((item) => ({
          id: group + ":" + item.label,
          label: item.label.replace(/…$/, ""),
          group,
          run: item.action,
          ...(item.shortcut ? { shortcut: item.shortcut } : {}),
          ...(item.checked !== undefined ? { checked: item.checked } : {}),
          ...(item.disabled ? { disabled: true } : {}),
        })),
    );
    if (!project || lakesActive) return commands;
    const browse = (scope: BrowseScope) => () => updateView({ scope });
    for (const source of sources.data?.items ?? [])
      commands.push({
        id: "source:" + source.id,
        label: "浏览数据湖 · " + source.name,
        group: "数据湖",
        run: browse({ kind: "source", id: source.id, name: source.name }),
      });
    for (const item of collections.data?.items ?? [])
      commands.push(
        {
          id: "workset:" + item.id,
          label: "浏览工作集 · " + item.name,
          group: "工作集",
          run: browse({ kind: "collection", id: item.id, name: item.name }),
        },
        {
          id: "export:" + item.id,
          label: "导出工作集原图 · " + item.name,
          group: "工作集",
          run: () => setExportScope(item.id),
        },
      );
    return commands;
  }
  const menus: MenuItems = {
    项目: [
      { label: "新建项目…", action: () => setDialog("new") },
      { label: "打开项目…", action: () => setDialog("open") },
      {
        label: "添加数据湖…",
        action: () => setDialog("source"),
        disabled: !project,
        separator: true,
      },
      {
        label: "项目管理…",
        action: () => openManagement({ kind: "project", id: currentId }),
        disabled: !project,
      },
      {
        label: "打开项目文件夹",
        action: () =>
          void act(() =>
            client.management.reveal(currentId, {
              kind: "project",
              id: currentId,
            }),
          ),
        disabled: !project,
      },
      {
        label: "关闭当前项目",
        action: () => void session.close(),
        disabled: !project,
        separator: true,
      },
    ],
    编辑: [
      {
        label: "撤销选择",
        shortcut: "Ctrl+Z",
        action: () => restoreSelection("undo"),
        disabled: lakesActive || busy || !selectionHistory.data?.undo_steps,
      },
      {
        label: "重做选择",
        shortcut: "Ctrl+Y",
        action: () => restoreSelection("redo"),
        disabled: lakesActive || busy || !selectionHistory.data?.redo_steps,
      },
      {
        label: "清除当前选择",
        separator: true,
        action: clearSelection,
        disabled: lakesActive || !selected,
      },
      {
        label: "保存选择为工作集…",
        action: () => setDialog("collection"),
        disabled: lakesActive || !hasFixedInput,
      },
      {
        label: "导出原图…",
        action: () => setExportScope(selected > 0 ? "selection" : defaultScope),
        disabled: lakesActive || !hasTaskInput,
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
        shortcut: "F5",
        separator: true,
        action: refreshProject,
        disabled: !project || lakesActive,
      },
    ],
    工具: [
      { label: "数据湖", action: () => openLakes() },
      ...modules.entries().map((entry, index) => ({
        separator: index === 0,
        label: entry.label,
        action: () => modules.execute(entry.command, moduleContext),
        disabled: !project || !workspace.editable,
      })),
    ],
    窗口: [
      {
        label: "命令搜索…",
        shortcut: "Ctrl+P",
        action: () => setPaletteOpen(true),
      },
      {
        label: "项目资源抽屉",
        separator: true,
        shortcut: "Ctrl+Space",
        checked: resourcesOpen,
        action: () => setResourcesOpen((v) => !v),
        disabled: !project,
      },
      ...(projectWorkbenchShown
        ? [
            {
              label: "项目面板",
              checked: projectsVisible,
              action: () => setProjectsVisible((v) => !v),
            },
            {
              label: "属性面板",
              checked: propertiesVisible,
              action: () => setPropertiesVisible((v) => !v),
            },
          ]
        : []),
      {
        label: "项目任务",
        checked: tasksVisible,
        action: () => setTasksVisible((v) => !v),
        disabled: !project,
      },
      {
        label: "下一个标签",
        shortcut: "Ctrl+Tab",
        separator: true,
        action: () =>
          activateView(
            tabOrder[(tabOrder.indexOf(activeTab) + 1) % tabOrder.length]!,
          ),
        disabled: tabOrder.length < 2,
      },
      {
        label: "关闭当前标签",
        shortcut: "Ctrl+W",
        action: () => closeView(activeTab),
        disabled: !tabOrder.length,
      },
      ...(projectWorkbenchShown
        ? [
            {
              label: "恢复默认布局",
              separator: true,
              disabled: !studioWorkbench.editable,
              action: resetStudioLayout,
            },
          ]
        : []),
    ],
    设置: settingsPages.map(({ id, title }) => ({
      label: title + "…",
      action: () => setSettingsPage(id),
    })),
    帮助: [{ label: "关于 Dataset Studio", action: () => setDialog("about") }],
  };
  const projectPanel = project ? (
    <aside className="project-panel">
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
            "tree-row " + (view.scope.kind === "selection" ? "active" : "")
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
          <span className="grow" />
          <button
            type="button"
            title="添加数据湖"
            aria-label="添加数据湖"
            className="icon-button"
            onClick={() => setDialog("source")}
          >
            <Plus size={12} />
          </button>
        </div>
        {sources.data?.items.map((source) => (
          <SourceRow
            key={source.id}
            source={source}
            onUpdates={() => openLakes(undefined, source.id)}
            onManage={(mode) =>
              openManagement({ kind: "source", id: source.id }, mode)
            }
            onRelink={() => {
              setRelinkTarget(source);
              setDialog("relink");
            }}
            active={view.scope.kind === "source" && view.scope.id === source.id}
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
          <button className="tree-add" onClick={() => setDialog("source")}>
            <Plus size={13} />
            添加第一个数据湖
          </button>
        )}
        <DetachedSources
          key={"detached:" + currentId}
          client={client}
          projectId={currentId}
          onManage={openManagement}
        />
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
        <WorksetTree
          key={"worksets:" + currentId}
          client={client}
          projectId={currentId}
          activeId={view.scope.kind === "collection" ? view.scope.id : null}
          onBrowse={(item) =>
            updateView({
              scope: { kind: "collection", id: item.id, name: item.name },
            })
          }
          onManage={openManagement}
          onExport={(item) => setExportScope(item.id)}
        />
      </div>
      <div className="project-foot">
        <div className="project-foot-title">
          <strong>{project.name}</strong>
          <MoreMenu
            label="项目"
            contextMenu
            items={[
              {
                label: "项目管理与备注",
                action: () =>
                  openManagement({ kind: "project", id: currentId }),
              },
              {
                label: "重命名项目…",
                action: () =>
                  openManagement({ kind: "project", id: currentId }, "rename"),
              },
              {
                label: "打开项目文件夹",
                action: () =>
                  void act(() =>
                    client.management.reveal(currentId, {
                      kind: "project",
                      id: currentId,
                    }),
                  ),
              },
            ]}
          />
        </div>
        <span>
          {sources.data?.items.length ?? 0} 个数据湖 ·{" "}
          {collections.data?.items.length ?? 0} 个工作集
        </span>
      </div>
    </aside>
  ) : null;
  const propertiesPanel = project ? (
    <aside className="properties-panel">
      {inspectorHeader}
      {workspace.value.inspectorTab === "management" ? (
        propertiesVisible ? (
          managementContent
        ) : null
      ) : view.focus ? (
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
            <h3 className="property-asset-title">{assetTitle(view.focus)}</h3>
            <dl>
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
            <details className="identity-details">
              <summary>存储身份与关联</summary>
              <code>{view.focus.key.asset_id}</code>
              <CopyButton label="复制图像身份" text={view.focus.key.asset_id} />
              {view.focus.summary?.post_ids.length ? (
                <p>
                  关联帖子：
                  {view.focus.summary.post_ids.map((id) => "#" + id).join("、")}
                  {Number(view.focus.summary.post_count) >
                  view.focus.summary.post_ids.length
                    ? " 等 " + view.focus.summary.post_count + " 个"
                    : ""}
                </p>
              ) : null}
            </details>
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
              onFilterTag={(tag) => {
                setQueryVisible(true);
                setInvocation((old) => ({
                  projectId: currentId,
                  sequence: (old?.sequence ?? 0) + 1,
                  args: {
                    sourceId: view.focus!.key.source_id,
                    exactTag: JSON.stringify(tag),
                  },
                }));
              }}
              onFilterOrigin={(field, value) => {
                setQueryVisible(true);
                setInvocation((old) => ({
                  projectId: currentId,
                  sequence: (old?.sequence ?? 0) + 1,
                  args: {
                    sourceId: view.focus!.key.source_id,
                    exactField: field,
                    exactValue: value,
                  },
                }));
              }}
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
            <Button
              disabled={!selected || busy}
              onClick={() => editCollectionMembers("add")}
            >
              加入已有工作集…
            </Button>
            <Button
              disabled={!selected || busy}
              onClick={() => editCollectionMembers("remove")}
            >
              从工作集移除…
            </Button>
            <Button
              disabled={!selected || busy}
              onClick={() => setExportScope("selection")}
            >
              <FolderOutput size={14} />
              导出原图
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
  ) : null;
  const mainContent = project ? (
    <div className="data-workspace">
      {workspace.controller && (!workspace.editable || workspace.error) && (
        <DraftStatus controller={workspace.controller} />
      )}
      <Suspense fallback={<p className="tool-hint">正在载入功能…</p>}>
        {workspace.editable &&
          ActiveModule &&
          (workspace.value.moduleId === "core.browser" &&
          (browserSourcesPending || browserSourcesError) ? (
            browserSourcesError ? (
              <ErrorDetails error={browserSourcesError} />
            ) : (
              <p className="tool-hint" role="status">
                正在读取浏览范围…
              </p>
            )
          ) : (
            <ActiveModule
              key={workspace.value.moduleId + project.id}
              {...moduleContext}
            />
          ))}
      </Suspense>
    </div>
  ) : null;
  return (
    <div className="studio-app">
      <div className="app-header">
        <div className="app-brand" data-tauri-drag-region>
          <Brand size={40} />
        </div>
        <MenuBar
          menus={menus}
          busy={busy}
          title={project?.name ?? "Dataset Studio"}
          brand={false}
        />
        <DraftStatus controller={application.controller} quiet />
        {(project || application.value.open) && (
          <EditorTabs
            open={[
              ...(project ? openViews : []),
              ...(application.value.open ? ["app.lakes"] : []),
            ]}
            active={lakesActive ? "app.lakes" : workspace.value.moduleId}
            projectAvailable={!!project}
            disabled={!workspace.editable}
            onOpen={activateView}
            onClose={closeView}
          />
        )}
      </div>
      {project &&
        !lakesActive &&
        workspace.value.moduleId === "core.browser" && (
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
            <button
              className="icon-button"
              title="撤销选择（Ctrl+Z）"
              aria-label="撤销选择"
              disabled={busy || !selectionHistory.data?.undo_steps}
              onClick={() => restoreSelection("undo")}
            >
              <Undo2 size={15} />
            </button>
            <button
              className="icon-button"
              title="重做选择（Ctrl+Y）"
              aria-label="重做选择"
              disabled={busy || !selectionHistory.data?.redo_steps}
              onClick={() => restoreSelection("redo")}
            >
              <Redo2 size={15} />
            </button>
            <span className="option-separator" />
            <button
              disabled={!hasFixedInput || busy}
              onClick={() => setDialog("collection")}
              title="将当前浏览范围保存为工作集"
            >
              <FolderPlus size={14} />
              保存当前范围
            </button>
            <button
              disabled={!hasTaskInput || busy}
              onClick={() => editCollectionMembers("add")}
              title="将选择或当前范围加入已有工作集"
            >
              加入已有工作集…
            </button>
            <button
              disabled={!hasTaskInput || busy}
              onClick={() => editCollectionMembers("remove")}
              title="从工作集中移除选择或当前范围的图片"
            >
              从工作集移除…
            </button>
            <button
              disabled={!hasTaskInput || busy}
              onClick={() =>
                activateView("core.tools", { operatorId: "core.manifest" })
              }
            >
              <FileText size={14} />
              生成清单
            </button>
            <button
              disabled={!hasTaskInput || busy}
              onClick={() =>
                setExportScope(selected > 0 ? "selection" : defaultScope)
              }
              title="把当前范围或选择的原图导出到文件夹"
            >
              <FolderOutput size={14} />
              导出原图
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
        )}
      {lakesActive ? (
        <Suspense
          fallback={<p className="lake-empty">正在打开数据湖工作台…</p>}
        >
          <LakeWorkspace
            client={client}
            openSettings={() => setSettingsPage("lake-api")}
            invocation={lakeInvocation}
            {...(project
              ? { project: { id: project.id, name: project.name, inputs } }
              : {})}
          />
        </Suspense>
      ) : !project ? (
        <main className="start-screen">
          <div className="start-actions">
            <Brand size={64} />
            <h1>Dataset Studio</h1>
            <p>打开项目，继续你的数据工作。</p>
            <Button onClick={() => openLakes()}>
              <Database size={15} />
              管理数据湖更新
            </Button>
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
        <div className="studio-workspace">
          {activeSurface?.ownsWorkbench ? (
            mainContent
          ) : (
            <Workbench
              title="项目工作台"
              layout={{
                ...studioWorkbench.value,
                panels: {
                  ...studioWorkbenchDefaults.panels,
                  ...studioWorkbench.value.panels,
                  "core.query": queryVisible
                    ? studioWorkbench.value.panels["core.query"] === "hidden"
                      ? "right"
                      : (studioWorkbench.value.panels["core.query"] ?? "right")
                    : "hidden",
                  project: projectsVisible
                    ? studioWorkbench.value.panels.project === "hidden"
                      ? "left"
                      : (studioWorkbench.value.panels.project ?? "left")
                    : "hidden",
                  inspector:
                    propertiesVisible && !activeSurface?.ownsInspector
                      ? studioWorkbench.value.panels.inspector === "hidden"
                        ? "right"
                        : (studioWorkbench.value.panels.inspector ?? "right")
                      : "hidden",
                },
              }}
              onLayout={(next) => {
                studioWorkbench.update(next);
                if (queryVisible && next.panels["core.query"] === "hidden")
                  setQueryVisible(false);
                layout.update({
                  projectsVisible: next.panels.project !== "hidden",
                  propertiesVisible: next.panels.inspector !== "hidden",
                });
              }}
              panels={[
                {
                  id: "project",
                  title: "项目",
                  icon: <Layers size={13} />,
                  content: projectPanel,
                },
                ...(!activeSurface?.ownsInspector
                  ? [
                      {
                        id: "inspector",
                        title: "检查器",
                        icon: <SlidersHorizontal size={13} />,
                        content: propertiesPanel,
                      },
                    ]
                  : []),
                ...(workspace.value.moduleId === "core.browser"
                  ? [
                      {
                        id: "browser.filters",
                        title: "筛选",
                        icon: <Filter size={13} />,
                        portal: true,
                        defaultPosition: "right" as const,
                      },
                      {
                        id: "browser.locate",
                        title: "定位",
                        icon: <LocateFixed size={13} />,
                        portal: true,
                        defaultPosition: "right" as const,
                      },
                      ...workspace.value.panels.flatMap((id) => {
                        const Panel = moduleViews.get(id)?.Component;
                        return Panel
                          ? [
                              {
                                id,
                                title: id === "core.query" ? "项目查询" : id,
                                icon: <Search size={13} />,
                                content: (
                                  <Suspense
                                    fallback={
                                      <p className="tool-hint">正在载入面板…</p>
                                    }
                                  >
                                    <Panel
                                      key={id + project.id}
                                      {...moduleContext}
                                    />
                                  </Suspense>
                                ),
                              },
                            ]
                          : [];
                      }),
                    ]
                  : []),
              ]}
              disabled={!studioWorkbench.editable}
              status={
                <WorkbenchPreferences
                  state={{ ...studioWorkbench, reset: resetStudioLayout }}
                />
              }
            >
              {mainContent}
            </Workbench>
          )}
        </div>
      )}
      {project && resourcesOpen && (
        <section className="studio-resources-drawer" aria-label="项目资源抽屉">
          <header className="wb-panel-header">
            <strong>项目资源</strong>
            <span className="grow" />
            <button
              type="button"
              className="icon-button"
              aria-label="关闭项目资源"
              onClick={() => setResourcesOpen(false)}
            >
              <X size={14} />
            </button>
          </header>
          {projectPanel}
        </section>
      )}
      {project && tasksVisible && (
        <Tasks
          client={client}
          projectId={project.id}
          onClose={() => setTasksVisible(false)}
          onError={setError}
          focusJobId={taskFocus}
          onManage={openManagement}
          onOpenRanking={(jobId) => {
            setTasksVisible(false);
            activateView("core.tools", {
              operatorId: "danbooru.metarecall",
              jobId,
            });
          }}
        />
      )}
      {error && (
        <div className="error-banner" role="alert">
          <Info size={15} />
          <ErrorDetails error={error} compact />
          <button
            className="icon-button"
            aria-label="关闭错误提示"
            onClick={() => setError("")}
          >
            <X size={14} />
          </button>
        </div>
      )}
      <NotificationStack
        notices={notices}
        onDismiss={(id) => setNotices((old) => old.filter((n) => n.id !== id))}
      />
      <footer className="status-bar">
        {project && (
          <button
            type="button"
            className="resource-drawer-trigger"
            aria-expanded={resourcesOpen}
            onClick={() => setResourcesOpen((value) => !value)}
          >
            <FolderOpen size={14} />
            项目资源
          </button>
        )}
        <span className={health.error ? "online-dot offline" : "online-dot"} />
        <span>{health.error ? "本机引擎已断开" : "本机引擎已连接"}</span>
        {operationNotice && (
          <span role="status" className="operation-notice">
            {operationNotice}
          </span>
        )}
        {health.error && <button onClick={onReconnect}>重新连接</button>}
        {project && (
          <>
            <span className="status-divider" />
            <span>
              {busy || draftState.saving
                ? "正在保存项目"
                : draftState.error
                  ? "草稿需要处理"
                  : draftState.dirty
                    ? "草稿待保存"
                    : "项目已保存"}
            </span>
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
        <span className="status-workbench" ref={onStatusHost} />
        <LakeActivity
          onPreparations={() => openLakes(undefined, undefined, "preparations")}
          status={lakeStatus.data}
          collections={collectionStatus.data}
          disconnected={lakeStatus.isError}
          onOpen={(id, family) => openLakes(id, undefined, "jobs", family)}
          onProjectTasks={project ? () => setTasksVisible(true) : undefined}
        />
        {project && (
          <TaskActivity
            jobs={jobs.data?.items ?? []}
            disconnected={jobs.isError || health.isError}
            expanded={tasksVisible}
            onOpen={() => setTasksVisible((v) => !v)}
          />
        )}
      </footer>
      {settingsPage && (
        <SettingsDialog
          client={client}
          initialPage={settingsPage}
          project={project ?? null}
          sources={sources.data?.items ?? []}
          activeResultId={activeResultId || null}
          onClose={() => setSettingsPage(null)}
        />
      )}
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
          }}
        />
      )}
      {paletteOpen && (
        <CommandPalette
          commands={paletteCommands()}
          onClose={() => setPaletteOpen(false)}
        />
      )}
      {exportScope !== null && project && (
        <ExportDialog
          client={client}
          projectId={project.id}
          options={inputs}
          defaultScope={exportScope}
          onClose={() => setExportScope(null)}
          onSubmitted={(job, count) => {
            setExportScope(null);
            void queryClient.invalidateQueries({
              queryKey: ["project", currentId, "jobs"],
            });
            notify({
              tone: "info",
              title: "已开始导出原图",
              detail:
                (count === null
                  ? ""
                  : count.toLocaleString("zh-CN") + " 项，") +
                "进度见项目任务。",
              action: {
                label: "查看任务",
                run: () => {
                  setTaskFocus(job.id);
                  setTasksVisible(true);
                },
              },
            });
          }}
        />
      )}
      {memberEdit && project && memberEdit.projectId === project.id && (
        <CollectionMembersDialog
          key={project.id}
          client={client}
          projectId={project.id}
          intent={memberEdit}
          options={inputs}
          onClose={() => setMemberEdit(null)}
          onApplied={(result) => {
            if (
              result.changed &&
              view.scope.kind === "collection" &&
              view.scope.id === result.collection.id
            )
              workspace.controller?.set((v) => ({
                ...v,
                position: null,
                ...(memberEdit.operation === "remove"
                  ? { focusKey: null }
                  : {}),
              }));
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
  onManage,
  onUpdates,
}: {
  source: Source;
  active: boolean;
  onClick: () => void;
  onRelink: () => void;
  onManage: (mode?: ManagementMode) => void;
  onUpdates: () => void;
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
      <MoreMenu
        label={source.name}
        contextMenu
        items={[
          ...(source.kind !== "demo"
            ? [{ label: "管理此数据湖更新", action: onUpdates }]
            : []),
          { label: "管理与引用关系", action: () => onManage() },
          { label: "重命名与备注…", action: () => onManage("rename") },
          ...(sourceSupports(source, "relink")
            ? [{ label: "重新关联本机位置…", action: onRelink }]
            : []),
          {
            label: "取消与项目关联…",
            danger: true,
            action: () => onManage("remove"),
          },
        ]}
      />
    </div>
  );
}
