import type { Schema, RankingParameters } from "@studio/contracts";
export type V2Parameters = Schema["RankingV2Parameters"];
export const v2OperatorId = "danbooru.metarecall_v2";
export const operatorFor = (p: RankingParameters) =>
  p.v2 ? v2OperatorId : "danbooru.metarecall";
export const isRankingOperator = (id: unknown) =>
  id === "danbooru.metarecall" || id === v2OperatorId;
export function v2Defaults(): V2Parameters {
  return {
    profiles: Object.fromEntries(
      ["g", "s", "q", "e"].map((r) => [
        r,
        { time_up: 0.3, time_down: 0, vote_weight: 0.08, era_weight: 0.3 },
      ]),
    ),
    feather_days: 90,
    minimum_effective: 5000,
    comic_penalty: 0,
    keep_per_mille: 333,
    direct_rescue: 50,
    era_rescue: 50,
    audit: 30,
    eras: [],
    strict_era_targets: false,
  };
}
const record = (v: unknown): v is Record<string, unknown> =>
  !!v && typeof v === "object" && !Array.isArray(v);
export function isV2Parameters(v: unknown): v is V2Parameters {
  if (
    !record(v) ||
    !record(v.profiles) ||
    !Array.isArray(v.eras) ||
    typeof v.strict_era_targets !== "boolean"
  )
    return false;
  const profiles = v.profiles;
  if (
    [
      "feather_days",
      "minimum_effective",
      "comic_penalty",
      "keep_per_mille",
      "direct_rescue",
      "era_rescue",
      "audit",
    ].some((k) => typeof v[k] !== "number" || !Number.isFinite(v[k]))
  )
    return false;
  if (
    ["g", "s", "q", "e"].some(
      (r) =>
        !record(profiles[r]) ||
        ["time_up", "time_down", "vote_weight", "era_weight"].some(
          (k) =>
            typeof (profiles[r] as Record<string, unknown>)[k] !== "number",
        ),
    )
  )
    return false;
  return v.eras.every(
    (e) =>
      record(e) &&
      ["from_year", "through_year", "bonus"].every(
        (k) => typeof e[k] === "number",
      ) &&
      (e.target_share == null || typeof e.target_share === "number"),
  );
}
export function v2Issue(p: V2Parameters): string | null {
  if (!isV2Parameters(p)) return "v2 参数格式不完整。";
  if (
    Object.keys(p.profiles).some((r) => !["g", "s", "q", "e"].includes(r)) ||
    Object.values(p.profiles).some((v) =>
      [v.time_up, v.time_down, v.vote_weight, v.era_weight].some(
        (x) => !Number.isFinite(x) || x < 0 || x > 1,
      ),
    )
  )
    return "各分级的 v2 权重需要在 0–1 之间。";
  if (
    !Number.isInteger(p.feather_days) ||
    p.feather_days < 0 ||
    p.feather_days > 180
  )
    return "羽化半宽需要是 0–180 天的整数。";
  if (
    !Number.isInteger(p.minimum_effective) ||
    p.minimum_effective < 1 ||
    p.minimum_effective > 10000000
  )
    return "有效样本下限需要是 1–10,000,000 的整数。";
  if (p.comic_penalty < 0 || p.comic_penalty > 10)
    return "漫画标签软降分需要在 0–10 分之间。";
  if (
    [p.keep_per_mille, p.direct_rescue, p.era_rescue, p.audit].some(
      (n) => !Number.isInteger(n) || n < 0 || n > 1000,
    ) ||
    p.direct_rescue + p.era_rescue + p.audit > 1000
  )
    return "保留比例和预算内补救、审计比例需要在 0–100% 之间。";
  if (p.eras.length > 64) return "最多配置 64 个年代区间。";
  const eras = [...p.eras].sort((a, b) => a.from_year - b.from_year);
  let end = 0,
    total = 0;
  for (const e of eras) {
    if (
      !Number.isInteger(e.from_year) ||
      !Number.isInteger(e.through_year) ||
      e.from_year < 1900 ||
      e.through_year > 2200 ||
      e.through_year < e.from_year ||
      e.from_year <= end
    )
      return "年代区间需要在 1900–2200 年之间，且不能重叠。";
    if (!Number.isFinite(e.bonus) || e.bonus < -20 || e.bonus > 20)
      return "年代偏好需要在 -20 至 20 分之间。";
    if (
      e.target_share != null &&
      (!Number.isInteger(e.target_share) ||
        e.target_share < 0 ||
        e.target_share > 1000)
    )
      return "年代目标占比需要在 0–100% 之间。";
    total += e.target_share ?? 0;
    end = e.through_year;
  }
  return total > 1000 ? "年代目标占比合计不能超过保留预算的 100%。" : null;
}
export function orderLabel(order: string | undefined, v2 = false): string {
  return (
    (
      {
        main: v2 ? "筛选优先级" : "主排名",
        rescue: v2 ? "年代相对排名" : "补救排名",
        direct: "直算排名",
        fused: "融合排名",
        input: "输入顺序",
      } as Record<string, string>
    )[order ?? "main"] ?? "主排名"
  );
}
