import { useCallback, useEffect, useRef, useState } from "react";
import type { KeyboardEvent, MouseEvent, ReactNode } from "react";
import { Maximize, ZoomIn, ZoomOut } from "lucide-react";
import "./media.css";

// Large enough for 1:1 on big originals; previews swap to the original when zoomed.
export const maxZoom = 16;
const fit = { zoom: 1, x: 0, y: 0 };
export type ImageZoom = ReturnType<typeof useImageZoom>;
/** Zoom relative to the fitted image; returns to fit whenever `resetKey` changes. */
export function useImageZoom(resetKey: string) {
  const [state, setState] = useState({ key: resetKey, ...fit });
  const current = state.key === resetKey ? state : { key: resetKey, ...fit };
  const scale = useCallback(
    (factor: number) =>
      setState((old) => ({
        key: resetKey,
        zoom: Math.max(
          1,
          Math.min(maxZoom, (old.key === resetKey ? old.zoom : 1) * factor),
        ),
        x: 0,
        y: 0,
      })),
    [resetKey],
  );
  const reset = useCallback(
    () => setState({ key: resetKey, ...fit }),
    [resetKey],
  );
  // Unlike wheel steps, an explicit zoom (1:1) may show small images below fit.
  const set = useCallback(
    (zoom: number) =>
      setState({
        key: resetKey,
        zoom: Math.max(0.05, Math.min(maxZoom, zoom)),
        x: 0,
        y: 0,
      }),
    [resetKey],
  );
  return {
    zoom: current.zoom,
    x: current.x,
    y: current.y,
    scale,
    set,
    reset,
    toggle: () =>
      setState({ key: resetKey, zoom: current.zoom > 1 ? 1 : 2, x: 0, y: 0 }),
    pan: (x: number, y: number) =>
      setState((old) => (old.key === resetKey ? { ...old, x, y } : old)),
    /** Handles + / - / 0; returns whether the key was consumed. */
    onKey(event: KeyboardEvent) {
      if (event.altKey || event.ctrlKey || event.metaKey) return false;
      if (event.key === "+" || event.key === "=") scale(1.25);
      else if (event.key === "-") scale(0.8);
      else if (event.key === "0") reset();
      else return false;
      event.preventDefault();
      return true;
    },
  };
}
export function ZoomableImage({
  zoom,
  children,
  overlay,
  onContextMenu,
}: {
  zoom: ImageZoom;
  children: ReactNode;
  overlay?: ReactNode;
  onContextMenu?: (event: MouseEvent<HTMLDivElement>) => void;
}) {
  const viewport = useRef<HTMLDivElement>(null);
  const drag = useRef<{
    x: number;
    y: number;
    originX: number;
    originY: number;
  } | null>(null);
  const { scale } = zoom;
  useEffect(() => {
    // Non-passive listener keeps zoom local to the viewport, including Ctrl+wheel.
    const element = viewport.current;
    const wheel = (event: WheelEvent) => {
      event.preventDefault();
      scale(event.deltaY < 0 ? 1.15 : 1 / 1.15);
    };
    element?.addEventListener("wheel", wheel, { passive: false });
    return () => element?.removeEventListener("wheel", wheel);
  }, [scale]);
  const release = () => {
    drag.current = null;
  };
  return (
    <div
      ref={viewport}
      className="zoom-viewport"
      data-zoom={zoom.zoom}
      data-pan-x={zoom.x}
      data-pan-y={zoom.y}
      style={{ cursor: zoom.zoom > 1 ? "grab" : "default" }}
      onDoubleClick={zoom.toggle}
      onContextMenu={onContextMenu}
      onPointerDown={(event) => {
        if (event.button !== 0 || zoom.zoom <= 1) return;
        event.preventDefault();
        event.currentTarget.setPointerCapture(event.pointerId);
        drag.current = {
          x: event.clientX,
          y: event.clientY,
          originX: zoom.x,
          originY: zoom.y,
        };
      }}
      onPointerMove={(event) => {
        const start = drag.current;
        if (!start) return;
        const xLimit = (event.currentTarget.clientWidth * (zoom.zoom - 1)) / 2;
        const yLimit = (event.currentTarget.clientHeight * (zoom.zoom - 1)) / 2;
        zoom.pan(
          Math.max(
            -xLimit,
            Math.min(xLimit, start.originX + event.clientX - start.x),
          ),
          Math.max(
            -yLimit,
            Math.min(yLimit, start.originY + event.clientY - start.y),
          ),
        );
      }}
      onPointerUp={release}
      onPointerCancel={release}
      onLostPointerCapture={release}
    >
      <div
        className="zoom-transform"
        style={{
          transform: `translate(${zoom.x}px, ${zoom.y}px) scale(${zoom.zoom})`,
        }}
      >
        {children}
      </div>
      {overlay}
    </div>
  );
}
export function ZoomControls({ zoom }: { zoom: ImageZoom }) {
  return (
    <>
      <button
        type="button"
        aria-label="缩小图片"
        disabled={zoom.zoom <= 1}
        onClick={() => zoom.scale(0.8)}
      >
        <ZoomOut size={15} />
      </button>
      <output aria-label="图片显示倍率" title="相对适应窗口的倍率">
        {Math.round(zoom.zoom * 100)}%
      </output>
      <button
        type="button"
        aria-label="放大图片"
        disabled={zoom.zoom >= maxZoom}
        onClick={() => zoom.scale(1.25)}
      >
        <ZoomIn size={15} />
      </button>
      <button type="button" aria-label="图片适应窗口" onClick={zoom.reset}>
        <Maximize size={15} />
        适应
      </button>
    </>
  );
}
