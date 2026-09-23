import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { CSSProperties, KeyboardEvent } from "react";
import {
  ChevronLeft,
  ChevronRight,
  LayoutGrid,
  Maximize,
  Shield,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import type { ModuleContext } from "@studio/ui";
import { AssetImage } from "../browser/AssetImage.js";
import { rankingAsset, rankingLabel } from "./analysisPresentation.js";
import type { RankingRow } from "./analysisPresentation.js";

type Navigation = {
  disabled: boolean;
  previous: boolean;
  next: boolean;
  onNavigate: (delta: number) => void;
};
export function RankingCanvas({
  context,
  items,
  selected,
  image,
  thumbnailSize,
  scrollTop,
  pageKey,
  loading,
  onSelect,
  onImage,
  onScroll,
  ...navigation
}: Navigation & {
  context: ModuleContext;
  items: RankingRow[];
  selected: RankingRow | undefined;
  image: boolean;
  thumbnailSize: number;
  scrollTop: number;
  pageKey: string;
  loading: boolean;
  onSelect: (ordinal: number) => void;
  onImage: (open: boolean) => void;
  onScroll: (value: number) => void;
}) {
  const grid = useRef<HTMLDivElement>(null);
  const scroll = useRef<HTMLDivElement>(null);
  const focusPending = useRef(false);
  const restoredPage = useRef("");
  const previousSize = useRef(thumbnailSize);
  const callback = useRef(onScroll);
  callback.current = onScroll;
  const scrollTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const pendingScroll = useRef<{
    top: number;
    save: (value: number) => void;
  } | null>(null);
  useEffect(
    () => () => {
      clearTimeout(scrollTimer.current);
      pendingScroll.current?.save(pendingScroll.current.top);
    },
    [],
  );
  useLayoutEffect(() => {
    if (previousSize.current !== thumbnailSize && !image && !loading) {
      previousSize.current = thumbnailSize;
      grid.current
        ?.querySelector('[aria-pressed="true"]')
        ?.scrollIntoView({ block: "nearest", inline: "nearest" });
    }
  }, [thumbnailSize, image, loading]);
  useLayoutEffect(() => {
    if (
      !loading &&
      !image &&
      restoredPage.current !== pageKey &&
      scroll.current
    ) {
      scroll.current.scrollTop = scrollTop;
      restoredPage.current = pageKey;
    }
  }, [pageKey, loading, image, scrollTop]);
  useLayoutEffect(() => {
    if (image || loading || !focusPending.current) return;
    const button = grid.current?.querySelector<HTMLButtonElement>(
      '[aria-pressed="true"]',
    );
    if (button) {
      button.focus({ preventScroll: true });
      button.scrollIntoView({ block: "nearest", inline: "nearest" });
      focusPending.current = false;
    }
  }, [selected?.ordinal, image, loading]);
  function openImage(open: boolean) {
    if (navigation.disabled) return;
    clearTimeout(scrollTimer.current);
    pendingScroll.current = null;
    if (open && scroll.current) onScroll(scroll.current.scrollTop);
    focusPending.current = !open;
    onImage(open);
  }
  function keyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (navigation.disabled || event.altKey || event.ctrlKey || event.metaKey)
      return;
    let delta: number;
    if (event.key === "Enter") {
      event.preventDefault();
      openImage(true);
      return;
    }
    const buttons = [
      ...(grid.current?.querySelectorAll<HTMLButtonElement>(
        ".ranking-image-tile",
      ) ?? []),
    ];
    const columns =
      buttons.filter((b) => b.offsetTop === buttons[0]?.offsetTop).length || 1;
    const index = items.findIndex((r) => r.ordinal === selected?.ordinal);
    if (event.key === "ArrowLeft") delta = -1;
    else if (event.key === "ArrowRight") delta = 1;
    else if (event.key === "ArrowUp") delta = -columns;
    else if (event.key === "ArrowDown") delta = columns;
    else if (event.key === "Home") delta = -index;
    else if (event.key === "End") delta = items.length - 1 - index;
    else return;
    event.preventDefault();
    focusPending.current = true;
    navigation.onNavigate(delta);
  }
  return (
    <div className="ranking-canvas" aria-busy={loading}>
      <div
        ref={scroll}
        className="ranking-image-scroll"
        hidden={image}
        onScroll={(event) => {
          if (image || loading || restoredPage.current !== pageKey) return;
          const top = event.currentTarget.scrollTop;
          const save = callback.current;
          clearTimeout(scrollTimer.current);
          pendingScroll.current = { top, save };
          scrollTimer.current = setTimeout(() => {
            pendingScroll.current = null;
            save(top);
          }, 180);
        }}
      >
        <div
          ref={grid}
          className="ranking-image-grid"
          aria-label="排名图片网格"
          style={
            { "--ranking-tile-size": `${thumbnailSize}px` } as CSSProperties
          }
          onKeyDown={keyDown}
        >
          {items.map((row) => (
            <button
              key={row.ordinal}
              type="button"
              className="ranking-image-tile"
              aria-label={`候选 ${row.ordinal + 1}，${rankingLabel(row)}`}
              aria-pressed={row.ordinal === selected?.ordinal}
              tabIndex={row.ordinal === selected?.ordinal ? 0 : -1}
              disabled={navigation.disabled}
              onClick={() => onSelect(row.ordinal)}
              onDoubleClick={() => openImage(true)}
            >
              <AssetImage
                client={context.client}
                projectId={context.projectId}
                asset={rankingAsset(row)}
                edge={480}
              />
              <span className="ranking-image-caption">
                <strong>{rankingLabel(row)}</strong>
                {row.protected && (
                  <Shield size={13} aria-label="快照内顶级提名" />
                )}
                <small>
                  候选 {row.ordinal + 1} · {row.exposures} 次曝光
                  {row.needs_review ? " · 待检查" : ""}
                </small>
              </span>
            </button>
          ))}
        </div>
      </div>
      {image && selected && (
        <ImageViewer
          key={`${selected.key.source_id}:${selected.key.asset_id}`}
          context={context}
          row={selected}
          {...navigation}
          onGrid={() => openImage(false)}
        />
      )}
      {loading && (
        <div className="ranking-canvas-message" role="status">
          正在读取图片页…
        </div>
      )}
      {!loading && !items.length && (
        <div className="ranking-canvas-message">
          <p>
            {navigation.next
              ? "当前扫描页没有符合条件的图片，可继续下一页。"
              : "当前范围没有图片。"}
          </p>
        </div>
      )}
    </div>
  );
}

function ImageViewer({
  context,
  row,
  onGrid,
  ...navigation
}: Navigation & {
  context: ModuleContext;
  row: RankingRow;
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
      aria-label={`排名大图，候选 ${row.ordinal + 1}`}
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
        <button type="button" aria-label="返回排名网格" onClick={onGrid}>
          <LayoutGrid size={15} />
          网格
        </button>
        <span className="ranking-current-image">
          候选 {row.ordinal + 1} · {rankingLabel(row)}
        </span>
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
            asset={rankingAsset(row)}
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
