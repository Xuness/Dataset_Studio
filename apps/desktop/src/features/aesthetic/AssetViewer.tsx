import { useEffect, useRef, useState } from "react";
import {
  ChevronLeft,
  ChevronRight,
  LayoutGrid,
  Maximize,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import type { ModuleContext } from "@studio/ui";
import type { Asset } from "@studio/contracts";
import { AssetImage } from "../browser/AssetImage.js";
type Navigation = {
  disabled: boolean;
  previous: boolean;
  next: boolean;
  onNavigate: (delta: number) => void;
};
export function AssetViewer({
  context,
  asset,
  title,
  ariaLabel,
  backLabel = "返回图片网格",
  onGrid,
  ...navigation
}: Navigation & {
  context: ModuleContext;
  asset: Asset;
  title: string;
  ariaLabel?: string;
  backLabel?: string;
  onGrid: () => void;
}) {
  const viewport = useRef<HTMLDivElement>(null);
  const region = useRef<HTMLDivElement>(null);
  const [transform, setTransform] = useState({ zoom: 1, x: 0, y: 0 });
  const drag = useRef<{
    x: number;
    y: number;
    originX: number;
    originY: number;
  } | null>(null);
  function zoom(factor: number) {
    setTransform((old) => {
      const value = Math.max(1, Math.min(4, old.zoom * factor));
      return { zoom: value, x: 0, y: 0 };
    });
  }
  useEffect(() => {
    // Non-passive listener keeps zoom local to the viewport, including Ctrl+wheel.
    const element = viewport.current;
    const wheel = (event: WheelEvent) => {
      event.preventDefault();
      zoom(event.deltaY < 0 ? 1.15 : 1 / 1.15);
    };
    element?.addEventListener("wheel", wheel, { passive: false });
    if (!document.activeElement?.closest(".ranking-review-panel"))
      region.current?.focus({ preventScroll: true });
    return () => element?.removeEventListener("wheel", wheel);
  }, []);
  return (
    <div
      ref={region}
      className="ranking-full-image"
      tabIndex={0}
      role="region"
      aria-label={ariaLabel ?? title}
      onKeyDown={(event) => {
        if (
          event.altKey ||
          event.ctrlKey ||
          event.metaKey ||
          navigation.disabled
        )
          return;
        if (event.key === "Escape") {
          event.preventDefault();
          onGrid();
        } else if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
          event.preventDefault();
          navigation.onNavigate(event.key === "ArrowLeft" ? -1 : 1);
        } else if (["+", "="].includes(event.key)) {
          event.preventDefault();
          zoom(1.25);
        } else if (event.key === "-") {
          event.preventDefault();
          zoom(0.8);
        } else if (event.key === "0") {
          event.preventDefault();
          setTransform({ zoom: 1, x: 0, y: 0 });
        }
      }}
    >
      <div className="ranking-image-toolbar">
        <button type="button" aria-label={backLabel} onClick={onGrid}>
          <LayoutGrid size={15} />
          网格
        </button>
        <span className="ranking-current-image">{title}</span>
        <span className="grow" />
        <button
          type="button"
          aria-label="上一张图片"
          disabled={navigation.disabled || !navigation.previous}
          onClick={() => navigation.onNavigate(-1)}
        >
          <ChevronLeft size={16} />
        </button>
        <button
          type="button"
          aria-label="下一张图片"
          disabled={navigation.disabled || !navigation.next}
          onClick={() => navigation.onNavigate(1)}
        >
          <ChevronRight size={16} />
        </button>
        <span className="wb-tool-separator" />
        <button
          type="button"
          aria-label="缩小图片"
          disabled={transform.zoom <= 1}
          onClick={() => zoom(0.8)}
        >
          <ZoomOut size={15} />
        </button>
        <output aria-label="图片显示倍率" title="相对适应窗口的倍率">
          {Math.round(transform.zoom * 100)}%
        </output>
        <button
          type="button"
          aria-label="放大图片"
          disabled={transform.zoom >= 4}
          onClick={() => zoom(1.25)}
        >
          <ZoomIn size={15} />
        </button>
        <button
          type="button"
          aria-label="图片适应窗口"
          onClick={() => setTransform({ zoom: 1, x: 0, y: 0 })}
        >
          <Maximize size={15} />
          适应
        </button>
      </div>
      <div
        ref={viewport}
        className="ranking-image-viewport"
        data-zoom={transform.zoom}
        data-pan-x={transform.x}
        data-pan-y={transform.y}
        style={{ cursor: transform.zoom > 1 ? "grab" : "default" }}
        onDoubleClick={() =>
          setTransform({ zoom: transform.zoom > 1 ? 1 : 2, x: 0, y: 0 })
        }
        onPointerDown={(event) => {
          if (event.button !== 0 || transform.zoom <= 1) return;
          event.preventDefault();
          event.currentTarget.setPointerCapture(event.pointerId);
          drag.current = {
            x: event.clientX,
            y: event.clientY,
            originX: transform.x,
            originY: transform.y,
          };
        }}
        onPointerMove={(event) => {
          const start = drag.current;
          if (!start) return;
          const xLimit =
            (event.currentTarget.clientWidth * (transform.zoom - 1)) / 2;
          const yLimit =
            (event.currentTarget.clientHeight * (transform.zoom - 1)) / 2;
          setTransform((old) => ({
            ...old,
            x: Math.max(
              -xLimit,
              Math.min(xLimit, start.originX + event.clientX - start.x),
            ),
            y: Math.max(
              -yLimit,
              Math.min(yLimit, start.originY + event.clientY - start.y),
            ),
          }));
        }}
        onPointerUp={() => {
          drag.current = null;
        }}
        onPointerCancel={() => {
          drag.current = null;
        }}
        onLostPointerCapture={() => {
          drag.current = null;
        }}
      >
        <div
          className="ranking-image-transform"
          style={{
            transform: `translate(${transform.x}px, ${transform.y}px) scale(${transform.zoom})`,
          }}
        >
          <AssetImage
            client={context.client}
            projectId={context.projectId}
            asset={asset}
            edge={2048}
          />
        </div>
      </div>
      <div className="ranking-viewer-hint">
        ← → 切换图片 · 滚轮缩放 · 放大后拖动平移 · Esc 返回
      </div>
    </div>
  );
}
