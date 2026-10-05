import {
  createContext,
  useContext,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { createPortal } from "react-dom";
import type {
  CSSProperties,
  PointerEvent as ReactPointerEvent,
  ReactNode,
} from "react";
import type { StudioClient } from "@studio/client";
import { PanelLeft, RotateCcw, X } from "lucide-react";
import { MoreMenu } from "./MoreMenu.js";
import { ResizeGrip } from "./ResizeGrip.js";
import { DraftStatus } from "./drafts.js";
import "./workbench.css";

export type DockPosition = "left" | "right" | "bottom" | "hidden";
export type WorkbenchLayout = {
  panels: Record<string, DockPosition>;
  active: Partial<Record<DockPosition, string>>;
  leftWidth: number;
  rightWidth: number;
  bottomHeight: number;
  /** Tab order across all zones; panels missing here follow declaration. */
  order?: string[];
};
export type WorkbenchPanel = {
  id: string;
  title: string;
  icon?: ReactNode;
  content?: ReactNode;
  portal?: boolean;
  defaultPosition?: "left" | "right" | "bottom";
};
type DragState = {
  id: string;
  title: string;
  x: number;
  y: number;
  target: { position: DockPosition; before?: string } | null;
};
/** Where workbenches render their status line; null keeps their own footer. */
export const WorkbenchStatusTarget = createContext<HTMLElement | null>(null);
const PanelContext = createContext<{
  hosts: Map<string, HTMLDivElement>;
  open: (id: string) => void;
} | null>(null);

/** Keep feature state with its view while presenting controls in a dock panel. */
export function WorkbenchPanelPortal({
  id,
  children,
}: {
  id: string;
  children: ReactNode;
}) {
  const context = useContext(PanelContext);
  const host = context?.hosts.get(id);
  return host ? createPortal(children, host) : context ? null : children;
}
export function useWorkbenchPanels() {
  return useContext(PanelContext);
}
function PanelHost({ host }: { host: HTMLDivElement }) {
  const container = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const parent = container.current;
    parent?.appendChild(host);
    return () => {
      if (host.parentNode === parent) host.remove();
    };
  }, [host]);
  return <div className="wb-panel-slot" ref={container} />;
}
export const defaultWorkbenchLayout: WorkbenchLayout = {
  panels: {},
  active: {},
  leftWidth: 224,
  rightWidth: 300,
  bottomHeight: 260,
};
function decodeLayout(value: unknown): WorkbenchLayout | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Record<string, unknown>;
  if (!v.panels || typeof v.panels !== "object" || Array.isArray(v.panels))
    return null;
  const panels: Record<string, DockPosition> = {};
  for (const [id, position] of Object.entries(v.panels)) {
    if (
      id.length > 120 ||
      !["left", "right", "bottom", "hidden"].includes(String(position))
    )
      return null;
    panels[id] = position as DockPosition;
  }
  const active: WorkbenchLayout["active"] = {};
  if (v.active && typeof v.active === "object") {
    for (const [position, id] of Object.entries(v.active)) {
      if (
        ["left", "right", "bottom"].includes(position) &&
        typeof id === "string"
      )
        active[position as DockPosition] = id;
    }
  }
  const size = (key: string, fallback: number, min: number, max: number) =>
    typeof v[key] === "number" && Number.isFinite(v[key])
      ? Math.max(min, Math.min(max, v[key]))
      : fallback;
  const order =
    Array.isArray(v.order) &&
    v.order.length <= 200 &&
    v.order.every((id) => typeof id === "string" && id.length <= 120)
      ? { order: v.order as string[] }
      : {};
  return {
    panels,
    active,
    ...order,
    leftWidth: size("leftWidth", 224, 180, 480),
    rightWidth: size("rightWidth", 300, 220, 600),
    bottomHeight: size("bottomHeight", 260, 150, 600),
  };
}

