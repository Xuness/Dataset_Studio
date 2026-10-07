import { useEffect, useRef } from "react";
import { ChevronLeft, ChevronRight, LayoutGrid } from "lucide-react";
import type { ModuleContext } from "@studio/ui";
import type { Asset } from "@studio/contracts";
import { AssetImage } from "../browser/AssetImage.js";
import {
  useImageZoom,
  ZoomableImage,
  ZoomControls,
} from "../browser/ZoomableImage.js";
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
  const region = useRef<HTMLDivElement>(null);
  const zoom = useImageZoom(`${asset.key.source_id}:${asset.key.asset_id}`);
  useEffect(() => {
    if (!document.activeElement?.closest(".ranking-review-panel"))
      region.current?.focus({ preventScroll: true });
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
        } else zoom.onKey(event);
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
        <ZoomControls zoom={zoom} />
      </div>
      <ZoomableImage zoom={zoom}>
        <AssetImage
          client={context.client}
          projectId={context.projectId}
          asset={asset}
          edge={2048}
        />
      </ZoomableImage>
      <div className="ranking-viewer-hint">
        ← → 切换图片 · 滚轮缩放 · 双击放大 · 放大后拖动平移 · + - 0 缩放 · Esc
        返回
      </div>
    </div>
  );
}
