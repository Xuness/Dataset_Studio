import type {
  Collection,
  QueryResult,
  QueryDefinition,
  ScopeRef,
  Selection,
  Source,
} from "@studio/contracts";

export type ScopeOption = {
  value: string;
  label: string;
  scope: ScopeRef;
  count: number | null;
};
export function scopeOptions(
  projectId: string,
  selection: Selection | undefined,
  sources: Source[],
  collections: Collection[],
  results: QueryResult[],
  definitions: QueryDefinition[] = [],
): ScopeOption[] {
  const options: ScopeOption[] = [];
  if (selection)
    options.push({
      value: "selection",
      label: "项目当前选择",
      count: selection.count,
      scope: {
        project_id: projectId,
        target: { kind: "selection", revision: selection.revision },
      },
    });
  for (const source of sources)
    if (source.available && source.revision)
      options.push({
        value: source.id,
        label: "数据湖 · " + source.name,
        count: source.count ?? null,
        scope: {
          project_id: projectId,
          target: {
            kind: "source",
            source_id: source.id,
            revision: source.revision,
          },
        },
      });
  for (const collection of collections)
    options.push({
      value: collection.id,
      label: "工作集 · " + collection.name,
      count: collection.count,
      scope: {
        project_id: projectId,
        target: { kind: "workset", collection_id: collection.id },
      },
    });
  for (const result of results)
    if (result.state === "ready")
      options.push({
        value: result.id,
        label:
          (result.cache.mode === "view" ? "浏览视图 · " : "查询结果 · ") +
          resultLabel(
            result,
            definitions.find((d) => d.id === result.definition_id)?.name,
          ),
        count: result.count ?? null,
        scope: {
          project_id: projectId,
          target: { kind: "query_result", result_id: result.id },
        },
      });
  return options;
}
export function resultLabel(result: QueryResult, name?: string) {
  return (
    (name ?? (result.definition_id ? "已保存查询" : "数据湖范围")) +
    (result.definition_revision
      ? " · 条件版本 " + result.definition_revision
      : "")
  );
}
export function scopeKindLabel(scope: ScopeRef | null | undefined): string {
  if (!scope) return "固定成员";
  return {
    source: "数据湖",
    workset: "工作集",
    query_result: "查询结果",
    selection: "项目选择",
  }[scope.target.kind];
}
