import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Play, RotateCw, X } from "lucide-react";
import { Button, useDraft, DraftStatus } from "@studio/ui";
import type { StudioClient } from "@studio/client";
import type {
  QueryDefinition,
  QueryResult,
  QuerySpec,
  ScopeOperation,
  Source,
} from "@studio/contracts";
import { QueryConditions } from "./QueryConditions.js";
import type { useProjectQueries } from "./useProjectQueries.js";
import "./query.css";
import { resultLabel } from "../scopes/scopes.js";

const stateNames: Record<QueryResult["state"], string> = {
  queued: "等待构建",
  running: "构建中",
  ready: "已完成",
  cancelled: "已取消",
  failed: "失败",
  interrupted: "需要重新计算",
  released: "成员已释放",
};
type Model = ReturnType<typeof useProjectQueries>;
type QueryDraft = {
  definition: QueryDefinition | null;
  name: string;
  sourceId: string;
  conditions: QuerySpec["conditions"];
  rule: QuerySpec["observation_rule"];
  order: QuerySpec["order"];
};
const initialDraft: QueryDraft = {
  definition: null,
  name: "资料筛选",
  sourceId: "",
  conditions: [],
  rule: "current_post",
  order: "asset_key_asc",
};
function decodeDraft(value: unknown): QueryDraft | null {
  if (typeof value !== "object" || value === null) return null;
  const v = value as Record<string, unknown>;
  if (
    typeof v.name !== "string" ||
    typeof v.sourceId !== "string" ||
    !Array.isArray(v.conditions) ||
    v.conditions.length > 12 ||
    !["current_post", "any_observation"].includes(String(v.rule)) ||
    !["asset_key_asc", "asset_key_desc"].includes(String(v.order))
  )
    return null;
  if (
    v.definition !== null &&
    (typeof v.definition !== "object" ||
      !v.definition ||
      !("id" in v.definition) ||
      !("spec" in v.definition) ||
      !("revision" in v.definition))
  )
    return null;
  if (
    v.conditions.some(
      (c) =>
        !c ||
        typeof c !== "object" ||
        typeof c.field !== "string" ||
        ![
          "eq",
          "ne",
          "gte",
          "lte",
          "has_tag",
          "is_missing",
          "is_present",
        ].includes(c.operator),
    )
  )
    return null;
  return value as QueryDraft;
}
export function QueryPanel({
  client,
  projectId,
  sources,
  model,
  onResult,
  onSelect,
  onClose,
}: {
  client: StudioClient;
  projectId: string;
  sources: Source[];
  model: Model;
  onResult: (result: QueryResult, name: string) => void;
  onSelect: (result: QueryResult, operation: ScopeOperation) => void;
  onClose: () => void;
}) {
  const cache = useQueryClient();
  const draft = useDraft(
    client,
    projectId,
    "core.query",
    initialDraft,
    decodeDraft,
  );
  const { definition, name, sourceId, conditions, rule, order } = draft.value;
  const setDefinition = (definition: QueryDefinition | null) =>
    draft.controller.set((v) => ({ ...v, definition }));
  const setName = (name: string) =>
    draft.controller.set((v) => ({ ...v, name }));
  const setSourceId = (sourceId: string) =>
    draft.controller.set((v) => ({ ...v, sourceId }));
  const setConditions = (conditions: QuerySpec["conditions"]) =>
    draft.controller.set((v) => ({ ...v, conditions }));
  const setRule = (rule: QuerySpec["observation_rule"]) =>
    draft.controller.set((v) => ({ ...v, rule }));
  const setOrder = (order: QuerySpec["order"]) =>
    draft.controller.set((v) => ({ ...v, order }));
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const multiSource = !!definition && definition.spec.source_ids.length > 1;
  const listedDefinitions = model.definitions.data?.items ?? [];
  const savedDefinitions =
    definition && !listedDefinitions.some((item) => item.id === definition.id)
      ? [definition, ...listedDefinitions]
      : listedDefinitions;
  useEffect(() => {
    if (draft.editable && !sourceId && sources[0])
      draft.controller.set((v) => ({ ...v, sourceId: sources[0]!.id }));
  }, [draft.controller, draft.editable, sourceId, sources]);
  const directory = useQuery({
    queryKey: ["project", projectId, "fields", sourceId],
    queryFn: ({ signal }) => client.queries.fields(projectId, sourceId, signal),
    enabled: !!sourceId,
  });
  const fields =
    directory.data?.fields.filter((f) => f.operators.length > 0) ?? [];
  async function action(fn: () => Promise<unknown>) {
    setPending(true);
    setError("");
    try {
      await fn();
      await cache.invalidateQueries({
        queryKey: ["project", projectId, "query-results"],
      });
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setPending(false);
    }
  }
  function load(query: QueryDefinition) {
    setDefinition(query);
    setName(query.name);
    setSourceId(query.spec.source_ids[0] ?? "");
    setConditions(query.spec.conditions);
    setRule(query.spec.observation_rule);
    setOrder(query.spec.order);
    setError("");
  }
  async function run() {
    if (multiSource && definition) {
      await client.queries.build(projectId, definition.id, definition.revision);
      model.firstPage();
      return;
    }
    const saved = await client.queries.save(
      projectId,
      {
        name,
        spec: {
          version: 1,
          source_ids: [sourceId],
          conditions,
          observation_rule: rule,
          order,
        },
        ...(definition ? { expected_revision: definition.revision } : {}),
      },
      definition?.id,
    );
    setDefinition(saved);
    await client.queries.build(projectId, saved.id, saved.revision);
    await cache.invalidateQueries({
      queryKey: ["project", projectId, "queries"],
    });
    model.firstPage();
    model.firstDefinitions();
  }
  return (
    <section className="query-panel" aria-label="项目查询">
      <header>
        <strong>查询</strong>
        <span className="subtle">全部条件联合判定</span>
        <span className="grow" />
        <button className="icon-button" aria-label="收起查询" onClick={onClose}>
          <X size={14} />
        </button>
      </header>
      <DraftStatus controller={draft.controller} />
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void action(run);
        }}
      >
        <fieldset
          className="draft-fields"
          disabled={!draft.editable || pending}
        >
          <div className="query-definition-row">
            <select
              aria-label="已保存查询"
              value={definition?.id ?? ""}
              onChange={(event) => {
                const query = savedDefinitions.find(
                  (q) => q.id === event.target.value,
                );
                if (query) load(query);
                else {
                  setDefinition(null);
                  setName("资料筛选");
                  setConditions([]);
                }
              }}
            >
              <option value="">新建查询</option>
              {savedDefinitions.map((q) => (
                <option key={q.id} value={q.id}>
                  {q.name}
                </option>
              ))}
            </select>
            <input
              aria-label="查询名称"
              disabled={multiSource}
              value={name}
              onChange={(event) => setName(event.target.value)}
              maxLength={120}
              required
            />
            <select
              aria-label="查询数据湖"
              disabled={multiSource}
              value={sourceId}
              onChange={(event) => {
                setSourceId(event.target.value);
                setConditions([]);
              }}
              required
            >
              <option value="" disabled>
                选择数据湖
              </option>
              {sources.map((s) => (
                <option key={s.id} value={s.id} disabled={!s.available}>
                  {s.name}
                  {s.available ? "" : "（不可用）"}
                </option>
              ))}
            </select>
          </div>
          {(model.definitionCursor || model.definitions.data?.next_cursor) && (
            <div className="query-saved-pages">
              <button
                type="button"
                disabled={!model.definitionCursor}
                onClick={model.firstDefinitions}
              >
                最近查询
              </button>
              <button
                type="button"
                disabled={!model.definitions.data?.next_cursor}
                onClick={model.nextDefinitions}
              >
                更多已保存查询
              </button>
            </div>
          )}
          {multiSource ? (
            <p className="query-rule-hint">
              此定义包含多个数据湖，可按保存的条件重新计算。单湖查询可在此编辑。
            </p>
          ) : (
            <QueryConditions
              fields={fields}
              conditions={conditions}
              onChange={setConditions}
            />
          )}
          <div className="query-run-row">
            <select
              aria-label="观察判定规则"
              disabled={multiSource}
              value={rule}
              onChange={(event) =>
                setRule(event.target.value as QuerySpec["observation_rule"])
              }
            >
              <option value="current_post">当前帖子观察</option>
              <option value="any_observation">任一关联观察</option>
            </select>
            <select
              aria-label="查询排序"
              disabled={multiSource}
              value={order}
              onChange={(event) =>
                setOrder(event.target.value as QuerySpec["order"])
              }
            >
              <option value="asset_key_asc">内容身份升序</option>
              <option value="asset_key_desc">内容身份降序</option>
            </select>
            <span className="grow" />
            <Button
              type="submit"
              className="primary"
              disabled={
                pending || !sourceId || directory.isPending || !!directory.error
              }
            >
              <Play size={12} />
              {pending
                ? "提交中…"
                : multiSource
                  ? "按已保存定义查询"
                  : "保存并查询"}
            </Button>
          </div>
          <p className="query-rule-hint">
            {rule === "current_post"
              ? "元数据条件只检查当前帖子关联的观察和存储对象。"
              : "每张图片须有同一条关联观察满足全部元数据条件。"}
            仅查询存储字段时包含无元数据的对象。
          </p>
        </fieldset>
      </form>
      {(error ||
        directory.error ||
        model.results.error ||
        model.definitions.error) && (
        <p className="query-error" role="alert">
          {error ||
            directory.error?.message ||
            model.results.error?.message ||
            model.definitions.error?.message}
        </p>
      )}
      <div className="query-result-list">
        {!model.results.data?.items.length && (
          <p className="query-rule-hint">
            结果保存在项目中，构建期间可以继续浏览或使用其他工具。
          </p>
        )}
        {model.results.data?.items.map((result) => {
          const queryName =
            savedDefinitions.find((q) => q.id === result.definition_id)?.name ??
            (result.definition_id ? "已保存查询" : "数据湖范围");
          const building =
            result.state === "queued" || result.state === "running";
          return (
            <div className="query-result-row" key={result.id}>
              <div>
                <strong>{queryName}</strong>
                <small>
                  {stateNames[result.state]} ·{" "}
                  {result.count == null
                    ? "数量未确定"
                    : result.count.toLocaleString() + " 项"}
                  {building && result.processed > 0
                    ? " · 已处理 " +
                      result.processed.toLocaleString() +
                      " 条记录"
                    : ""}
                  <span className="result-id" title={result.id}>
                    {" "}
                    ·{" "}
                    {result.definition_revision
                      ? "条件版本 " + result.definition_revision + " · "
                      : ""}
                    {new Date(Number(result.created_at)).toLocaleTimeString(
                      [],
                      { hour: "2-digit", minute: "2-digit" },
                    )}
                  </span>
                </small>
                {result.error && (
                  <small className="query-error">{result.error}</small>
                )}
              </div>
              {result.state === "ready" && (
                <>
                  <Button
                    disabled={pending}
                    onClick={() =>
                      onResult(result, resultLabel(result, queryName))
                    }
                  >
                    查看
                  </Button>
                  <Button
                    disabled={pending}
                    onClick={() => onSelect(result, "replace")}
                  >
                    全选结果
                  </Button>
                </>
              )}
              {building ? (
                <Button
                  disabled={pending}
                  onClick={() =>
                    void action(() =>
                      client.queries.cancel(projectId, result.id),
                    )
                  }
                >
                  取消
                </Button>
              ) : (
                <>
                  {result.definition_id && (
                    <button
                      className="icon-button"
                      disabled={pending}
                      title="按已保存定义重新计算"
                      onClick={() =>
                        void action(async () => {
                          const query = await client.queries.definition(
                            projectId,
                            result.definition_id!,
                          );
                          await client.queries.build(
                            projectId,
                            query.id,
                            query.revision,
                          );
                          model.firstPage();
                        })
                      }
                    >
                      <RotateCw size={13} />
                    </button>
                  )}
                  {result.state !== "released" && (
                    <button
                      disabled={pending}
                      className="release-result"
                      onClick={() =>
                        void action(() =>
                          client.queries.release(projectId, result.id),
                        )
                      }
                    >
                      释放
                    </button>
                  )}
                </>
              )}
            </div>
          );
        })}
      </div>
      {(model.cursor || model.results.data?.next_cursor) && (
        <div className="query-history-page">
          <Button disabled={!model.cursor} onClick={model.firstPage}>
            第一页
          </Button>
          <Button
            disabled={!model.results.data?.next_cursor}
            onClick={model.nextPage}
          >
            更多结果
          </Button>
        </div>
      )}
    </section>
  );
}
