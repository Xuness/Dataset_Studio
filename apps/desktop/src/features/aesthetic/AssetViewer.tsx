import { useEffect, useRef } from "react";
import { ChevronLeft, ChevronRight, LayoutGrid } from "lucide-react";
import type { ModuleContext } from "@studio/ui";
import type { Asset } from "@studio/contracts";
import {
  ImageStage,
  ImageTools,
  useImageViewer,
  viewerHint,
} from "../browser/ImageStage.js";
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
  const viewer = useImageViewer(context.client, context.projectId, asset);
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
        } else viewer.onKey(event);
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
        <ImageTools viewer={viewer} />
      </div>
      <ImageStage
        viewer={viewer}
        client={context.client}
        projectId={context.projectId}
        edge={1600}
      />
      <div className="ranking-viewer-hint">{viewerHint}· Esc 返回</div>
    </div>
  );
}
