import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import {
  Check,
  ChevronLeft,
  ChevronRight,
  Images,
  LayoutGrid,
  Maximize2,
  Image as ImageIcon,
} from "lucide-react";
import { Button, EmptyState } from "@studio/ui";
import type { Asset, AssetKey, ScopeOperation } from "@studio/contracts";
import { assetIdentity } from "@studio/client";
import type { StudioClient } from "@studio/client";
import { AssetImage } from "./AssetImage.js";
export type Scope =
  | { kind: "all" }
  | { kind: "source"; id: string; name: string }
  | { kind: "collection"; id: string; name: string }
  | { kind: "result"; id: string; name: string }
  | { kind: "selection"; name: string };
export interface BrowserProps {
  client: StudioClient;
  projectId: string;
  scope: Scope;
  focus: Asset | null;
  onFocus: (asset: Asset) => void;
  onPick: (keys: AssetKey[], remove?: boolean) => void;
  onScopeOperation: (operation: ScopeOperation) => void;
  selectionRevision: number;
  busy: boolean;
  view: "grid" | "image";
  setView: (view: "grid" | "image") => void;
}
export function Browser({
  client,
  projectId,
  scope,
  focus,
  onFocus,
  onPick,
  onScopeOperation,
  selectionRevision,
  busy,
  view,
  setView,
}: BrowserProps) {
  const [cursors, setCursors] = useState<(string | undefined)[]>([undefined]);
  const [page, setPage] = useState(0);
  const [pageBase, setPageBase] = useState(0);
  const [pageSize, setPageSize] = useState(48);
  const [scopeOperation, setScopeOperation] =
    useState<ScopeOperation>("replace");
  const [size, setSize] = useState(176);
  const scopeKey = JSON.stringify([
    scope,
    scope.kind === "selection" ? selectionRevision : null,
    pageSize,
  ]);
  useEffect(() => {
    setCursors([undefined]);
    setPage(0);
    setPageBase(0);
  }, [projectId, scopeKey]);
  const query = useQuery({
    queryKey: ["project", projectId, "assets", scopeKey, cursors[page]],
    queryFn: async ({ signal }) =>
      scope.kind === "result"
        ? (
            await client.queries.assets(projectId, scope.id, {
              ...(cursors[page] ? { cursor: cursors[page] } : {}),
              limit: pageSize,
              signal,
            })
          ).page
        : client.assets(projectId, {
            ...(scope.kind === "source" ? { sourceId: scope.id } : {}),
            ...(scope.kind === "collection" ? { collectionId: scope.id } : {}),
            ...(scope.kind === "selection" ? { selection: true } : {}),
            ...(cursors[page] ? { cursor: cursors[page] } : {}),
            limit: pageSize,
            signal,
          }),
    gcTime: 0,
    retry: 1,
  });
  const items = query.data?.items ?? [];
  function next() {
    const cursor = query.data?.next_cursor;
    if (cursor) {
      const history = [...cursors.slice(0, page + 1), cursor];
      if (history.length > 128) {
        history.shift();
        setPageBase((base) => base + 1);
      }
      setCursors(history);
      setPage(Math.min(page + 1, 127));
    }
  }
  return (
    <section className="browser-view">
      <div className="content-bar">
        <span>{scope.kind === "all" ? "项目全部数据" : scope.name}</span>
        <span className="subtle">/ 存储对象</span>
        <span className="grow" />
        {(scope.kind === "result" || scope.kind === "collection") && (
          <div className="scope-actions">
            <select
              aria-label="范围选择操作"
              value={scopeOperation}
              onChange={(event) =>
                setScopeOperation(event.target.value as ScopeOperation)
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
          className={view === "grid" ? "icon-button active" : "icon-button"}
          title="图像网格"
          onClick={() => setView("grid")}
        >
          <LayoutGrid size={16} />
        </button>
        <button
          className={view === "image" ? "icon-button active" : "icon-button"}
          title="单图查看"
          onClick={() => {
            if (!focus && items[0]) onFocus(items[0]);
            setView("image");
          }}
        >
          <ImageIcon size={16} />
        </button>
      </div>
      <div className="browser-options">
        <span className="subtle">
          {query.isFetching ? "读取中…" : items.length + " 项 / 本页"}
        </span>
        <span className="grow" />
        {view === "grid" && (
          <label className="density-label">
            缩略图
            <input
              aria-label="缩略图尺寸"
              type="range"
              min="128"
              max="256"
              step="16"
              value={size}
              onChange={(e) => setSize(Number(e.target.value))}
            />
          </label>
        )}
        <Button
          disabled={!items.length || busy}
          onClick={() => onPick(items.map((a) => a.key))}
        >
          选择本页
        </Button>
      </div>
      {query.error ? (
        <EmptyState title="当前范围暂不可用" icon={<ImageIcon size={36} />}>
          <p>{query.error.message}</p>
          <Button
            onClick={() => {
              setCursors([undefined]);
              setPage(0);
              setPageBase(0);
              void query.refetch();
            }}
          >
            重新读取
          </Button>
        </EmptyState>
      ) : !items.length && !query.isFetching ? (
        <EmptyState title="这里还没有可查看的资料" icon={<Images size={42} />}>
          <p>从左侧为项目添加数据湖，或切换到其他范围。</p>
        </EmptyState>
      ) : view === "image" && focus ? (
        <div className="image-canvas">
          <AssetImage
            client={client}
            projectId={projectId}
            asset={focus}
            edge={1200}
          />
          <div className="canvas-caption">
            {focus.name} <span>适合窗口</span>
          </div>
        </div>
      ) : (
        <div className="asset-scroll">
          <div
            className="asset-grid"
            style={{
              gridTemplateColumns:
                "repeat(auto-fill,minmax(" + size + "px,1fr))",
            }}
          >
            {items.map((asset) => (
              <article
                key={assetIdentity(asset.key)}
                className={
                  "asset-card " +
                  (asset.selected ? "selected " : "") +
                  (focus &&
                  assetIdentity(focus.key) === assetIdentity(asset.key)
                    ? "focused"
                    : "")
                }
              >
                <div
                  className="asset-thumb"
                  tabIndex={0}
                  role="button"
                  aria-label={"查看 " + asset.name}
                  onClick={() => onFocus(asset)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") onFocus(asset);
                  }}
                  onDoubleClick={() => {
                    onFocus(asset);
                    setView("image");
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
                    aria-label={
                      (asset.selected ? "取消选择 " : "选择 ") + asset.name
                    }
                    aria-pressed={asset.selected}
                    onClick={(e) => {
                      e.stopPropagation();
                      onPick([asset.key], asset.selected);
                    }}
                  >
                    {asset.selected && <Check size={13} />}
                  </button>
                  <span className="file-kind">
                    {asset.extension.toUpperCase()}
                  </span>
                  <button
                    className="asset-zoom"
                    title="单图查看"
                    onClick={(e) => {
                      e.stopPropagation();
                      onFocus(asset);
                      setView("image");
                    }}
                  >
                    <Maximize2 size={13} />
                  </button>
                </div>
                <div className="asset-caption">
                  <span title={asset.name}>{asset.name}</span>
                  <small>{asset.source_name}</small>
                </div>
              </article>
            ))}
          </div>
        </div>
      )}
      <div className="paging">
        <span className="subtle">按数据湖与内容身份分页</span>
        <span className="grow" />
        <select
          aria-label="每页数量"
          value={pageSize}
          onChange={(event) => setPageSize(Number(event.target.value))}
        >
          <option value={12}>12 项 / 页</option>
          <option value={48}>48 项 / 页</option>
          <option value={96}>96 项 / 页</option>
        </select>
        {pageBase > 0 && (
          <button
            onClick={() => {
              setCursors([undefined]);
              setPage(0);
              setPageBase(0);
            }}
          >
            第一页
          </button>
        )}
        <button
          className="icon-button"
          disabled={page === 0}
          aria-label="上一页"
          onClick={() => setPage((p) => p - 1)}
        >
          <ChevronLeft size={15} />
        </button>
        <span>第 {pageBase + page + 1} 页</span>
        <button
          className="icon-button"
          disabled={!query.data?.next_cursor || query.isFetching}
          aria-label="下一页"
          onClick={next}
        >
          <ChevronRight size={15} />
        </button>
      </div>
    </section>
  );
}
