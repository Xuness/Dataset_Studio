import type { QuerySpec, QueryResult } from "@studio/contracts";

export function queryCacheLabel(result: QueryResult) {
  const mode = result.cache.mode;
  if (["queued", "running"].includes(result.state)) {
    return (
      (
        {
          index: "正在更新排序索引…",
          publishing: "正在保存查询结果…",
          incremental: "正在检查来源变更…",
          refresh: "等待增量刷新…",
          rebuilt: "变更记录不完整，正在重新检查…",
        } as Record<string, string>
      )[mode] ?? "正在查询…"
    );
  }
  if (result.state !== "ready") return "";
  if (mode === "reused") return "已复用现有结果，无需重新扫描";
  if (mode === "incremental")
    return `已增量刷新 · 检查 ${result.cache.evaluated_objects.toLocaleString()} 个图像 · 变更 ${result.cache.changed_members.toLocaleString()} 项`;
  if (mode === "rebuilt")
    return "来源记录不足以增量更新，已重新计算并复用未变化的成员";
  return "查询已完成，结果可复用";
}

/** Compare query meaning without treating clause/set order as an edit. */
export function querySignature(spec: QuerySpec) {
  const scope = spec.input_scope;
  const target = scope?.target;
  const input =
    !scope || !target
      ? null
      : [
          scope.project_id,
          target.kind,
          target.kind === "source"
            ? [target.source_id, target.revision]
            : target.kind === "selection"
              ? target.revision
              : target.kind === "workset"
                ? target.collection_id
                : target.result_id,
        ];
  return JSON.stringify({
    sources: [...new Set(spec.source_ids)].sort(),
    conditions: [
      ...new Set(
        spec.conditions.map((c) =>
          JSON.stringify({
            field: c.field,
            operator: c.operator,
            value:
              c.value?.type === "text_list"
                ? {
                    type: "text_list",
                    value: [...new Set(c.value.value)].sort(),
                  }
                : (c.value ?? null),
          }),
        ),
      ),
    ].sort(),
    rule: spec.observation_rule,
    order: spec.order,
    input,
  });
}
