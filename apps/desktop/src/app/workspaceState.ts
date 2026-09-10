import { useMemo, useSyncExternalStore } from "react";
import type { StudioClient, DraftSnapshot, ObjectTarget } from "@studio/client";
import type { AssetKey, QuerySpec } from "@studio/contracts";
import type { BrowseScope, BrowserPosition } from "@studio/ui";
export type WorkspaceState = {
  order: QuerySpec["order"];
  moduleId: string;
  panels: string[];
  scope: BrowseScope;
  focusKey: AssetKey | null;
  view: "grid" | "image";
  position: BrowserPosition | null;
  inspectorTab: "properties" | "management";
  managementTarget: ObjectTarget | null;
};
const initial: WorkspaceState = {
  order: "post_id_desc",
  moduleId: "core.browser",
  panels: [],
  scope: { kind: "all" },
  focusKey: null,
  view: "grid",
  position: null,
  inspectorTab: "properties",
  managementTarget: null,
};
const fallback: DraftSnapshot<WorkspaceState> = {
  value: initial,
  status: "loading",
  error: "",
  revision: 0,
  dirty: false,
  raw: null,
};
const emptySubscribe = () => () => {};
const emptySnapshot = () => fallback;
function record(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}
function decode(value: unknown): WorkspaceState | null {
  if (
    !record(value) ||
    typeof value.moduleId !== "string" ||
    !Array.isArray(value.panels) ||
    value.panels.length > 8 ||
    value.panels.some((id) => typeof id !== "string") ||
    !record(value.scope) ||
    !["all", "selection", "source", "collection", "result"].includes(
      String(value.scope.kind),
    ) ||
    !["grid", "image"].includes(String(value.view))
  )
    return null;
  if (
    ["source", "collection", "result"].includes(String(value.scope.kind)) &&
    (typeof value.scope.id !== "string" || typeof value.scope.name !== "string")
  )
    return null;
  if (
    value.focusKey !== null &&
    (!record(value.focusKey) ||
      typeof value.focusKey.source_id !== "string" ||
      typeof value.focusKey.asset_id !== "string")
  )
    return null;
  if (
    value.position !== null &&
    (!record(value.position) ||
      typeof value.position.scopeKey !== "string" ||
      typeof value.position.pageNumber !== "number" ||
      ![12, 48, 96].includes(Number(value.position.pageSize)) ||
      (value.position.cursor !== null &&
        typeof value.position.cursor !== "string"))
  )
    return null;
  if (
    value.order !== undefined &&
    ![
      "post_id_asc",
      "post_id_desc",
      "asset_key_asc",
      "asset_key_desc",
    ].includes(String(value.order))
  )
    return null;
  return {
    ...value,
    moduleId:
      value.moduleId === "core.resources" ? "core.browser" : value.moduleId,
    order: value.order ?? "post_id_desc",
    inspectorTab:
      value.inspectorTab === "management" ? "management" : "properties",
    managementTarget:
      record(value.managementTarget) &&
      typeof value.managementTarget.id === "string" &&
      [
        "project",
        "source",
        "workset",
        "artifact",
        "query",
        "job",
        "query_result",
        "selection",
        "selection_history",
      ].includes(String(value.managementTarget.kind))
        ? value.managementTarget
        : null,
  } as WorkspaceState;
}
export function useWorkspaceState(client: StudioClient, projectId: string) {
  const controller = useMemo(
    () =>
      projectId
        ? client.edits.open(
            {
              projectId,
              moduleId: "studio.session",
              instanceId: "default",
              schemaVersion: 1,
            },
            initial,
            decode,
          )
        : null,
    [client, projectId],
  );
  const state = useSyncExternalStore(
    controller?.subscribe ?? emptySubscribe,
    controller?.getSnapshot ?? emptySnapshot,
  );
  return {
    controller,
    ...state,
    editable:
      !!controller &&
      !["loading", "unsupported"].includes(state.status) &&
      !(state.status === "error" && !state.dirty),
  };
}
type LayoutState = {
  projectsVisible: boolean;
  propertiesVisible: boolean;
  tasksVisible: boolean;
  thumbnailSize: number;
  projectWidth: number;
  propertiesWidth: number;
  queryHeight: number;
};
const initialLayout: LayoutState = {
  projectsVisible: true,
  propertiesVisible: true,
  tasksVisible: false,
  thumbnailSize: 208,
  projectWidth: 224,
  propertiesWidth: 320,
  queryHeight: 350,
};
function decodeLayout(value: unknown): LayoutState | null {
  return record(value) &&
    typeof value.projectsVisible === "boolean" &&
    typeof value.propertiesVisible === "boolean" &&
    typeof value.tasksVisible === "boolean" &&
    typeof value.thumbnailSize === "number" &&
    value.thumbnailSize >= 128 &&
    value.thumbnailSize <= 320
    ? ({
        ...initialLayout,
        ...value,
        projectWidth:
          typeof value.projectWidth === "number"
            ? Math.max(180, Math.min(360, value.projectWidth))
            : initialLayout.projectWidth,
        propertiesWidth:
          typeof value.propertiesWidth === "number"
            ? Math.max(260, Math.min(600, value.propertiesWidth))
            : initialLayout.propertiesWidth,
        queryHeight:
          typeof value.queryHeight === "number"
            ? Math.max(190, Math.min(640, value.queryHeight))
            : initialLayout.queryHeight,
      } as LayoutState)
    : null;
}
export function useLayoutState(client: StudioClient) {
  const controller = useMemo(
    () => client.edits.preference("studio.layout", initialLayout, decodeLayout),
    [client],
  );
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
  );
  const editable =
    !["loading", "unsupported"].includes(state.status) &&
    !(state.status === "error" && !state.dirty);
  function update(patch: Partial<LayoutState>) {
    if (editable) controller.set((v) => ({ ...v, ...patch }));
  }
  return { ...state, controller, editable, update };
}
