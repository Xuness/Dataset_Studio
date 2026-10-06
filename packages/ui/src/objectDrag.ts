import { useEffect, useId, useRef, useSyncExternalStore } from "react";
import type { PointerEvent as ReactPointerEvent } from "react";

/**
 * Pointer-driven object drag between lists and drop zones. Tauri's WebView on
 * Windows intercepts native HTML5 drag events, so this mirrors the dock-tab
 * drag in Workbench: a 6px threshold, a floating ghost and Escape to cancel.
 */
export type DragObject = { kind: string; id: string; label: string };
/** Optional sub-target inside a zone, such as "before" or "after" a list row. */
export type DropLocate = (rect: DOMRect, x: number, y: number) => string | null;
type Zone = {
  accepts: (object: DragObject) => boolean;
  drop: (object: DragObject, place: string | null) => void;
  locate: DropLocate | undefined;
};
type Target = { over: string | null; place: string | null };
type DragSnapshot = ({ object: DragObject } & Target) | null;

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
function targetAt(object: DragObject, x: number, y: number): Target {
  const element = document
    .elementFromPoint(x, y)
    ?.closest<HTMLElement>("[data-drop-zone]");
  const id = element?.dataset.dropZone;
  const zone = id ? zones.get(id) : undefined;
  if (!element || !id || !zone?.accepts(object))
    return { over: null, place: null };
  return {
    over: id,
    place: zone.locate?.(element.getBoundingClientRect(), x, y) ?? null,
  };
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
    const target = targetAt(object, e.clientX, e.clientY);
    if (
      current?.object !== object ||
      current.over !== target.over ||
      current.place !== target.place
    )
      publish({ object, ...target });
  };
  const stop = (commit: boolean, e?: PointerEvent) => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", cancel);
    window.removeEventListener("keydown", key, true);
    if (!ghost) return;
    ghost.remove();
    delete document.body.dataset.objectDragging;
    const target =
      commit && e
        ? targetAt(object, e.clientX, e.clientY)
        : { over: null, place: null };
    publish(null);
    // The release would otherwise click whatever row the drag started on.
    const swallow = (click: MouseEvent) => {
      click.stopPropagation();
      click.preventDefault();
    };
    window.addEventListener("click", swallow, { capture: true, once: true });
    setTimeout(() => window.removeEventListener("click", swallow, true), 0);
    if (target.over) zones.get(target.over)?.drop(object, target.place);
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
 * `locate` splits the zone, e.g. into the halves of a row; `place` reports the
 * part under the pointer and is passed to `drop`.
 */
export function useDropZone(
  accepts: (object: DragObject) => boolean,
  drop: (object: DragObject, place: string | null) => void,
  disabled = false,
  locate?: DropLocate,
) {
  const id = useId();
  const latest = useRef({ accepts, drop, locate });
  latest.current = { accepts, drop, locate };
  useEffect(() => {
    if (disabled) return;
    zones.set(id, {
      accepts: (object) => latest.current.accepts(object),
      drop: (object, place) => latest.current.drop(object, place),
      locate: (rect, x, y) => latest.current.locate?.(rect, x, y) ?? null,
    });
    return () => {
      zones.delete(id);
    };
  }, [id, disabled]);
  const state = useSyncExternalStore(subscribe, () => current);
  const active = !disabled && !!state && accepts(state.object);
  const over = active && state?.over === id;
  return {
    props: { "data-drop-zone": id },
    active,
    over,
    place: over ? (state?.place ?? null) : null,
    dragging: state?.object ?? null,
  };
}

/** The object currently being dragged, if any. */
export function useObjectDragging() {
  return useSyncExternalStore(subscribe, () => current)?.object ?? null;
}
