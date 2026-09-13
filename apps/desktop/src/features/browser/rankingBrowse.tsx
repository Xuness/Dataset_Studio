import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { browseScopeIdentity } from "@studio/ui";
import type { BrowseScope, RankedBrowseSettings } from "@studio/ui";
import type { StudioClient } from "@studio/client";
import type { AssetRanking, ScopeRef } from "@studio/contracts";
import "./rankingBrowse.css";

export function rankableScope(
  projectId: string,
  scope: BrowseScope,
): ScopeRef | null {
  if (scope.kind === "collection")
    return {
      project_id: projectId,
      target: { kind: "workset", collection_id: scope.id },
    };
  if (scope.kind === "result")
    return {
      project_id: projectId,
      target: { kind: "query_result", result_id: scope.id },
    };
  return null;
}

export function useRankingBrowse(
  client: StudioClient,
  projectId: string,
  scope: BrowseScope,
  persisted: RankedBrowseSettings | null,
  onChange: (value: RankedBrowseSettings) => void,
) {
  const identity = JSON.stringify(browseScopeIdentity(scope));
  const target = rankableScope(projectId, scope);
  const info = useQuery({
    queryKey: ["project", projectId, "ranking-browse-info", identity],
    queryFn: ({ signal }) =>
      client.ranking.browseInfo(projectId, target!, signal),
    enabled: !!target,
    staleTime: 15000,
  });
  const settings: RankedBrowseSettings =
    persisted?.scopeKey === identity
      ? persisted
      : {
          scopeKey: identity,
          sort: "saved",
          descending: false,
          startPostId: null,
          startCursor: null,
        };
  const active = !!info.data?.ranking && settings.sort !== "off";
  const key = active
    ? [settings.sort, settings.descending, settings.startPostId]
    : null;
  const [draft, setDraft] = useState(settings.startPostId ?? "");
  const [error, setError] = useState("");
  useEffect(() => {
    setDraft(settings.startPostId ?? "");
    setError("");
  }, [identity, settings.startPostId]);
  function change(patch: Partial<RankedBrowseSettings>) {
    setError("");
    onChange({ ...settings, ...patch, startCursor: null });
  }
  function start(first: () => void) {
    const text = draft.trim().replace(/^0+(?=\d)/, "");
    if (
      !/^[1-9][0-9]{0,18}$/.test(text) ||
      BigInt(text) > 9223372036854775807n
    ) {
      setError("请填写有效的 Danbooru ID 正整数。");
      return;
    }
    setDraft(text);
    setError("");
    if (text === settings.startPostId) first();
    else change({ startPostId: text });
  }
  return {
    target,
    info: info.data?.ranking ?? null,
    loading: !!target && info.isPending,
    infoError: target ? info.error : null,
    settings,
    active,
    key,
    draft,
    setDraft,
    error,
    change,
    start,
    rememberStart: (cursor: string) => {
      if (settings.startPostId && settings.startCursor !== cursor)
        onChange({ ...settings, startCursor: cursor });
    },
  };
}
export type RankingBrowseState = ReturnType<typeof useRankingBrowse>;

export function RankingStart({
  state,
  busy,
  first,
}: {
  state: RankingBrowseState;
  busy: boolean;
  first: () => void;
}) {
  if (!state.active) return null;
  return (
    <div className="ranking-start-controls">
      <label title="同一个 ID 对应多张图片时，定位当前查看顺序中的第一个匹配成员">
        从图片定位
        <input
          aria-label="起点 Danbooru ID"
          value={state.draft}
          inputMode="numeric"
          placeholder="Danbooru ID"
          onChange={(event) => state.setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !busy) {
              event.preventDefault();
              state.start(first);
            }
          }}
        />
      </label>
      <button
        disabled={busy || !state.draft.trim()}
        onClick={() => state.start(first)}
      >
        从此图开始
      </button>
      {state.settings.startPostId ? (
        <>
          <span className="subtle">
            起点 #{state.settings.startPostId} · 包含这张图
          </span>
          <button
            disabled={busy}
            title="沿当前查看方向，从整张榜单开始"
            onClick={() => state.change({ startPostId: null })}
          >
            重置起点
          </button>
        </>
      ) : (
        <span className="subtle">按帖子 ID 定位图片，再沿当前排名方向浏览</span>
      )}
      {state.error && (
        <span className="error" role="alert">
          {state.error}
        </span>
      )}
    </div>
  );
}

const number = (value: number) => value.toLocaleString("zh-CN");
export function RankingBadge({
  ranking,
}: {
  ranking: AssetRanking | null | undefined;
}) {
  if (!ranking) return null;
  const rank = (value: number | null | undefined) =>
    value == null ? "未排名" : "#" + number(value);
  const score = (value: number | null | undefined) =>
    value != null && Number.isFinite(value) ? value.toFixed(4) : "未评分";
  return (
    <div
      className="asset-ranking"
      data-main-rank={ranking.main_rank ?? "none"}
      data-rescue-rank={ranking.rescue_rank ?? "none"}
      title={`评分时的 Danbooru ID：${ranking.post_id ?? "未知"}；名次在 ${ranking.rating?.toUpperCase() ?? "未知"} 分级内计算`}
    >
      <span className="ranking-rating">
        {ranking.rating?.toUpperCase() ?? "未知分级"}
      </span>
      <span>
        主 {rank(ranking.main_rank)} <b>{score(ranking.main_score)}</b>
      </span>
      <span>
        {ranking.v2 ? "年" : "补"} {rank(ranking.rescue_rank)}{" "}
        <b>{score(ranking.rescue_score)}</b>
      </span>
    </div>
  );
}
