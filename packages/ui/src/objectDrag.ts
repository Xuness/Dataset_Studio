import { useEffect, useId, useRef, useSyncExternalStore } from "react";
import type { PointerEvent as ReactPointerEvent } from "react";

/**
 * Pointer-driven object drag between lists and drop zones. Tauri's WebView on
 * Windows intercepts native HTML5 drag events, so this mirrors the dock-tab
 * drag in Workbench: a 6px threshold, a floating ghost and Escape to cancel.
 */
export type DragObject = { kind: string; id: string; label: string };
type Zone = {
  accepts: (object: DragObject) => boolean;
  drop: (object: DragObject) => void;
};
type DragSnapshot = { object: DragObject; over: string | null } | null;

const zones = new Map<string, Zone>();
const listeners = new Set<() => void>();
let current: DragSnapshot = null;
function publish(next: DragSnapshot) {
  current = next;
  for (const listener of listeners) listener();
}
function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
function zoneAt(object: DragObject, x: number, y: number) {
  const id = document
    .elementFromPoint(x, y)
    ?.closest<HTMLElement>("[data-drop-zone]")?.dataset.dropZone;
  return id && zones.get(id)?.accepts(object) ? id : null;
}

/** Start dragging `object` from a pointerdown; a plain click stays a click. */
export function startObjectDrag(event: ReactPointerEvent, object: DragObject) {
  if (event.button !== 0 || current) return;
  const origin = { x: event.clientX, y: event.clientY };
  let ghost: HTMLDivElement | null = null;
  const move = (e: PointerEvent) => {
    if (!ghost && Math.hypot(e.clientX - origin.x, e.clientY - origin.y) < 6)
      return;
    if (!ghost) {
      ghost = document.createElement("div");
      ghost.className = "wb-drag-ghost";
      ghost.textContent = object.label;
      document.body.append(ghost);
      document.body.dataset.objectDragging = "true";
    }
    ghost.style.left = e.clientX + 12 + "px";
    ghost.style.top = e.clientY + 10 + "px";
    const over = zoneAt(object, e.clientX, e.clientY);
    if (current?.over !== over || current.object !== object)
      publish({ object, over });
  };
  const stop = (commit: boolean, e?: PointerEvent) => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", cancel);
    window.removeEventListener("keydown", key, true);
    if (!ghost) return;
    ghost.remove();
    delete document.body.dataset.objectDragging;
    const over = commit && e ? zoneAt(object, e.clientX, e.clientY) : null;
    publish(null);
    // The release would otherwise click whatever row the drag started on.
    const swallow = (click: MouseEvent) => {
      click.stopPropagation();
      click.preventDefault();
    };
    window.addEventListener("click", swallow, { capture: true, once: true });
    setTimeout(() => window.removeEventListener("click", swallow, true), 0);
    if (over) zones.get(over)?.drop(object);
  };
  const up = (e: PointerEvent) => stop(true, e);
  const cancel = () => stop(false);
  const key = (e: KeyboardEvent) => {
    if (e.key !== "Escape") return;
    e.preventDefault();
    e.stopPropagation();
    stop(false);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", cancel);
  window.addEventListener("keydown", key, true);
}

/**
 * Register a drop zone. Spread `props` on the target element; `active` is true
 * while an acceptable object is being dragged and `over` while it is above.
 */
export function useDropZone(
  accepts: (object: DragObject) => boolean,
  drop: (object: DragObject) => void,
  disabled = false,
) {
  const id = useId();
  const latest = useRef<Zone>({ accepts, drop });
  latest.current = { accepts, drop };
  useEffect(() => {
    if (disabled) return;
    zones.set(id, {
      accepts: (object) => latest.current.accepts(object),
      drop: (object) => latest.current.drop(object),
    });
    return () => {
      zones.delete(id);
    };
  }, [id, disabled]);
  const state = useSyncExternalStore(subscribe, () => current);
  const active = !disabled && !!state && accepts(state.object);
  return {
    props: { "data-drop-zone": id },
    active,
    over: active && state?.over === id,
    dragging: state?.object ?? null,
  };
}

/** The object currently being dragged, if any. */
export function useObjectDragging() {
  return useSyncExternalStore(subscribe, () => current)?.object ?? null;
}
