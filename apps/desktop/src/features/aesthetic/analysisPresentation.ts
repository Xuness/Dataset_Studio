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

const estimators: Record<string, string> = {
  davidson_v2: "Davidson v2",
  davidson_v1: "Davidson v1",
  borda_v1: "Borda",
};
export function estimatorLabel(kind: string) {
  return estimators[kind] ?? kind;
}
/** Fit settings of a snapshot, or null for other job kinds. */
export function fitConfig(
  job: Schema["AestheticAnalysisJob"],
): Schema["AestheticFit"] | null {
  const spec = job.request.spec;
  return spec.kind === "fit" ? spec.config : null;
}
/** Short parameter line that tells snapshots of one stage apart. */
export function fitParameters(config: Schema["AestheticFit"]) {
  const { kind, regularization, tie_strength } = config.estimator;
  return kind === "borda_v1"
    ? estimatorLabel(kind)
    : `${estimatorLabel(kind)} · 正则 ${regularization} · 并列 ${tie_strength}`;
}
/** Full fit settings for a tooltip, one item per line. */
export function fitDescription(
  job: Schema["AestheticAnalysisJob"],
  stageName: string | undefined,
) {
  const config = fitConfig(job);
  if (!config) return job.request.name;
  const { kind, iterations, regularization, tie_strength } = config.estimator;
  return [
    job.request.name,
    `来源阶段：${stageName ?? job.input.stage_id}（${job.input.observations.toLocaleString()} 批证据）`,
    `估计器：${estimatorLabel(kind)}`,
    ...(kind === "borda_v1"
      ? []
      : [
          `迭代上限：${iterations}`,
          `正则化：${regularization}`,
          `并列强度：${tie_strength}`,
        ]),
    `整批分半诊断：${config.stability_seed == null ? "关闭" : "开启"}`,
    `候选：${job.input.candidates.toLocaleString()} 张`,
  ].join("\n");
}

export function aestheticTime(value: string | null | undefined): string {
  if (!value) return "—";
  const time = new Date(/^\d+$/.test(value) ? Number(value) : value);
  return Number.isFinite(time.getTime()) ? time.toLocaleString() : "时间未知";
}
