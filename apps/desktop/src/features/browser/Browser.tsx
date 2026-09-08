import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { CSSProperties, ReactNode } from "react";
import {
  Check,
  ChevronLeft,
  ChevronRight,
  Images,
  LayoutGrid,
  Maximize2,
  Image as ImageIcon,
  LoaderCircle,
  RotateCw,
} from "lucide-react";
import {
  Button,
  EmptyState,
  ErrorDetails,
  CopyButton,
  assetTitle,
  assetSummaryNote,
  initialHistory,
  restoreHistory,
  nextHistory,
} from "@studio/ui";
import type { ModuleContext, BrowseScope, BrowseViewProps } from "@studio/ui";
import { assetIdentity } from "@studio/client";
import type { StudioClient } from "@studio/client";
import type { Asset, ScopeOperation, QueryResult } from "@studio/contracts";
import { AssetImage } from "./AssetImage.js";
import { QuickFilters, adoptRefreshedFilter } from "./QuickFilters.js";
export type Scope = BrowseScope;
export interface BrowserProps extends BrowseViewProps {
  client: StudioClient;
  projectId: string;
  filters?: ReactNode;
  onOpenQuery: () => void;
  onRefreshed?: (previousId: string, result: QueryResult) => void;
}
export default function BrowserModule(context: ModuleContext) {
  return (
    <Browser
      client={context.client}
      projectId={context.projectId}
      {...context.browser}
      onOpenQuery={() => context.openPanel("core.query")}
      onRefreshed={(previousId, result) =>
        adoptRefreshedFilter(
          context.client,
          context.projectId,
          previousId,
          result,
        )
      }
      filters={
        !context.panels.includes("core.query") ? (
          <QuickFilters context={context} />
        ) : null
      }
    />
  );
}
export function Browser(props: BrowserProps) {
  const identity = JSON.stringify([
    props.projectId,
    props.scope,
    props.order,
    props.scope.kind === "selection" ? props.selectionRevision : null,
  ]);
  return <BrowserContent key={identity} {...props} />;
}
function BrowserContent({
  client,
  projectId,
  scope,
  order,
  onOrder,
  onScope,
  focus,
  focusPending,
  onFocus,
  onInspect,
  onPick,
  onScopeOperation,
  busy,
  view,
  setView,
  position,
  onPosition,
  thumbnailSize,
  onThumbnailSize,
  selectionRevision,
  filters,
  onOpenQuery,
  onRefreshed,
}: BrowserProps) {
  const [pageSize, setPageSize] = useState(position?.pageSize ?? 48);
  const queryCache = useQueryClient();
  const [refreshId, setRefreshId] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [refreshError, setRefreshError] = useState<unknown>(null);
  const scopeKey = JSON.stringify([
    scope,
    scope.kind === "selection" ? selectionRevision : null,
    pageSize,
    order,
  ]);
  const [history, setHistory] = useState(() =>
    restoreHistory(position, scopeKey),
  );
  const [notice, setNotice] = useState("");
  const [pendingFocus, setPendingFocus] = useState<"first" | "last" | null>(
    null,
  );
  const [scrollTop, setScrollTop] = useState(
    position?.scopeKey === scopeKey ? (position.scrollTop ?? 0) : 0,
  );
  const [scopeOperation, setScopeOperation] =
    useState<ScopeOperation>("replace");
  const restoreCheck = useRef(
    position?.scopeKey === scopeKey ? position : null,
  );
  const savedScroll = useRef(scrollTop);
  const scrollTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const gridRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLDivElement>(null);
  const restoreCanvasFocus = useRef(false);
  const selectionAnchor = useRef<number | null>(null);
  const focusCallback = useRef(onFocus);
  focusCallback.current = onFocus;
  const positionCallback = useRef(onPosition);
  positionCallback.current = onPosition;
  const cursor = history.cursors[history.index] ?? null;
  const pageNumber = history.firstPage + history.index;
  const query = useQuery({
    queryKey: ["project", projectId, "assets", scopeKey, cursor],
    queryFn: async ({ signal }) =>
      scope.kind === "result"
        ? (
            await client.queries.assets(projectId, scope.id, {
              ...(cursor ? { cursor } : {}),
              limit: pageSize,
              order,
              signal,
            })
          ).page
        : client.assets(projectId, {
            ...(scope.kind === "source" ? { sourceId: scope.id } : {}),
            ...(scope.kind === "collection" ? { collectionId: scope.id } : {}),
            ...(scope.kind === "selection" ? { selection: true } : {}),
            ...(cursor ? { cursor } : {}),
            limit: pageSize,
            order,
            signal,
          }),
    gcTime: 0,
    retry: 1,
    refetchInterval: (q) =>
      q.state.status !== "error" && q.state.data?.preparing ? 800 : false,
  });
  const validity = useQuery({
    queryKey: [
      "project",
      projectId,
      "result-validity",
      scope.kind === "result" ? scope.id : null,
    ],
    queryFn: ({ signal }) =>
      client.queries.validity(
        projectId,
        scope.kind === "result" ? scope.id : "",
        signal,
      ),
    enabled: scope.kind === "result",
    refetchInterval: 30000,
  });
  const refreshed = useQuery({
    queryKey: ["project", projectId, "result-refresh", refreshId],
    queryFn: ({ signal }) =>
      client.queries.result(projectId, refreshId!, signal),
    enabled: !!refreshId,
    refetchInterval: (q) =>
      q.state.data && !["queued", "running"].includes(q.state.data.state)
        ? false
        : 800,
  });
  useEffect(() => {
    if (refreshed.data?.state === "ready" && scope.kind === "result") {
      onRefreshed?.(scope.id, refreshed.data);
      onScope({ ...scope, id: refreshed.data.id });
      void queryCache.invalidateQueries({
        queryKey: ["project", projectId, "query-results"],
      });
    }
  }, [refreshed.data, scope, onScope, onRefreshed, queryCache, projectId]);
  const heldResult =
    scope.kind === "result"
      ? scope.id
      : query.data?.preparing
        ? undefined
        : query.data?.result_id;
  useEffect(() => {
    if (!heldResult) return;
    const leaseId = crypto.randomUUID();
    let stopped = false;
    const renew = () =>
      void client.queries.lease(projectId, heldResult, leaseId).catch(() => {
        if (!stopped) void query.refetch();
      });
    renew();
    const timer = setInterval(renew, 20000);
    return () => {
      stopped = true;
      clearInterval(timer);
      void client.queries
        .lease(projectId, heldResult, leaseId, true)
        .catch(() => {});
    };
  }, [client, projectId, heldResult]);
  async function refreshResult() {
    if (scope.kind !== "result") {
      if (query.error && query.data?.result_id)
        await client.queries
          .release(projectId, query.data.result_id)
          .catch(() => {});
      first();
      void query.refetch();
      return;
    }
    setRefreshing(true);
    setRefreshError(null);
    try {
      const previous = await client.queries.result(projectId, scope.id);
      const next = await client.queries.runLatest(projectId, {
        ...previous.spec,
        version: 3,
        order,
      });
      if (next.state === "ready") {
        onRefreshed?.(scope.id, next);
        onScope({ ...scope, id: next.id });
      } else setRefreshId(next.id);
    } catch (error) {
      setRefreshError(error);
    } finally {
      setRefreshing(false);
    }
  }
  const items = query.data?.items ?? [];
  const focusIndex = focus
    ? items.findIndex((a) => assetIdentity(a.key) === assetIdentity(focus.key))
    : -1;
  const activeAsset = focusIndex >= 0 ? items[focusIndex]! : focus;
  const waiting =
    query.isFetching ||
    !!query.data?.preparing ||
    focusPending ||
    !!pendingFocus;
  useEffect(() => {
    if (view === "image") canvasRef.current?.focus({ preventScroll: true });
  }, [view]);
  useEffect(() => {
    if (view !== "image" || waiting || !restoreCanvasFocus.current) return;
    restoreCanvasFocus.current = false;
    const active = document.activeElement;
    if (active === document.body || active?.closest(".image-canvas"))
      canvasRef.current?.focus({ preventScroll: true });
  }, [view, waiting]);
  useEffect(() => {
    if (!query.data || query.isFetching || query.data.preparing) return;
    const expected = restoreCheck.current;
    restoreCheck.current = null;
    if (
      expected &&
      (expected.version !== query.data.revision ||
        (expected.anchor &&
          query.data.items[0] &&
          assetIdentity(expected.anchor) !==
            assetIdentity(query.data.items[0].key)))
    ) {
      setHistory(initialHistory());
      setScrollTop(0);
      savedScroll.current = 0;
      setNotice("来源已更新，已返回第一页。");
      return;
    }
    if (
      items.length &&
      (pendingFocus || (view === "image" && !focusPending && focusIndex < 0))
    ) {
      const target =
        pendingFocus === "last" ? items[items.length - 1] : items[0];
      if (target) focusCallback.current(target);
      setPendingFocus(null);
    }
  }, [
    query.data,
    query.isFetching,
    pendingFocus,
    view,
    focusPending,
    focusIndex,
    items,
  ]);
  useEffect(() => {
    if (!query.data || query.isFetching || query.data.preparing) return;
    positionCallback.current({
      scopeKey,
      cursor,
      pageNumber,
      pageSize,
      anchor: query.data.items[0]?.key ?? null,
      version: query.data.revision,
      history,
      scrollTop,
    });
  }, [
    query.data,
    query.isFetching,
    scopeKey,
    cursor,
    pageNumber,
    pageSize,
    history,
    scrollTop,
  ]);
  useEffect(() => {
    if (
      query.error &&
      cursor &&
      "code" in query.error &&
      ["SOURCE_CHANGED", "INVALID_INPUT", "REVISION_CONFLICT"].includes(
        String(query.error.code),
      )
    ) {
      restoreCheck.current = null;
      setHistory(initialHistory());
      setScrollTop(0);
      savedScroll.current = 0;
      setNotice("范围已更新，已返回第一页。");
    }
  }, [query.error, cursor]);
  useLayoutEffect(() => {
    if (view !== "grid" || !query.data) return;
    if (scrollRef.current) scrollRef.current.scrollTop = savedScroll.current;
    const focused = gridRef.current?.querySelector<HTMLElement>(
      ".asset-card.focused",
    );
    focused?.scrollIntoView({ block: "nearest" });
  }, [view, query.data?.revision, cursor]);
  useEffect(
    () => () => {
      if (scrollTimer.current) clearTimeout(scrollTimer.current);
    },
    [],
  );
  useEffect(() => {
    const next = query.data?.next_cursor;
    if (!next || query.isFetching || view !== "grid") return;
    const abort = new AbortController();
    const timer = setTimeout(() => {
      const options = {
        cursor: next,
        limit: 4,
        order,
        signal: abort.signal,
        priority: "prefetch" as const,
      };
      const page =
        scope.kind === "result"
          ? client.queries
              .assets(projectId, scope.id, options)
              .then((r) => r.page)
          : client.assets(projectId, {
              ...options,
              ...(scope.kind === "source" ? { sourceId: scope.id } : {}),
              ...(scope.kind === "collection"
                ? { collectionId: scope.id }
                : {}),
              ...(scope.kind === "selection" ? { selection: true } : {}),
            });
      void page
        .then((p) =>
          Promise.allSettled(
            p.items.map((asset) =>
              client
                .acquireMedia(projectId, asset, 360, {
                  signal: abort.signal,
                  priority: "prefetch",
                })
                .then((h) => h.release()),
            ),
          ),
        )
        .catch(() => {});
    }, 450);
    return () => {
      clearTimeout(timer);
      abort.abort();
    };
  }, [
    client,
    projectId,
    scope,
    query.data?.next_cursor,
    query.isFetching,
    view,
    order,
  ]);
  function first() {
    restoreCheck.current = null;
    setHistory(initialHistory());
    setScrollTop(0);
    savedScroll.current = 0;
    if (view === "image") setPendingFocus("first");
  }
  function turnPage(direction: 1 | -1, imageEdge: "first" | "last" = "first") {
    if (query.isFetching) return;
    if (direction < 0) {
      if (history.index === 0) return;
      setHistory((h) => ({ ...h, index: h.index - 1 }));
    } else {
      const next = query.data?.next_cursor;
      if (!next) return;
      setHistory((h) => nextHistory(h, next));
    }
    restoreCheck.current = null;
    setScrollTop(0);
    savedScroll.current = 0;
    selectionAnchor.current = null;
    if (view === "image") {
      restoreCanvasFocus.current = true;
      setPendingFocus(imageEdge);
    }
  }
  function moveImage(direction: 1 | -1) {
    if (waiting || !items.length) return;
    const index =
      focusIndex < 0
        ? direction > 0
          ? 0
          : items.length - 1
        : focusIndex + direction;
    const target = items[index];
    if (target) {
      onFocus(target);
      return;
    }
    turnPage(direction, direction > 0 ? "first" : "last");
  }
  function selectAt(asset: Asset, index: number, range: boolean) {
    if (busy || index < 0 || index >= items.length) return;
    if (range) {
      const anchor =
        selectionAnchor.current ?? (focusIndex < 0 ? index : focusIndex);
      onPick(
        items
          .slice(Math.min(anchor, index), Math.max(anchor, index) + 1)
          .map((a) => a.key),
        asset.selected,
      );
    } else {
      selectionAnchor.current = index;
      onPick([asset.key], asset.selected);
    }
  }
  function choose(
    asset: Asset,
    index: number,
    event: { shiftKey: boolean; ctrlKey: boolean; metaKey: boolean },
  ) {
    onFocus(asset);
    if (event.shiftKey && !busy) {
      selectAt(asset, index, true);
    } else {
      selectionAnchor.current = index;
      if ((event.ctrlKey || event.metaKey) && !busy)
        selectAt(asset, index, false);
    }
  }
  const empty =
    scope.kind === "selection"
      ? {
          title: "当前还没有选择图片",
          text: "切换到数据湖，勾选图片后会显示在这里。",
        }
      : scope.kind === "collection"
        ? {
            title: "这个工作集没有图片",
            text: "可以从其他范围重新选择图片并保存为工作集。",
          }
        : scope.kind === "result"
          ? {
              title: "没有符合条件的图片",
              text: "可以放宽分级或标签条件，再次执行筛选。",
            }
          : {
              title: "这里还没有可查看的资料",
              text: "从左侧添加数据湖，或切换到已有的数据范围。",
            };
  return (
    <section
      className="browser-view"
      aria-label="资料浏览"
      onKeyDown={(e) => {
        if (
          (e.target as HTMLElement).closest(
            "input,select,textarea,summary,a,[contenteditable=true]",
          )
        )
          return;
        const button = (e.target as HTMLElement).closest("button");
        if (
          button &&
          (!button.classList.contains("image-step") || e.key === " ")
        )
          return;
        if (view === "image") {
          if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
            e.preventDefault();
            moveImage(e.key === "ArrowLeft" ? -1 : 1);
          }
          if (e.key === "Escape") {
            e.preventDefault();
            setView("grid");
          }
          if (e.key === " " && activeAsset && !busy && !waiting) {
            e.preventDefault();
            selectAt(activeAsset, focusIndex, e.shiftKey);
          }
        } else if (
          (e.ctrlKey || e.metaKey) &&
          e.key.toLowerCase() === "a" &&
          !busy &&
          !query.isFetching
        ) {
          e.preventDefault();
          onPick(items.map((a) => a.key));
        }
      }}
    >
      <div className="content-bar">
        <span title={scope.kind === "all" ? "全部项目数据" : scope.name}>
          {scope.kind === "all" ? "全部项目数据" : scope.name}
        </span>
        <span className="subtle">浏览范围</span>
        <span className="grow" />
        {(scope.kind === "result" || scope.kind === "collection") && (
          <div className="scope-actions">
            <select
              aria-label="范围选择操作"
              value={scopeOperation}
              onChange={(e) =>
                setScopeOperation(e.target.value as ScopeOperation)
              }
            >
              <option value="replace">替换选择</option>
              <option value="add">添加到选择</option>
              <option value="remove">从选择移除</option>
              <option value="intersect">与选择取交集</option>
            </select>
            <Button
              disabled={busy || !!query.error || query.isFetching}
              onClick={() => onScopeOperation(scopeOperation)}
            >
              {scopeOperation === "replace" ? "选择全部范围" : "应用范围"}
            </Button>
          </div>
        )}
        <button
          className={"icon-button " + (view === "grid" ? "active" : "")}
          title="图像网格"
          aria-pressed={view === "grid"}
          onClick={() => setView("grid")}
        >
          <LayoutGrid size={17} />
        </button>
        <button
          className={"icon-button " + (view === "image" ? "active" : "")}
          title="单图查看"
          aria-pressed={view === "image"}
          disabled={!items.length}
          onClick={() => {
            if (focusIndex < 0 && items[0]) onFocus(items[0]);
            setView("image");
          }}
        >
          <ImageIcon size={17} />
        </button>
      </div>
      {filters}
      {scope.kind === "result" &&
        (validity.data?.current === false ||
          (query.error &&
            "code" in query.error &&
            ["SORT_REQUIRES_REFRESH", "RESULT_NOT_READY"].includes(
              String(query.error.code),
            ))) && (
          <div className="browser-refresh-note">
            <span>结果需要刷新；将按最新来源检查条件。</span>
            <Button
              disabled={refreshing || (!!refreshId && !refreshed.data?.error)}
              onClick={() => void refreshResult()}
            >
              <RotateCw size={14} />
              {refreshing ||
              (refreshId &&
                ["queued", "running"].includes(
                  refreshed.data?.state ?? "queued",
                ))
                ? "刷新中…"
                : "刷新结果"}
            </Button>
          </div>
        )}
      {!!(refreshError || refreshed.error || refreshed.data?.error) && (
        <ErrorDetails
          compact
          error={refreshError || refreshed.error || refreshed.data?.error}
        />
      )}
      <div className="browser-options">
        {notice && (
          <span role="status" className="subtle">
            {notice}
          </span>
        )}
        <span className="subtle">
          {query.data?.preparing ??
            (query.isFetching ? "读取中…" : items.length + " 张 / 本页")}
        </span>
        <span className="grow" />
        <select
          aria-label="浏览排序"
          title="同图关联多个帖子时取最小 ID；无帖子 ID 的图像排在末尾"
          value={order}
          onChange={(e) => onOrder(e.target.value as BrowserProps["order"])}
        >
          <option value="post_id_desc">Danbooru ID 从新到旧</option>
          <option value="post_id_asc">Danbooru ID 从旧到新</option>
          <option value="asset_key_asc">图像身份升序</option>
          <option value="asset_key_desc">图像身份降序</option>
        </select>
        <button
          className="icon-button"
          title="刷新当前范围"
          disabled={waiting || refreshing}
          onClick={() => void refreshResult()}
        >
          <RotateCw size={14} />
        </button>
        {view === "grid" ? (
          <>
            <label className="density-label">
              预览尺寸
              <input
                aria-label="缩略图尺寸"
                type="range"
                min={128}
                max={320}
                step={16}
                value={thumbnailSize}
                onChange={(e) => onThumbnailSize(Number(e.target.value))}
              />
            </label>
            <Button
              title="选择本页（Ctrl+A）"
              disabled={!items.length || busy || query.isFetching}
              onClick={() => onPick(items.map((a) => a.key))}
            >
              选择本页
            </Button>
          </>
        ) : (
          <Button
            disabled={!activeAsset || busy || waiting}
            aria-pressed={activeAsset?.selected ?? false}
            onClick={() => {
              if (activeAsset) onPick([activeAsset.key], activeAsset.selected);
            }}
          >
            {activeAsset?.selected ? <Check size={14} /> : null}
            {activeAsset?.selected ? "取消选择当前图像" : "选择当前图像"}
          </Button>
        )}
      </div>
      {query.error ? (
        <EmptyState title="当前范围暂不可用" icon={<ImageIcon size={36} />}>
          <ErrorDetails error={query.error} />
          <Button onClick={() => void refreshResult()}>重新读取</Button>
        </EmptyState>
      ) : query.data?.preparing ? (
        <EmptyState
          title={query.data.preparing}
          icon={<LoaderCircle className="loading-icon" size={36} />}
        >
          <p>准备完成后会自动显示图像。</p>
        </EmptyState>
      ) : !items.length && !query.isFetching ? (
        <EmptyState title={empty.title} icon={<Images size={40} />}>
          <p>{empty.text}</p>
          {scope.kind === "result" && (
            <Button onClick={onOpenQuery}>调整查询条件</Button>
          )}
        </EmptyState>
      ) : view === "image" ? (
        <div
          className="image-canvas"
          ref={canvasRef}
          tabIndex={0}
          aria-label="单图查看画布"
        >
          {activeAsset && (
            <AssetImage
              client={client}
              projectId={projectId}
              asset={activeAsset}
              edge={1600}
            />
          )}
          {waiting && (
            <div className="image-loading" role="status">
              <LoaderCircle className="loading-icon" size={18} />
              正在载入当前图像…
            </div>
          )}
          <button
            className="image-step previous"
            aria-label="上一张"
            disabled={waiting || (focusIndex <= 0 && history.index === 0)}
            onClick={() => moveImage(-1)}
          >
            <ChevronLeft size={25} />
          </button>
          <button
            className="image-step next"
            aria-label="下一张"
            disabled={
              waiting ||
              (focusIndex >= items.length - 1 && !query.data?.next_cursor)
            }
            onClick={() => moveImage(1)}
          >
            <ChevronRight size={25} />
          </button>
          {activeAsset && (
            <div className="canvas-caption">
              <strong>{assetTitle(activeAsset)}</strong>
              <CopyButton
                label="复制图像身份"
                text={activeAsset.key.asset_id}
              />
              <span className="grow" />
              <span>适合窗口 · 预览</span>
            </div>
          )}
        </div>
      ) : (
        <div
          className="asset-scroll"
          ref={scrollRef}
          onScroll={(e) => {
            savedScroll.current = e.currentTarget.scrollTop;
            if (scrollTimer.current) clearTimeout(scrollTimer.current);
            scrollTimer.current = setTimeout(
              () => setScrollTop(savedScroll.current),
              120,
            );
          }}
        >
          <div
            className="asset-grid"
            ref={gridRef}
            style={
              {
                gridTemplateColumns:
                  "repeat(auto-fill,minmax(" + thumbnailSize + "px,1fr))",
                "--thumbnail-size": thumbnailSize + "px",
              } as CSSProperties
            }
          >
            {items.map((asset, index) => (
              <article
                key={assetIdentity(asset.key)}
                className={
                  "asset-card " +
                  (asset.selected ? "selected " : "") +
                  (focusIndex === index ? "focused" : "")
                }
              >
                <div
                  className="asset-thumb"
                  tabIndex={
                    focusIndex === index || (focusIndex < 0 && index === 0)
                      ? 0
                      : -1
                  }
                  role="button"
                  aria-label={"查看 " + assetTitle(asset)}
                  onClick={(e) => choose(asset, index, e)}
                  onDoubleClick={() => {
                    onFocus(asset);
                    setView("image");
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      onFocus(asset);
                      setView("image");
                    } else if (e.key === " ") {
                      e.preventDefault();
                      selectAt(asset, index, e.shiftKey);
                    } else if (
                      [
                        "ArrowLeft",
                        "ArrowRight",
                        "ArrowUp",
                        "ArrowDown",
                      ].includes(e.key)
                    ) {
                      e.preventDefault();
                      const columns = gridRef.current
                        ? getComputedStyle(
                            gridRef.current,
                          ).gridTemplateColumns.split(" ").length
                        : 1;
                      const offset =
                        e.key === "ArrowLeft"
                          ? -1
                          : e.key === "ArrowRight"
                            ? 1
                            : e.key === "ArrowUp"
                              ? -columns
                              : columns;
                      const next = Math.max(
                        0,
                        Math.min(items.length - 1, index + offset),
                      );
                      const target = items[next];
                      if (target) {
                        onFocus(target);
                        gridRef.current
                          ?.querySelectorAll<HTMLElement>(".asset-thumb")
                          .item(next)
                          ?.focus();
                      }
                    }
                  }}
                >
                  <AssetImage
                    client={client}
                    projectId={projectId}
                    asset={asset}
                  />
                  <button
                    disabled={busy}
                    className="asset-check"
                    tabIndex={-1}
                    aria-label={
                      (asset.selected ? "取消选择 " : "选择 ") +
                      assetTitle(asset)
                    }
                    aria-pressed={asset.selected}
                    title="Shift：连续选中或取消本页区间，以末尾项当前状态为准"
                    onClick={(e) => {
                      e.stopPropagation();
                      selectAt(asset, index, e.shiftKey);
                    }}
                  >
                    {asset.selected && <Check size={14} />}
                  </button>
                  <span className="file-kind">
                    {asset.extension.toUpperCase()}
                  </span>
                  <button
                    className="asset-zoom"
                    tabIndex={-1}
                    title="单图查看"
                    onClick={(e) => {
                      e.stopPropagation();
                      onFocus(asset);
                      setView("image");
                    }}
                  >
                    <Maximize2 size={14} />
                  </button>
                </div>
                <div className="asset-caption">
                  <span title={assetTitle(asset)}>{assetTitle(asset)}</span>
                  {Number(asset.summary?.post_count ?? 0) > 1 ? (
                    <button
                      className="asset-links"
                      title={asset.summary?.post_ids
                        .map((id) => "#" + id)
                        .join("、")}
                      onClick={() => onInspect(asset)}
                    >
                      {assetSummaryNote(asset)}
                    </button>
                  ) : asset.summary?.status === "unavailable" ? (
                    <button
                      className="asset-summary-unavailable"
                      title={asset.summary.issue ?? ""}
                      onClick={() => onInspect(asset)}
                    >
                      帖子 ID 暂不可用
                    </button>
                  ) : scope.kind !== "source" ? (
                    <small>{asset.source_name}</small>
                  ) : null}
                </div>
              </article>
            ))}
          </div>
        </div>
      )}
      <div className="paging">
        {view === "grid" ? (
          <>
            <span className="subtle">
              Space 选择 · Enter 单图 · Shift 区间选中 / 取消
            </span>
            <span className="grow" />
            <select
              aria-label="每页数量"
              value={pageSize}
              onChange={(e) => {
                restoreCheck.current = null;
                setHistory(initialHistory());
                setPageSize(Number(e.target.value));
                setScrollTop(0);
                savedScroll.current = 0;
              }}
            >
              <option value={12}>12 张 / 页</option>
              <option value={48}>48 张 / 页</option>
              <option value={96}>96 张 / 页</option>
            </select>
            {pageNumber > 1 && <button onClick={first}>第一页</button>}
            <button
              className="icon-button"
              aria-label="上一页"
              title={
                history.index === 0 && pageNumber > 1
                  ? "较早记录已退出浏览历史，可返回第一页"
                  : "上一页"
              }
              disabled={history.index === 0 || query.isFetching}
              onClick={() => turnPage(-1)}
            >
              <ChevronLeft size={17} />
            </button>
            <span>第 {pageNumber} 页</span>
            <button
              className="icon-button"
              aria-label="下一页"
              disabled={!query.data?.next_cursor || query.isFetching}
              onClick={() => turnPage(1)}
            >
              <ChevronRight size={17} />
            </button>
          </>
        ) : (
          <>
            <button onClick={() => setView("grid")}>
              <LayoutGrid size={15} />
              返回网格
            </button>
            <span className="subtle">← → 逐张浏览 · Space 选择 · Esc 返回</span>
            <span className="grow" />
            <span aria-label="当前图像位置">
              第 {pageNumber} 页 · {focusIndex >= 0 ? focusIndex + 1 : "—"} /{" "}
              {items.length} 张
            </span>
          </>
        )}
      </div>
    </section>
  );
}
