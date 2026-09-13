import { isV2Parameters, v2Issue } from "./v2.js";
import type { V2Parameters } from "./v2.js";
import type {
  RankingParameters,
  RankingFilter,
  ScopeRef,
  RankingEligibility,
  RankingRoute,
} from "@studio/contracts";
export const operatorId = "danbooru.metarecall";
export const defaults: RankingParameters = {
  ratings: ["e", "g", "q", "s"],
  mode: "rank",
  quotas: [280, 43, 10],
  seed: "metarecall-v1",
  minimum_stored_side: null,
  exclude_banned: false,
  time_enabled: true,
  artist_enabled: false,
  votes_enabled: true,
  damage_enabled: true,
  cohort_minimum: 5000,
  time_weight: 0.3,
  artist_weight: 0.25,
  vote_weight: 0.08,
  damage_weight: 0.04,
};
export const defaultFilter: RankingFilter = {
  rating: "g",
  route: null,
  eligibility: "eligible",
  missing_only: false,
  selected_only: false,
  top: null,
  order: "main",
};
export type RankingDraft = {
  v2Saved?: V2Parameters | null;
  parameters: RankingParameters;
  scope: ScopeRef | null;
  scopeId: string;
  tab: "config" | "results" | "diagnostics";
  lastJob: string | null;
  artifactId: string;
  filter: RankingFilter;
  filterArtifact: string;
  submission: { key: string; signature: string } | null;
  worksetName: string;
  worksetSubmission: { key: string; signature: string } | null;
};
export const initial: RankingDraft = {
  parameters: defaults,
  scope: null,
  scopeId: "",
  tab: "config",
  lastJob: null,
  artifactId: "",
  filter: defaultFilter,
  filterArtifact: "",
  submission: null,
  worksetName: "元数据排名候选",
  worksetSubmission: null,
};
const record = (v: unknown): v is Record<string, unknown> =>
  !!v && typeof v === "object" && !Array.isArray(v);
export function decode(value: unknown): RankingDraft | null {
  if (!record(value) || !record(value.parameters) || !record(value.filter))
    return null;
  const p = value.parameters;
  if (p.v2 != null && !isV2Parameters(p.v2)) return null;
  if (value.v2Saved != null && !isV2Parameters(value.v2Saved)) return null;
  if (
    !Array.isArray(p.ratings) ||
    p.ratings.some((r) => !["g", "s", "q", "e"].includes(String(r))) ||
    !Array.isArray(p.quotas) ||
    p.quotas.length !== 3 ||
    p.quotas.some((n) => typeof n !== "number" || !Number.isFinite(n)) ||
    !["rank", "select"].includes(String(p.mode)) ||
    typeof p.seed !== "string" ||
    [
      "time_weight",
      "artist_weight",
      "vote_weight",
      "damage_weight",
      "cohort_minimum",
    ].some((k) => typeof p[k] !== "number" || !Number.isFinite(p[k])) ||
    [
      "time_enabled",
      "artist_enabled",
      "votes_enabled",
      "damage_enabled",
      "exclude_banned",
    ].some((k) => typeof p[k] !== "boolean") ||
    (p.minimum_stored_side !== null &&
      typeof p.minimum_stored_side !== "number") ||
    typeof value.scopeId !== "string" ||
    typeof value.artifactId !== "string" ||
    typeof value.filterArtifact !== "string" ||
    typeof value.worksetName !== "string" ||
    (value.lastJob !== null && typeof value.lastJob !== "string") ||
    !["config", "results", "diagnostics"].includes(String(value.tab)) ||
    (value.scope !== null &&
      (!record(value.scope) ||
        typeof value.scope.project_id !== "string" ||
        !record(value.scope.target)))
  )
    return null;
  return value as unknown as RankingDraft;
}
export function parameterIssue(p: RankingParameters): string | null {
  if (p.v2) {
    const issue = v2Issue(p.v2);
    if (issue) return issue;
  }
  if (!p.ratings.length) return "至少选择一个分级。";
  if (
    p.quotas.some((v) => !Number.isInteger(v) || v < 0 || v > 1000) ||
    p.quotas.reduce((a, b) => a + b, 0) > 1000
  )
    return "通道比例合计不能超过 100%，精度为 0.1%。";
  if (!p.seed || new TextEncoder().encode(p.seed).length > 128)
    return "抽样种子需要 1–128 字节。";
  if (
    !Number.isInteger(p.cohort_minimum) ||
    p.cohort_minimum < 1 ||
    p.cohort_minimum > 10000000
  )
    return "邻域最低数量需要是 1–10,000,000 的整数。";
  if (
    p.minimum_stored_side != null &&
    (!Number.isInteger(p.minimum_stored_side) ||
      p.minimum_stored_side < 1 ||
      p.minimum_stored_side > 65535)
  )
    return "成品最短边需要是 1–65,535 的整数。";
  if (
    [p.time_weight, p.artist_weight, p.vote_weight, p.damage_weight].some(
      (v) => !Number.isFinite(v) || v < 0 || v > 1,
    )
  )
    return "评分系数需要在 0–1 之间。";
  return null;
}
export const eligibilityNames: Record<RankingEligibility, string> = {
  eligible: "合格候选",
  metadata_unavailable: "无符合范围的元数据",
  rating_unknown: "分级未知",
  rating_excluded: "分级不在本次范围",
  dimensions_unknown: "成品尺寸待核验",
  dimensions_excluded: "成品尺寸不符合",
  policy_excluded: "用途策略排除",
  duplicate: "重复图片",
};
export const routeNames: Record<RankingRoute, string> = {
  ineligible: "未参与排名",
  ranked: "仅排名",
  main: "主通道",
  rescue: "补救通道",
  audit: "随机审计",
  budget_rejected: "未入选",
};
export const flagNames: Record<string, string> = {
  fav_unknown_or_invalid: "收藏数未知或无效",
  up_unknown_or_invalid: "赞票未知或无效",
  down_unknown_or_invalid: "负票未知或无效",
  created_time_unknown: "创建时间未知",
  observation_time_unknown: "观察时间未知",
  observation_date_only: "日期级观察",
  observation_time_invalid: "观察时间无效",
  artist_unknown: "画师信息未知",
  score_inconsistent: "净分与票数不一致",
  stored_dimensions_unknown: "成品尺寸未知",
  tags_unknown: "标签未知",
  rating_conflict: "存在分级冲突",
  source_issues: "来源字段提示",
  cohort_insufficient: "比较群体不足",
};
export const number = (n: number | null | undefined) =>
  n == null ? "待计算" : n.toLocaleString("zh-CN");
export const score = (n: number | null | undefined) =>
  n == null ? "—" : n.toFixed(2);
export function integer(value: string | null | undefined) {
  if (value == null) return "未知";
  try {
    return BigInt(value).toLocaleString("zh-CN");
  } catch {
    return value;
  }
}
export function time(value: string | null | undefined, dateOnly = false) {
  if (value == null) return "未知";
  const date = new Date(Number(value) / 1000);
  if (!Number.isFinite(date.getTime())) return "时间超出显示范围";
  return dateOnly
    ? date.toISOString().slice(0, 10) + "（UTC 日期）"
    : date.toLocaleString("zh-CN");
}
