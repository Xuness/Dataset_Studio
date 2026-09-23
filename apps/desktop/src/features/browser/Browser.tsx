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
  Filter,
  LocateFixed,
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
  browseScopeIdentity,
  normalizeBrowseScopeKey,
  WorkbenchPanelPortal,
  useWorkbenchPanels,
} from "@studio/ui";
import type { ModuleContext, BrowseScope, BrowseViewProps } from "@studio/ui";
import { assetIdentity } from "@studio/client";
import type { StudioClient } from "@studio/client";
import type {
  Asset,
  AssetPage,
  ScopeOperation,
  QueryResult,
} from "@studio/contracts";
import { AssetImage } from "./AssetImage.js";
import { QuickFilters, adoptRefreshedFilter } from "./QuickFilters.js";
import {
  useRankingBrowse,
  RankingBadge,
  RankingStart,
} from "./rankingBrowse.js";
import type { RankingBrowseState } from "./rankingBrowse.js";
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
      filters={<QuickFilters context={context} presentation="panel" />}
    />
  );
}
export function Browser(props: BrowserProps) {
  const ranked = useRankingBrowse(
    props.client,
    props.projectId,
    props.scope,
    props.rankedBrowse,
    props.onRankedBrowse,
  );
  const leasedScope =
    ranked.active && ranked.target ? JSON.stringify(ranked.target) : null;
  useEffect(() => {
    if (!leasedScope) return;
    const scope = JSON.parse(leasedScope) as NonNullable<typeof ranked.target>;
    const id = crypto.randomUUID();
    const renew = () =>
      void props.client.ranking
        .leaseScope(props.projectId, scope, id)
        .catch(() => {});
    renew();
    const timer = setInterval(renew, 20000);
    return () => {
      clearInterval(timer);
      void props.client.ranking
        .leaseScope(props.projectId, scope, id, true)
        .catch(() => {});
    };
  }, [props.client, props.projectId, leasedScope]);
  const identity = JSON.stringify([
    props.projectId,
    browseScopeIdentity(props.scope),
    props.order,
    ranked.key,
    ranked.info?.artifact_id,
    props.scope.kind === "selection" ? props.selectionRevision : null,
  ]);
  const position = props.position
    ? {
        ...props.position,
        scopeKey: normalizeBrowseScopeKey(props.position.scopeKey),
      }
    : null;
  return (
    <BrowserContent
      key={identity}
      {...props}
      position={position}
      ranked={ranked}
    />
  );
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
  ranked,
}: BrowserProps & { ranked: RankingBrowseState }) {
  const [pageSize, setPageSize] = useState(position?.pageSize ?? 48);
  const queryCache = useQueryClient();
  const [refreshId, setRefreshId] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [refreshError, setRefreshError] = useState<unknown>(null);
  const scopeKey = JSON.stringify([
    browseScopeIdentity(scope),
    scope.kind === "selection" ? selectionRevision : null,
    pageSize,
    ranked.active ? ranked.key : order,
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
  const workbench = useWorkbenchPanels();
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
  const preparation = useRef<{ key: string; page: AssetPage } | null>(null);
  const requestKey = JSON.stringify([
    client.connection.instance_id,
    scopeKey,
    cursor,
  ]);
  const query = useQuery({
    queryKey: ["project", projectId, "assets", scopeKey, cursor],
    queryFn: async ({ signal }) => {
      if (ranked.infoError) throw ranked.infoError;
      if (ranked.active && ranked.target) {
        const pending =
          preparation.current?.key === requestKey
            ? preparation.current.page
            : null;
        const continuation =
          pending?.next_cursor ?? cursor ?? ranked.settings.startCursor;
        const page = await client.ranking.browseAssets(
          projectId,
          {
            scope: ranked.target,
            ...(ranked.settings.sort !== "saved" &&
            ranked.settings.sort !== "off"
              ? { order: ranked.settings.sort }
              : {}),
            descending: ranked.settings.descending,
            ...(ranked.settings.startPostId
              ? { start_post_id: ranked.settings.startPostId }
              : {}),
            ...(ranked.settings.startRank
              ? {
                  start_rank: ranked.settings.startRank,
                  start_rating: ranked.settings.startRating ?? null,
                }
              : {}),
            ...(continuation ? { cursor: continuation } : {}),
            limit: pageSize,
          },
          signal,
        );
        if (!signal.aborted)
          preparation.current = page.preparing
            ? { key: requestKey, page }
            : null;
        return page;
      }
      if (scope.kind === "result")
        return (
          await client.queries.assets(projectId, scope.id, {
            ...(cursor ? { cursor } : {}),
            limit: pageSize,
            order,
            signal,
          })
        ).page;
      const pending =
        preparation.current?.key === requestKey
          ? preparation.current.page
          : null;
      if (pending?.result_id) {
        const result = await client.queries.result(
          projectId,
          pending.result_id,
          signal,
        );
        if (["queued", "running"].includes(result.state)) {
          return {
            ...pending,
            ...(pending.scan
              ? { scan: { ...pending.scan, scanned: result.processed } }
              : {}),
          };
        }
        if (result.state !== "ready")
          throw Object.assign(
            new Error(result.error ?? `范围排序未完成（${result.state}）`),
            { code: "SCOPE_SORT_FAILED" },
          );
      }
      const continuation = pending?.next_cursor ?? cursor;
      const page = await client.assets(projectId, {
        ...(scope.kind === "source" ? { sourceId: scope.id } : {}),
        ...(scope.kind === "collection" ? { collectionId: scope.id } : {}),
        ...(scope.kind === "selection" ? { selection: true } : {}),
        ...(continuation ? { cursor: continuation } : {}),
        limit: pageSize,
        order,
        signal,
      });
      if (!signal.aborted)
        preparation.current = page.preparing ? { key: requestKey, page } : null;
      return page;
    },
    gcTime: 60000,
    staleTime: (q) =>
      !q.state.data?.preparing &&
      (ranked.active || scope.kind === "result" || scope.kind === "collection")
        ? 60000
        : 0,
    enabled: !ranked.loading,
    retry: 1,
    refetchInterval: (q) =>
      q.state.status !== "error" && q.state.data?.preparing
        ? ranked.active
          ? 500
          : 800
        : false,
  });
  useEffect(() => {
    if (ranked.active && query.data?.start_cursor)
      ranked.rememberStart(query.data.start_cursor);
  }, [ranked.active, query.data?.start_cursor, ranked.settings.startCursor]);
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
    preparation.current = null;
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
  const pageItems = query.data?.items ?? [];
  const pageKeys = pageItems.map((item) => item.key);
  const summaries = useQuery({
    queryKey: ["project", projectId, "asset-summaries", pageKeys],
    queryFn: ({ signal }) => client.assetSummaries(projectId, pageKeys, signal),
    enabled: pageKeys.length > 0 && !ranked.active && !query.data?.preparing,
    staleTime: 15000,
    gcTime: 0,
    refetchInterval: (q) =>
      q.state.status !== "error" && q.state.data?.preparing ? 800 : false,
  });
  const memberSelection = useQuery({
    queryKey: [
      "project",
      projectId,
      "selection-members",
      selectionRevision,
      pageKeys,
    ],
    queryFn: ({ signal }) =>
      client.selectionMembers(projectId, pageKeys, signal),
    enabled: pageKeys.length > 0 && !query.data?.preparing,
    gcTime: 0,
  });
  const items = pageItems.map((item, index) => ({
    ...item,
    selected: memberSelection.data?.selected[index] ?? item.selected,
    summary: summaries.data?.items[index]?.summary ?? item.summary ?? null,
  }));
  useEffect(() => {
    const inactive = queryCache
      .getQueryCache()
      .findAll({ queryKey: ["project", projectId, "assets"] })
      .filter((entry) => entry.getObserversCount() === 0)
      .sort((a, b) => b.state.dataUpdatedAt - a.state.dataUpdatedAt);
    for (const entry of inactive.slice(32))
      queryCache.removeQueries({ queryKey: entry.queryKey, exact: true });
  }, [queryCache, projectId, query.data]);
  const focusIndex = focus
    ? items.findIndex((a) => assetIdentity(a.key) === assetIdentity(focus.key))
    : -1;
  const activeAsset = focusIndex >= 0 ? items[focusIndex]! : focus;
  const waiting =
    ranked.loading ||
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
    if (
      !next ||
      query.isFetching ||
      query.data?.preparing ||
      query.error ||
      view !== "grid"
    )
      return;
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
        ranked.active && ranked.target
          ? client.ranking.browseAssets(
              projectId,
              {
                scope: ranked.target,
                ...(ranked.settings.sort !== "saved" &&
                ranked.settings.sort !== "off"
                  ? { order: ranked.settings.sort }
                  : {}),
                descending: ranked.settings.descending,
                ...(ranked.settings.startPostId
                  ? { start_post_id: ranked.settings.startPostId }
                  : {}),
                ...(ranked.settings.startRank
                  ? {
                      start_rank: ranked.settings.startRank,
                      start_rating: ranked.settings.startRating ?? null,
                    }
                  : {}),
                cursor: next,
                limit: 4,
              },
              abort.signal,
              "prefetch",
            )
          : scope.kind === "result"
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
    query.data?.preparing,
    query.error,
    query.isFetching,
    view,
    order,
    ranked.active,
    ranked.settings.sort,
    ranked.settings.descending,
    ranked.settings.startPostId,
    ranked.settings.startRank,
    ranked.settings.startRating,
  ]);
  function chooseOrder(value: string) {
    if (value.startsWith("ranking:")) {
      ranked.change({
        sort: value.slice(8) as
          "saved" | "main" | "rescue" | "input" | "direct" | "fused",
      });
    } else {
      if (ranked.info) ranked.change({ sort: "off", startPostId: null });
      onOrder(value as BrowserProps["order"]);
    }
  }
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
      <div className="browser-controls" aria-label="浏览控制区">
        <WorkbenchPanelPortal id="browser.filters">
          {filters}
        </WorkbenchPanelPortal>
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
          <button
            aria-label="Rating / Tag 筛选"
            title="打开筛选面板"
            onClick={() => workbench?.open("browser.filters")}
          >
            <Filter size={14} />
            筛选
          </button>
          <button
            aria-label="定位与显示"
            title="打开定位与显示面板"
            onClick={() => workbench?.open("browser.locate")}
          >
            <LocateFixed size={14} />
            定位
          </button>
          {notice && (
            <span role="status" className="subtle">
              {notice}
            </span>
          )}
          <span className="subtle">
            {query.error
              ? "读取未完成"
              : (query.data?.preparing ??
                (query.isFetching ? "读取中…" : items.length + " 张 / 本页"))}
          </span>
          <span className="grow" />
          <select
            aria-label="浏览排序"
            title={
              ranked.active
                ? "排名沿用计算时的评分，名次在各分级内计算"
                : "同图关联多个帖子时取最小 ID；无帖子 ID 的图像排在末尾"
            }
            value={ranked.active ? "ranking:" + ranked.settings.sort : order}
            onChange={(e) => chooseOrder(e.target.value)}
          >
            {ranked.info && (
              <optgroup label="排名排序">
                <option value="ranking:saved">保存时的榜单顺序</option>
                <option value="ranking:main">
                  {ranked.info.schema_version >= 2 ? "筛选优先级" : "主排名"}
                </option>
                <option value="ranking:rescue">
                  {ranked.info.schema_version >= 2
                    ? "年代相对排名"
                    : "补救排名"}
                </option>
                {ranked.info.schema_version >= 2 && (
                  <>
                    <option value="ranking:direct">直算排名</option>
                    <option value="ranking:fused">融合排名</option>
                  </>
                )}
                <option value="ranking:input">排名输入顺序</option>
              </optgroup>
            )}
            <option value="post_id_desc">Danbooru ID 从新到旧</option>
            <option value="post_id_asc">Danbooru ID 从旧到新</option>
            <option value="asset_key_asc">图像身份升序</option>
            <option value="asset_key_desc">图像身份降序</option>
          </select>
          {ranked.active && (
            <select
              className="ranking-direction"
              aria-label="排名查看方向"
              value={ranked.settings.descending ? "desc" : "asc"}
              onChange={(event) =>
                ranked.change({ descending: event.target.value === "desc" })
              }
            >
              <option value="asc">
                升序 ·{" "}
                {ranked.settings.sort === "input" ||
                (ranked.settings.sort === "saved" &&
                  ranked.info?.saved_filter.order === "input")
                  ? "序号"
                  : "名次"}
                从小到大
              </option>
              <option value="desc">
                降序 ·{" "}
                {ranked.settings.sort === "input" ||
                (ranked.settings.sort === "saved" &&
                  ranked.info?.saved_filter.order === "input")
                  ? "序号"
                  : "名次"}
                从大到小
              </option>
            </select>
          )}
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
                if (activeAsset)
                  onPick([activeAsset.key], activeAsset.selected);
              }}
            >
              {activeAsset?.selected ? <Check size={14} /> : null}
              {activeAsset?.selected ? "取消选择当前图像" : "选择当前图像"}
            </Button>
          )}
        </div>
        <WorkbenchPanelPortal id="browser.locate">
          <div className="browser-navigation">
            <details className="wb-fold" open>
              <summary>显示</summary>
              <div className="wb-fold-body">
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
              </div>
            </details>
            <details className="wb-fold" open>
              <summary>范围内定位</summary>
              <RankingStart
                state={ranked}
                busy={busy || ranked.loading}
                first={first}
              />
              {ranked.info && (
                <div className="ranking-browse-note">
                  {ranked.info.artifact_name} · 主 /{" "}
                  {ranked.info.schema_version >= 2 ? "年代相对" : "补救"}
                  名次与分数按分级独立计算
                  {ranked.active && ranked.settings.descending
                    ? " · 当前沿榜单反向查看"
                    : ""}
                </div>
              )}
              {!ranked.active && (
                <p className="aesthetic-help">
                  排名工作集可按图片 ID 或名次定位；当前范围可使用下方分页浏览。
                </p>
              )}
            </details>
          </div>
        </WorkbenchPanelPortal>
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
          {query.data.scan && (
            <div className="scope-scan-progress">
              <progress
                aria-label="范围成员定位进度"
                value={query.data.scan.scanned}
                max={Math.max(1, query.data.scan.total)}
              />
              <span>
                已检查 {query.data.scan.scanned.toLocaleString("zh-CN")} /{" "}
                {query.data.scan.total.toLocaleString("zh-CN")} 项
              </span>
            </div>
          )}
          <Button onClick={() => chooseOrder("asset_key_asc")}>
            先按图像身份浏览
          </Button>
        </EmptyState>
      ) : ranked.loading ? (
        <EmptyState
          title="正在读取浏览方式…"
          icon={<LoaderCircle className="loading-icon" size={36} />}
        >
          {null}
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
              <RankingBadge ranking={activeAsset.ranking} />
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
                  <RankingBadge ranking={asset.ranking} />
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