/** Layout preferences contain presentation only. Closing panels never controls jobs. */
export function useWorkbenchLayout(
  client: StudioClient,
  profile: string,
  initial: WorkbenchLayout,
) {
  const controller = useMemo(
    () =>
      client.edits.preference(
        "studio.workbench." + profile,
        initial,
        decodeLayout,
      ),
    [client, profile, initial],
  );
  const snapshot = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
  );
  const editable =
    !["loading", "unsupported"].includes(snapshot.status) &&
    !(snapshot.status === "error" && !snapshot.dirty);
  function update(
    value: WorkbenchLayout | ((old: WorkbenchLayout) => WorkbenchLayout),
  ) {
    if (editable) controller.set(value);
  }
  return {
    ...snapshot,
    controller,
    editable,
    update,
    reset: () => update(initial),
  };
}

export function Workbench({
  title,
  panels,
  layout,
  onLayout,
  children,
  toolbar,
  status,
  disabled = false,
}: {
  title: string;
  panels: WorkbenchPanel[];
  layout: WorkbenchLayout;
  onLayout: (value: WorkbenchLayout) => void;
  children: ReactNode;
  toolbar?: ReactNode;
  status?: ReactNode;
  disabled?: boolean;
}) {
  const uid = useId();
  const body = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<DragState | null>(null);
  const hosts = useMemo(() => new Map<string, HTMLDivElement>(), []);
  for (const panel of panels) {
    if (panel.portal && !hosts.has(panel.id)) {
      const host = document.createElement("div");
      host.className = "wb-panel-portal";
      hosts.set(panel.id, host);
    }
  }
  useLayoutEffect(() => {
    for (const id of hosts.keys()) {
      if (!panels.some((panel) => panel.id === id && panel.portal))
        hosts.delete(id);
    }
  }, [hosts, panels]);
  const content = (panel: WorkbenchPanel) =>
    panel.portal ? <PanelHost host={hosts.get(panel.id)!} /> : panel.content;
  const rank = (id: string) => {
    const index = layout.order?.indexOf(id) ?? -1;
    return index < 0
      ? 1000 + panels.findIndex((panel) => panel.id === id)
      : index;
  };
  const groups = (position: DockPosition) =>
    panels
      .filter((p) => layout.panels[p.id] === position)
      .sort((a, b) => rank(a.id) - rank(b.id));
  function move(id: string, position: DockPosition) {
    onLayout({
      ...layout,
      panels: { ...layout.panels, [id]: position },
      active: { ...layout.active, [position]: id },
    });
  }
  function activate(position: DockPosition, id: string) {
    onLayout({ ...layout, active: { ...layout.active, [position]: id } });
  }
  /** Where a dragged tab would land for a pointer position, if anywhere. */
  function dropTarget(id: string, x: number, y: number): DragState["target"] {
    const root = body.current;
    if (!root) return null;
    const element = document.elementFromPoint(x, y);
    const tab = element?.closest<HTMLElement>("[data-wb-tab]");
    if (tab && root.contains(tab)) {
      const position = tab.dataset.position as DockPosition;
      const rect = tab.getBoundingClientRect();
      const ids = groups(position)
        .map((p) => p.id)
        .filter((other) => other !== id);
      const at = ids.indexOf(tab.dataset.wbTab!);
      const after = x > rect.left + rect.width / 2;
      const before = after ? ids[at + 1] : tab.dataset.wbTab;
      return before && before !== id ? { position, before } : { position };
    }
    const header = element?.closest<HTMLElement>("[data-wb-zone]");
    if (header && root.contains(header))
      return { position: header.dataset.wbZone as DockPosition };
    const rect = root.getBoundingClientRect();
    if (x < rect.left || x > rect.right || y < rect.top || y > rect.bottom)
      return null;
    if (x < rect.left + rect.width * 0.2) return { position: "left" };
    if (x > rect.right - rect.width * 0.2) return { position: "right" };
    if (y > rect.bottom - rect.height * 0.32) return { position: "bottom" };
    return null;
  }
  function drop(id: string, target: NonNullable<DragState["target"]>) {
    const order = [...panels]
      .sort((a, b) => rank(a.id) - rank(b.id))
      .map((p) => p.id)
      .filter((other) => other !== id);
    const index = target.before ? order.indexOf(target.before) : -1;
    order.splice(index < 0 ? order.length : index, 0, id);
    onLayout({
      ...layout,
      order,
      panels: { ...layout.panels, [id]: target.position },
      active: { ...layout.active, [target.position]: id },
    });
  }
  function startDrag(event: ReactPointerEvent, panel: WorkbenchPanel) {
    if (event.button !== 0 || disabled) return;
    const origin = { x: event.clientX, y: event.clientY };
    let started = false;
    let latest: DragState | null = null;
    const update = (next: DragState | null) => {
      latest = next;
      setDrag(next);
    };
    const move = (e: PointerEvent) => {
      if (
        !started &&
        Math.hypot(e.clientX - origin.x, e.clientY - origin.y) < 6
      )
        return;
      started = true;
      update({
        id: panel.id,
        title: panel.title,
        x: e.clientX,
        y: e.clientY,
        target: dropTarget(panel.id, e.clientX, e.clientY),
      });
    };
    const stop = (commit: boolean) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("keydown", key, true);
      const final = latest as DragState | null;
      update(null);
      if (commit && final?.target) drop(panel.id, final.target);
    };
    const up = () => stop(true);
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      stop(false);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("keydown", key, true);
  }
  function zone(position: "left" | "right" | "bottom") {
    const entries = groups(position);
    if (!entries.length) return null;
    const current =
      entries.find((p) => p.id === layout.active[position]) ?? entries[0]!;
    return (
      <section
        className={"wb-zone wb-" + position}
        aria-label={current.title + "面板"}
      >
        <header className="wb-panel-header" data-wb-zone={position}>
          <div
            className="wb-panel-tabs"
            role="tablist"
            aria-label={title + " " + position + " 面板"}
          >
            {entries.map((p) => (
              <div
                key={p.id}
                className="wb-panel-tab"
                data-wb-tab={p.id}
                data-position={position}
                data-active={p.id === current.id}
                data-dragging={drag?.id === p.id || undefined}
                data-drop-before={
                  drag?.target?.position === position &&
                  drag.target.before === p.id
                    ? true
                    : undefined
                }
                onPointerDown={(event) => startDrag(event, p)}
                onAuxClick={(event) => {
                  if (event.button === 1 && !disabled) move(p.id, "hidden");
                }}
              >
                <button
                  type="button"
                  role="tab"
                  id={uid + p.id + "-tab"}
                  aria-controls={uid + p.id}
                  aria-selected={p.id === current.id}
                  disabled={disabled}
                  onClick={() => activate(position, p.id)}
                  onContextMenu={() => {
                    if (p.id !== current.id) activate(position, p.id);
                  }}
                  onKeyDown={(event) => {
                    if (
                      !["ArrowLeft", "ArrowRight", "Home", "End"].includes(
                        event.key,
                      )
                    )
                      return;
                    event.preventDefault();
                    const index =
                      event.key === "Home"
                        ? 0
                        : event.key === "End"
                          ? entries.length - 1
                          : (entries.indexOf(p) +
                              (event.key === "ArrowLeft" ? -1 : 1) +
                              entries.length) %
                            entries.length;
                    const next = entries[index]!;
                    activate(position, next.id);
                    document.getElementById(uid + next.id + "-tab")?.focus();
                  }}
                >
                  {p.icon && <span className="wb-tab-icon">{p.icon}</span>}
                  {p.title}
                </button>
                <button
                  type="button"
                  className="wb-tab-close"
                  tabIndex={-1}
                  aria-label={"隐藏" + p.title + "面板"}
                  title={"隐藏" + p.title + "面板"}
                  disabled={disabled}
                  onPointerDown={(event) => event.stopPropagation()}
                  onClick={() => move(p.id, "hidden")}
                >
                  <X size={11} />
                </button>
              </div>
            ))}
          </div>
          <MoreMenu
            label={current.title + "面板"}
            disabled={disabled}
            contextMenu
            items={[
              {
                label: "停靠到左侧",
                action: () => move(current.id, "left"),
                disabled: position === "left",
              },
              {
                label: "停靠到右侧",
                action: () => move(current.id, "right"),
                disabled: position === "right",
              },
              {
                label: "停靠到底部",
                action: () => move(current.id, "bottom"),
                disabled: position === "bottom",
              },
              {
                label: "隐藏面板",
                action: () => move(current.id, "hidden"),
                separator: true,
              },
            ]}
          />
        </header>
        {entries.map((panel) => (
          <div
            key={panel.id}
            className="wb-panel-content"
            role="tabpanel"
            id={uid + panel.id}
            aria-labelledby={uid + panel.id + "-tab"}
            hidden={panel.id !== current.id}
          >
            {content(panel)}
          </div>
        ))}
        <ResizeGrip
          label={
            current.title + (position === "bottom" ? "面板高度" : "面板宽度")
          }
          orientation={position === "bottom" ? "horizontal" : "vertical"}
          reverse={position !== "left"}
          value={
            position === "bottom"
              ? layout.bottomHeight
              : position === "left"
                ? layout.leftWidth
                : layout.rightWidth
          }
          minimum={
            position === "bottom" ? 150 : position === "left" ? 180 : 220
          }
          maximum={position === "left" ? 480 : 600}
          onChange={(size) =>
            onLayout({
              ...layout,
              [position === "bottom"
                ? "bottomHeight"
                : position === "left"
                  ? "leftWidth"
                  : "rightWidth"]: size,
            })
          }
          onReset={() =>
            onLayout({
              ...layout,
              [position === "bottom"
                ? "bottomHeight"
                : position === "left"
                  ? "leftWidth"
                  : "rightWidth"]:
                position === "bottom" ? 260 : position === "left" ? 224 : 300,
            })
          }
        />
      </section>
    );
  }
  const left = groups("left").length > 0,
    right = groups("right").length > 0;
  const statusTarget = useContext(WorkbenchStatusTarget);
  const statusLine = (
    <>
      <span className="wb-status-content">{status}</span>
      {panels
        .filter((p) => !layout.panels[p.id] || layout.panels[p.id] === "hidden")
        .map((p) => (
          <button
            type="button"
            key={p.id}
            className="wb-restore-panel"
            disabled={disabled}
            title={"显示" + p.title + "面板"}
            onClick={() =>
              move(
                p.id,
                p.defaultPosition ??
                  (p.id.includes("inspector") ? "right" : "left"),
              )
            }
          >
            <PanelLeft size={13} />
            {p.title}
          </button>
        ))}
    </>
  );
  return (
    <PanelContext.Provider
      value={{
        hosts,
        open: (id) => {
          const position = layout.panels[id];
          move(id, position && position !== "hidden" ? position : "right");
        },
      }}
    >
      <section className="wb-shell" aria-label={title}>
        {toolbar && <div className="wb-toolbar">{toolbar}</div>}
        <div
          ref={body}
          className="wb-body"
          data-dragging={drag ? true : undefined}
          data-left={left}
          data-right={right}
          style={
            {
              "--wb-left": layout.leftWidth + "px",
              "--wb-right": layout.rightWidth + "px",
              "--wb-bottom": layout.bottomHeight + "px",
            } as CSSProperties
          }
        >
          {zone("left")}
          <div className="wb-center">
            <div className="wb-canvas">{children}</div>
            {zone("bottom")}
          </div>
          {zone("right")}
          {drag?.target && !drag.target.before && (
            <div
              className="wb-dock-preview"
              data-position={drag.target.position}
              aria-hidden="true"
            />
          )}
        </div>
        {drag && (
          <div
            className="wb-drag-ghost"
            style={{ left: drag.x + 12, top: drag.y + 10 }}
            aria-hidden="true"
          >
            {drag.title}
          </div>
        )}
        <div hidden>
          {panels
            .filter(
              (panel) =>
                panel.portal &&
                (!layout.panels[panel.id] ||
                  layout.panels[panel.id] === "hidden"),
            )
            .map((panel) => (
              <PanelHost key={panel.id} host={hosts.get(panel.id)!} />
            ))}
        </div>
        {statusTarget ? (
          createPortal(statusLine, statusTarget)
        ) : (
          <footer className="wb-status">{statusLine}</footer>
        )}
      </section>
    </PanelContext.Provider>
  );
}

export function WorkbenchPreferences({
  state,
}: {
  state: ReturnType<typeof useWorkbenchLayout>;
}) {
  return (
    <>
      <DraftStatus controller={state.controller} quiet />
      <button
        type="button"
        className="icon-button"
        title="恢复默认布局"
        aria-label="恢复默认布局"
        disabled={!state.editable}
        onClick={state.reset}
      >
        <RotateCcw size={14} />
      </button>
    </>
  );
}
