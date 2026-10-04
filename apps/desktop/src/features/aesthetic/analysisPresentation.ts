import type { Schema, Asset } from "@studio/contracts";
export type RankingRow = Schema["AestheticRankingRow"];
export function comparisonDeltaLabel(
  row: Schema["AestheticComparisonRow"],
): string {
  if (
    !row.comparable ||
    row.percentile_delta == null ||
    row.left_percentile == null ||
    row.right_percentile == null
  )
    return row.reason ?? "不可比较";
  // The contract's percentile_delta is absolute. Direction comes from A and B.
  const delta = (row.right_percentile - row.left_percentile) * 100;
  return Math.abs(delta) < 0.005
    ? "无变化"
    : `${delta < 0 ? "上移" : "下移"} ${Math.abs(delta).toFixed(2)} 个百分点`;
}
export function rankingLabel(row: RankingRow): string {
  const min = row.rating_rank_min ?? row.rank_min;
  const max = row.rating_rank_max ?? row.rank_max;
  if (min == null) return "暂无可比名次";
  const value = max != null && min !== max ? `${min}–${max}` : String(min);
  return row.rating_rank_min != null
    ? `${row.rating.toUpperCase()} · 第 ${value} 名`
    : `分量 ${row.component ?? "?"} · 第 ${value} 名`;
}
export function rankingAsset(row: RankingRow): Asset {
  return {
    key: row.key,
    name: `候选 ${row.ordinal + 1}`,
    bytes: "0",
    extension: "",
    source_name: "",
    selected: false,
    summary: null,
  };
}
export function analysisState(value: string) {
  return (
    (
      {
        queued: "排队中",
        running: "计算中",
        completed: "已完成",
        cancelled: "已取消",
        interrupted: "已中断",
        failed: "失败",
        cancelling: "取消中",
      } as Record<string, string>
    )[value] ?? value
  );
}
export function analysisActive(value: string) {
  return !["completed", "cancelled", "interrupted", "failed"].includes(value);
}

export function aestheticTime(value: string | null | undefined): string {
  if (!value) return "—";
  const time = new Date(/^\d+$/.test(value) ? Number(value) : value);
  return Number.isFinite(time.getTime()) ? time.toLocaleString() : "时间未知";
}
