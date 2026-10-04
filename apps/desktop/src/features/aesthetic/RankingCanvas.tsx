import { useEffect, useLayoutEffect, useRef } from "react";
import type { CSSProperties, KeyboardEvent } from "react";
import { Shield } from "lucide-react";
import { AssetViewer } from "./AssetViewer.js";
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
        <AssetViewer
          key={`${selected.key.source_id}:${selected.key.asset_id}`}
          context={context}
          asset={rankingAsset(selected)}
          title={`候选 ${selected.ordinal + 1} · ${rankingLabel(selected)}`}
          ariaLabel={`排名大图，候选 ${selected.ordinal + 1}`}
          backLabel="返回排名网格"
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
