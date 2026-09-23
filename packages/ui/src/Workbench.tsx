import {
  createContext,
  useContext,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useSyncExternalStore,
} from "react";
import { createPortal } from "react-dom";
import type { CSSProperties, ReactNode } from "react";
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
};
export type WorkbenchPanel = {
  id: string;
  title: string;
  content?: ReactNode;
  portal?: boolean;
  defaultPosition?: "left" | "right" | "bottom";
};
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
  return {
    panels,
    active,
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
  const groups = (position: DockPosition) =>
    panels.filter((p) => layout.panels[p.id] === position);
  function move(id: string, position: DockPosition) {
    onLayout({
      ...layout,
      panels: { ...layout.panels, [id]: position },
      active: { ...layout.active, [position]: id },
    });
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
        <header className="wb-panel-header">
          <div
            className="wb-panel-tabs"
            role="tablist"
            aria-label={title + " " + position + " 面板"}
          >
            {entries.map((p) => (
              <button
                key={p.id}
                type="button"
                role="tab"
                id={uid + p.id + "-tab"}
                aria-controls={uid + p.id}
                aria-selected={p.id === current.id}
                disabled={disabled}
                onClick={() =>
                  onLayout({
                    ...layout,
                    active: { ...layout.active, [position]: p.id },
                  })
                }
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
                  onLayout({
                    ...layout,
                    active: { ...layout.active, [position]: next.id },
                  });
                  document.getElementById(uid + next.id + "-tab")?.focus();
                }}
              >
                {p.title}
              </button>
            ))}
          </div>
          <MoreMenu
            label={current.title + "面板"}
            disabled={disabled}
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
            ]}
          />
          <button
            type="button"
            className="icon-button"
            aria-label={"隐藏" + current.title + "面板"}
            disabled={disabled}
            onClick={() => move(current.id, "hidden")}
          >
            <X size={13} />
          </button>
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
          className="wb-body"
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
        </div>
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
        <footer className="wb-status">
          <span className="wb-status-content">{status}</span>
          {panels
            .filter(
              (p) => !layout.panels[p.id] || layout.panels[p.id] === "hidden",
            )
            .map((p) => (
              <button
                type="button"
                key={p.id}
                disabled={disabled}
                onClick={() =>
                  move(
                    p.id,
                    p.defaultPosition ??
                      (p.id.includes("inspector") ? "right" : "left"),
                  )
                }
              >
                <PanelLeft size={13} />
                显示{p.title}
              </button>
            ))}
        </footer>
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
