import { useMemo, useSyncExternalStore } from "react";
import type { StudioClient, DraftSnapshot } from "@studio/client";
import type { AssetKey } from "@studio/contracts";
import type { BrowseScope, BrowserPosition } from "@studio/ui";
export type WorkspaceState = {
  moduleId: string;
  panels: string[];
  scope: BrowseScope;
  focusKey: AssetKey | null;
  view: "grid" | "image";
  position: BrowserPosition | null;
};
const initial: WorkspaceState = {
  moduleId: "core.browser",
  panels: [],
  scope: { kind: "all" },
  focusKey: null,
  view: "grid",
  position: null,
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
  return value as WorkspaceState;
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
};
const initialLayout: LayoutState = {
  projectsVisible: true,
  propertiesVisible: true,
  tasksVisible: false,
  thumbnailSize: 176,
};
function decodeLayout(value: unknown): LayoutState | null {
  return record(value) &&
    typeof value.projectsVisible === "boolean" &&
    typeof value.propertiesVisible === "boolean" &&
    typeof value.tasksVisible === "boolean" &&
    typeof value.thumbnailSize === "number" &&
    value.thumbnailSize >= 128 &&
    value.thumbnailSize <= 256
    ? (value as LayoutState)
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
