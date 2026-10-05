import { useLayoutEffect, useRef, useState } from "react";

const storageKey = (id: string) => "studio.property-split." + id;
function stored(id: string, fallback: number) {
  try {
    const value = Number(localStorage.getItem(storageKey(id)));
    return Number.isFinite(value) && value > 0 ? value : fallback;
  } catch {
    return fallback;
  }
}

/**
 * The draggable divider between property names and values. It sets
 * `--property-split` on its positioned parent; rows read that variable for
 * their name column. The width is a per-viewer convenience kept locally.
 */
export function PropertySplitter({
  id,
  initial = 200,
  min = 110,
}: {
  id: string;
  initial?: number;
  min?: number;
}) {
  const handle = useRef<HTMLDivElement>(null);
  const [split, setSplit] = useState(() => stored(id, initial));
  useLayoutEffect(() => {
    handle.current?.parentElement?.style.setProperty(
      "--property-split",
      split + "px",
    );
  }, [split]);
  function apply(value: number, save: boolean) {
    const parent = handle.current?.parentElement;
    const max = Math.max(min, (parent?.clientWidth ?? 600) * 0.6);
    const next = Math.round(Math.max(min, Math.min(max, value)));
    setSplit(next);
    if (save)
      try {
        localStorage.setItem(storageKey(id), String(next));
      } catch {
        /* Storage is optional; the split still applies for this view. */
      }
  }
  return (
    <div
      ref={handle}
      className="property-splitter"
      role="separator"
      aria-orientation="vertical"
      aria-label="属性名称列宽度"
      aria-valuenow={split}
      tabIndex={0}
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        event.preventDefault();
        const parent = event.currentTarget.parentElement;
        if (!parent) return;
        const left = parent.getBoundingClientRect().left;
        const target = event.currentTarget;
        target.setPointerCapture(event.pointerId);
        const move = (e: PointerEvent) => apply(e.clientX - left, false);
        const up = (e: PointerEvent) => {
          apply(e.clientX - left, true);
          target.removeEventListener("pointermove", move);
          target.removeEventListener("pointerup", up);
          target.removeEventListener("pointercancel", up);
        };
        target.addEventListener("pointermove", move);
        target.addEventListener("pointerup", up);
        target.addEventListener("pointercancel", up);
      }}
      onDoubleClick={() => apply(initial, true)}
      onKeyDown={(event) => {
        if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
          event.preventDefault();
          apply(split + (event.key === "ArrowLeft" ? -10 : 10), true);
        } else if (event.key === "Home") {
          event.preventDefault();
          apply(initial, true);
        }
      }}
    />
  );
}
