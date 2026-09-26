import { validSourceTag, displayTag } from "@studio/ui";
import { sourceSupports } from "@studio/client";
import { useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Play, RotateCw, X, Save } from "lucide-react";
import {
  Button,
  useDraft,
  DraftStatus,
  ErrorDetails,
  ResizeGrip,
  querySignature,
  queryCacheLabel,
  MoreMenu,
} from "@studio/ui";
import type { BrowseScope, ModuleScopeOption } from "@studio/ui";
import { StudioError } from "@studio/client";
import type { StudioClient, ObjectTarget } from "@studio/client";
import type {
  QueryDefinition,
  QueryResult,
  QuerySpec,
  ScopeOperation,
  Source,
  ScopeRef,
} from "@studio/contracts";
import {
  QueryConditions,
  conditionIssue,
  operatorNames,
} from "./QueryConditions.js";
import type { useProjectQueries } from "./useProjectQueries.js";
import { resultLabel } from "../scopes/scopes.js";
import { useQueryFields } from "./useQueryFields.js";
import "./query.css";
const stateNames: Record<QueryResult["state"], string> = {
  queued: "等待执行",
  running: "查询中",
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
  sourceIds: string[] | null;
  conditions: QuerySpec["conditions"];
  rule: QuerySpec["observation_rule"];
  order: QuerySpec["order"];
  inputScope: ScopeRef | null;
};
const initialDraft: QueryDraft = {
  definition: null,
  name: "资料筛选",
  sourceIds: null,
  conditions: [],
  rule: "current_post",
  order: "post_id_desc",
  inputScope: null,
};
function decodeDraft(value: unknown): QueryDraft | null {
  if (!value || typeof value !== "object") return null;
  const v = value as QueryDraft & { sourceId?: string };
  const sourceIds =
    v.sourceIds === undefined
      ? Array.isArray(v.definition?.spec?.source_ids) &&
        v.definition.spec.source_ids.length > 1
        ? v.definition.spec.source_ids
        : typeof v.sourceId === "string"
          ? v.sourceId
            ? [v.sourceId]
            : null
          : undefined
      : v.sourceIds;
  if (
    typeof v.name !== "string" ||
    (sourceIds !== null &&
      (!Array.isArray(sourceIds) ||
        sourceIds.length > 8 ||
        sourceIds.some((id) => typeof id !== "string" || !id))) ||
    !Array.isArray(v.conditions) ||
    v.conditions.length > 12 ||
    !["current_post", "any_observation"].includes(v.rule) ||
    ![
      "asset_key_asc",
      "asset_key_desc",
      "post_id_asc",
      "post_id_desc",
    ].includes(v.order)
  )
    return null;
  if (
    v.definition !== null &&
    (!v.definition ||
      typeof v.definition.id !== "string" ||
      typeof v.definition.revision !== "number" ||
      !v.definition.spec ||
      !Array.isArray(v.definition.spec.source_ids))
  )
    return null;
  if (
    v.conditions.some(
      (c) =>
        !c ||
        typeof c.field !== "string" ||
        !Object.hasOwn(operatorNames, c.operator),
    )
  )
    return null;
  return {
    ...v,
    sourceIds: sourceIds === null ? null : [...new Set(sourceIds)],
    inputScope: v.inputScope ?? v.definition?.spec.input_scope ?? null,
  };
}
function scopeLabel(scope: ScopeRef | null, options: ModuleScopeOption[]) {
  if (!scope || scope.target.kind === "source") return "整个数据湖";
  return (
    options.find(
      (o) => JSON.stringify(o.scope.target) === JSON.stringify(scope.target),
    )?.label ??
    {
      selection: "已保存的选择范围",
      workset: "已保存的工作集",
      query_result: "已保存的查询结果",
    }[scope.target.kind]
  );
}
function conditionText(result: QueryResult) {
  return result.spec.conditions
    .map(
      (c) =>
        c.field +
        " " +
        (operatorNames[c.operator] ?? c.operator) +
        " " +
        (c.value?.type === "text_list"
          ? c.value.value.join("、")
          : String(c.value?.value ?? "")),
    )
    .join(" · ");
}
export function QueryPanel({
  client,
  projectId,
  sources,
  model,
  onResult,
  onSelect,
  onClose,
  browserScope,
  inputOptions,
  height,
  onHeight,
  onManage,
  invocation,
}: {
  client: StudioClient;
  projectId: string;
  sources: Source[];
  model: Model;
  onResult: (result: QueryResult, name: string) => void;
  onSelect: (result: QueryResult, operation: ScopeOperation) => void;
  onClose: () => void;
  browserScope: BrowseScope;
  inputOptions: ModuleScopeOption[];
  height: number;
  onHeight: (height: number) => void;
  onManage?: (
    target: ObjectTarget,
    mode?: "details" | "rename" | "remove",
  ) => void;
  invocation?: { sequence: number; args: Record<string, string> } | null;
}) {
  const cache = useQueryClient();
  const draft = useDraft(
    client,
    projectId,
    "core.query",
    initialDraft,
    decodeDraft,
  );
  const { definition, name, conditions, rule, order, inputScope } = draft.value;
  const sourceIds = draft.value.sourceIds ?? [];
  const update = (patch: Partial<QueryDraft>) =>
    draft.controller.set((v) => ({ ...v, ...patch }));
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [notice, setNotice] = useState("");
  const currentDefinition = useQuery({
    queryKey: ["project", projectId, "query-definition", definition?.id ?? ""],
    queryFn: () => client.queries.definition(projectId, definition!.id),
    enabled: !!definition && draft.editable,
    retry: false,
  });
  useEffect(() => {
    if (
      definition &&
      currentDefinition.error instanceof StudioError &&
      currentDefinition.error.code === "OBJECT_REMOVED"
    ) {
      draft.controller.set((old) => ({ ...old, definition: null }));
      setNotice("原保存查询已删除，当前条件草稿仍保留，可另存为新查询。");
    }
  }, [currentDefinition.error, definition, draft.controller]);
  const appliedInvocation = useRef<number | null>(null);
  const requestedId = invocation?.args.queryId ?? "";
  const requestedDefinition = useQuery({
    queryKey: ["project", projectId, "query-definition", requestedId],
    queryFn: () => client.queries.definition(projectId, requestedId),
    enabled: !!requestedId && draft.editable,
    retry: false,
  });
  useEffect(() => {
    if (
      !draft.editable ||
      !invocation ||
      appliedInvocation.current === invocation.sequence ||
      !requestedDefinition.data
    )
      return;
    load(requestedDefinition.data);
    appliedInvocation.current = invocation.sequence;
  }, [draft.editable, invocation, requestedDefinition.data]);
  useEffect(() => {
    if (
      !draft.editable ||
      !invocation ||
      appliedInvocation.current === invocation.sequence ||
      !invocation.args.exactTag
    )
      return;
    try {
      const tag: unknown = JSON.parse(invocation.args.exactTag);
      if (
        typeof tag !== "string" ||
        !validSourceTag(tag) ||
        !sources.some(
          (s) =>
            s.id === invocation.args.sourceId && sourceSupports(s, "query"),
        )
      )
        return;
      draft.controller.set((old) => ({
        ...old,
        definition: null,
        sourceIds: [invocation.args.sourceId!],
        name: "标签 · " + displayTag(tag),
        conditions: [
          {
            field: "tags",
            operator: "has_tag",
            value: { type: "text", value: tag },
          },
        ],
        rule: "current_post",
        inputScope: null,
      }));
      appliedInvocation.current = invocation.sequence;
    } catch {
      /* Only structured literal tags can populate this draft. */
    }
  }, [draft.editable, draft.controller, invocation, sources]);
  const [showReleased, setShowReleased] = useState(false);
  const [lastRun, setLastRun] = useState<string | null>(null);
  const submittedScope = useRef<string | null>(null);
  const listed = model.definitions.data?.items ?? [];
  const savedDefinitions =
    definition && !listed.some((q) => q.id === definition.id)
      ? [definition, ...listed]
      : listed;
  useEffect(() => {
    if (draft.editable && draft.value.sourceIds === null) {
      const preferred = sources
        .filter(
          (s) =>
            sourceSupports(s, "query") &&
            s.available &&
            (browserScope.kind !== "source" || s.id === browserScope.id),
        )
        .map((s) => s.id);
      if (preferred.length && preferred.length <= 8)
        draft.controller.set((v) => ({ ...v, sourceIds: preferred }));
    }
  }, [
    draft.editable,
    draft.controller,
    draft.value.sourceIds,
    sources,
    browserScope,
  ]);
  const directory = useQueryFields(client, projectId, sources, sourceIds);
  const fields = directory.fields;
  const invalid =
    !directory.orders.includes(order) ||
    !directory.observation_rules.includes(rule) ||
    conditions.length > directory.max_conditions ||
    conditions.some(
      (c) =>
        !!conditionIssue(
          c,
          fields.find((f) => f.id === c.field),
        ),
    );
  const scopeOptions = inputOptions.filter(
    (o) => o.scope.target.kind !== "source",
  );
  const selectedOption = scopeOptions.find(
    (o) =>
      inputScope &&
      o.scope.target.kind === inputScope.target.kind &&
      (o.scope.target.kind === "selection" ||
        JSON.stringify(o.scope.target) === JSON.stringify(inputScope.target)),
  );
  const staleInput =
    !!inputScope &&
    selectedOption &&
    JSON.stringify(selectedOption.scope) !== JSON.stringify(inputScope);
  const running = model.results.data?.items.find((r) => r.id === lastRun);
  const building = !!running && ["queued", "running"].includes(running.state);
  const spec = (): QuerySpec => ({
    version: 3,
    source_ids: sourceIds,
    conditions,
    observation_rule: rule,
    order,
    ...(inputScope ? { input_scope: inputScope } : {}),
  });
  async function action(fn: () => Promise<unknown>) {
    setPending(true);
    setError(null);
    setNotice("");
    try {
      await fn();
      await cache.invalidateQueries({
        queryKey: ["project", projectId, "query-results"],
      });
    } catch (e) {
      setError(e);
    } finally {
      setPending(false);
    }
  }
  function load(query: QueryDefinition) {
    update({
      definition: query,
      name: query.name,
      sourceIds: query.spec.source_ids,
      conditions: query.spec.conditions,
      rule: query.spec.observation_rule,
      order: query.spec.order,
      inputScope: query.spec.input_scope ?? null,
    });
    setError(null);
    setNotice("");
  }
  function fresh() {
    const selected = browserScope.kind === "source" ? [browserScope.id] : null;
    draft.controller.set({ ...initialDraft, sourceIds: selected });
    setError(null);
    setNotice("");
  }
  async function save() {
    if (!sourceIds.length || directory.pending || directory.error || invalid)
      return;
    const saved = await client.queries.save(
      projectId,
      {
        name,
        spec: spec(),
        ...(definition ? { expected_revision: definition.revision } : {}),
      },
      definition?.id,
    );
    update({ definition: saved, name: saved.name });
    await cache.invalidateQueries({
      queryKey: ["project", projectId, "queries"],
    });
    model.firstDefinitions();
    setNotice("查询条件已保存。");
  }
  async function run() {
    if (
      !sourceIds.length ||
      directory.pending ||
      directory.error ||
      invalid ||
      staleInput
    )
      return;
    const next = await client.queries.latestSpec(projectId, spec());
    const unchanged =
      definition && querySignature(next) === querySignature(definition.spec);
    const result =
      unchanged && definition
        ? await client.queries.build(
            projectId,
            definition.id,
            definition.revision,
          )
        : await client.queries.run(projectId, next);
    submittedScope.current = JSON.stringify(browserScope);
    setLastRun(result.id);
    model.firstPage();
  }
  useEffect(() => {
    if (
      !running ||
      !["ready", "failed", "cancelled", "interrupted"].includes(running.state)
    )
      return;
    if (
      running.state === "ready" &&
      submittedScope.current === JSON.stringify(browserScope)
    )
      onResult(running, resultLabel(running, name));
    setLastRun(null);
  }, [running, browserScope, onResult, name]);
  const mismatch =
    browserScope.kind === "source" &&
    (sourceIds.length !== 1 || sourceIds[0] !== browserScope.id);
  const displayedResults = (model.results.data?.items ?? []).filter(
    (r) => showReleased || r.state !== "released",
  );
  return (
    <section
      className="query-panel"
      aria-label="项目查询"
      style={{ "--query-height": height + "px" } as CSSProperties}
    >
      <header>
        <strong>项目查询</strong>
        <span className="subtle">多条件联合筛选</span>
        <span className="grow" />
        <button className="icon-button" aria-label="收起查询" onClick={onClose}>
          <X size={15} />
        </button>
      </header>
      <DraftStatus controller={draft.controller} quiet />
      <div className="query-panel-scroll">
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void action(run);
          }}
        >
          <fieldset
            className="draft-fields"
            disabled={!draft.editable || pending}
          >
            <div className="query-definition-row">
              <div className="query-saved-control">
                <span>已保存查询</span>
                <select
                  aria-label="已保存查询"
                  value={definition?.id ?? ""}
                  onChange={(e) => {
                    const q = savedDefinitions.find(
                      (q) => q.id === e.target.value,
                    );
                    if (q) load(q);
                    else fresh();
                  }}
                >
                  <option value="">新建查询</option>
                  {savedDefinitions.map((q) => (
                    <option key={q.id} value={q.id}>
                      {q.name}
                    </option>
                  ))}
                </select>
                {definition && onManage && (
                  <MoreMenu
                    label="已保存查询"
                    items={[
                      {
                        label: "管理与引用关系",
                        action: () =>
                          onManage({ kind: "query", id: definition.id }),
                      },
                      {
                        label: "重命名与备注…",
                        action: () =>
                          onManage(
                            { kind: "query", id: definition.id },
                            "rename",
                          ),
                      },
                      {
                        label: "删除保存的查询…",
                        danger: true,
                        action: () =>
                          onManage(
                            { kind: "query", id: definition.id },
                            "remove",
                          ),
                      },
                    ]}
                  />
                )}
              </div>
              <label>
                <span>查询名称</span>
                <input
                  aria-label="查询名称"
                  value={name}
                  onChange={(e) => update({ name: e.target.value })}
                  maxLength={120}
                  required
                />
              </label>
              <fieldset className="query-sources" aria-label="查询数据湖">
                <legend>查询数据湖（可多选，最多 8 个）</legend>
                {[
                  ...sources,
                  ...sourceIds
                    .filter((id) => !sources.some((s) => s.id === id))
                    .map((id) => ({ id, name: id, available: false })),
                ].map((s) => {
                  const checked = sourceIds.includes(s.id);
                  const source = sources.find((item) => item.id === s.id);
                  const supported = !!source && sourceSupports(source, "query");
                  return (
                    <label key={s.id}>
                      <input
                        type="checkbox"
                        checked={checked}
                        disabled={
                          !checked &&
                          (!s.available || !supported || sourceIds.length >= 8)
                        }
                        onChange={(e) =>
                          update({
                            sourceIds: e.target.checked
                              ? [...sourceIds, s.id]
                              : sourceIds.filter((id) => id !== s.id),
                            inputScope:
                              inputScope?.target.kind === "source"
                                ? null
                                : inputScope,
                          })
                        }
                      />
                      {s.name}
                      {!s.available
                        ? "（不可用）"
                        : !supported
                          ? "（不支持查询）"
                          : ""}
                    </label>
                  );
                })}
              </fieldset>
              <label>
                <span>范围限制</span>
                <select
                  aria-label="查询范围限制"
                  value={
                    !inputScope || inputScope.target.kind === "source"
                      ? ""
                      : (selectedOption?.value ?? "saved")
                  }
                  onChange={(e) =>
                    update({
                      inputScope:
                        scopeOptions.find((o) => o.value === e.target.value)
                          ?.scope ?? null,
                    })
                  }
                >
                  <option value="">整个查询数据湖</option>
                  {scopeOptions.map((o) => (
                    <option key={o.value} value={o.value}>
                      {o.label}
                      {o.count == null
                        ? ""
                        : " · " + o.count.toLocaleString() + " 项"}
                    </option>
                  ))}
                  {inputScope &&
                    !selectedOption &&
                    inputScope.target.kind !== "source" && (
                      <option value="saved">已保存范围（需检查）</option>
                    )}
                </select>
              </label>
            </div>
            {mismatch && (
              <p className="query-scope-note">
                当前查询：
                {sourceIds
                  .map((id) => sources.find((s) => s.id === id)?.name ?? id)
                  .join("、") || "未选择来源"}
                ；当前浏览：{browserScope.name}。
                <button type="button" onClick={fresh}>
                  以浏览来源新建查询
                </button>
              </p>
            )}
            {staleInput && (
              <p className="query-scope-note">
                选择范围已经变化。
                <button
                  type="button"
                  onClick={() => update({ inputScope: selectedOption.scope })}
                >
                  重新使用当前选择
                </button>
              </p>
            )}
            {inputScope && inputScope.target.kind !== "source" && (
              <p className="query-scope-note">
                仅在{scopeLabel(inputScope, inputOptions)}
                中查询所选数据湖的图片。
              </p>
            )}
            {(model.definitionCursor ||
              model.definitions.data?.next_cursor) && (
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
            {sourceIds.length > 1 && (
              <p className="query-rule-hint">
                多湖查询使用所选来源共同支持的字段与操作；同一图片在不同数据湖中分别保留。
              </p>
            )}
            <QueryConditions
              fields={fields}
              conditions={conditions}
              onChange={(conditions) => update({ conditions })}
            />
            {!directory.pending &&
              sourceIds.length > 0 &&
              (!directory.orders.includes(order) ||
                !directory.observation_rules.includes(rule)) && (
                <p className="condition-error">
                  所选来源不共同支持当前排序或观察规则，请调整后执行。
                </p>
              )}
            <div className="query-run-row">
              <select
                aria-label="观察判定规则"
                value={rule}
                onChange={(e) =>
                  update({
                    rule: e.target.value as QuerySpec["observation_rule"],
                  })
                }
              >
                <option
                  value="current_post"
                  disabled={
                    !directory.observation_rules.includes("current_post")
                  }
                >
                  当前帖子记录
                </option>
                <option
                  value="any_observation"
                  disabled={
                    !directory.observation_rules.includes("any_observation")
                  }
                >
                  任一关联历史记录
                </option>
              </select>
              <select
                aria-label="查询排序"
                value={order}
                onChange={(e) =>
                  update({ order: e.target.value as QuerySpec["order"] })
                }
              >
                <option
                  value="post_id_desc"
                  disabled={!directory.orders.includes("post_id_desc")}
                >
                  帖子 ID 降序
                </option>
                <option
                  value="post_id_asc"
                  disabled={!directory.orders.includes("post_id_asc")}
                >
                  帖子 ID 升序
                </option>
                <option
                  value="asset_key_asc"
                  disabled={!directory.orders.includes("asset_key_asc")}
                >
                  图像身份升序
                </option>
                <option
                  value="asset_key_desc"
                  disabled={!directory.orders.includes("asset_key_desc")}
                >
                  图像身份降序
                </option>
              </select>
              <span className="grow" />
              <Button
                type="button"
                disabled={
                  pending ||
                  invalid ||
                  !sourceIds.length ||
                  !name.trim() ||
                  directory.pending ||
                  !!directory.error
                }
                onClick={() => void action(save)}
              >
                <Save size={13} />
                保存条件
              </Button>
              <Button
                type="submit"
                className="primary"
                disabled={
                  pending ||
                  building ||
                  invalid ||
                  !!staleInput ||
                  !sourceIds.length ||
                  directory.pending ||
                  !!directory.error
                }
              >
                <Play size={13} />
                {pending ? "提交中…" : building ? "查询中…" : "执行查询"}
              </Button>
            </div>
            <p className="query-rule-hint">
              {rule === "current_post"
                ? "按当前帖子记录筛选。"
                : "同一条历史记录必须满足全部条件。"}
              分级和标签均按来源原值匹配；查询不会自动覆盖保存的条件。
            </p>
          </fieldset>
        </form>
        {!!(
          error ||
          directory.error ||
          model.results.error ||
          model.definitions.error
        ) && (
          <ErrorDetails
            error={
              error ||
              directory.error ||
              model.results.error ||
              model.definitions.error
            }
          />
        )}
        {notice && (
          <p role="status" className="query-scope-note">
            {notice}
          </p>
        )}
        <details className="query-history" open>
          <summary>查询结果 · {displayedResults.length} 项</summary>
          <div className="query-history-options">
            <label>
              <input
                type="checkbox"
                checked={showReleased}
                onChange={(e) => setShowReleased(e.target.checked)}
              />
              显示已释放记录
            </label>
          </div>
          <div className="query-result-list">
            {!displayedResults.length && (
              <p className="query-rule-hint">
                执行查询后，结果会显示在这里。查询期间可以继续使用其他页面。
              </p>
            )}
            {displayedResults.map((result) => {
              const queryName =
                savedDefinitions.find((q) => q.id === result.definition_id)
                  ?.name ??
                (result.definition_id
                  ? "已保存查询"
                  : result.spec.conditions.length
                    ? "即时筛选"
                    : "数据湖范围");
              const working = ["queued", "running"].includes(result.state);
              const visible =
                browserScope.kind === "result" && browserScope.id === result.id;
              return (
                <div
                  className={"query-result-row " + (visible ? "current" : "")}
                  key={result.id}
                >
                  <div>
                    <strong>
                      {queryName}
                      {visible && (
                        <span className="result-current">正在浏览</span>
                      )}
                    </strong>
                    <small>
                      {stateNames[result.state]} ·{" "}
                      {result.count == null
                        ? "数量待确定"
                        : result.count.toLocaleString() + " 张"}
                      {working && result.processed > 0
                        ? " · 已处理 " +
                          result.processed.toLocaleString() +
                          " 条记录"
                        : ""}
                      <span className="result-id" title={result.id}>
                        {" "}
                        ·{" "}
                        {new Date(Number(result.created_at)).toLocaleTimeString(
                          [],
                          { hour: "2-digit", minute: "2-digit" },
                        )}
                      </span>
                    </small>
                    <small>
                      数据湖：
                      {result.spec.source_ids
                        .map(
                          (id) => sources.find((s) => s.id === id)?.name ?? id,
                        )
                        .join("、")}
                    </small>
                    <p
                      className="result-condition-summary"
                      title={conditionText(result)}
                    >
                      {conditionText(result)}
                    </p>
                    <small>{queryCacheLabel(result)}</small>
                    {result.error && result.state !== "released" && (
                      <ErrorDetails compact error={result.error} />
                    )}
                  </div>
                  <div className="query-result-actions">
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
                          选择全部
                        </Button>
                      </>
                    )}
                    {working ? (
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
                        {
                          <button
                            className="icon-button"
                            disabled={pending}
                            title="按最新来源刷新结果"
                            onClick={() =>
                              void action(async () => {
                                const q = result.definition_id
                                  ? (
                                      await client.queries.definition(
                                        projectId,
                                        result.definition_id,
                                      )
                                    ).spec
                                  : result.spec;
                                const next = await client.queries.runLatest(
                                  projectId,
                                  q,
                                );
                                submittedScope.current =
                                  JSON.stringify(browserScope);
                                setLastRun(next.id);
                                model.firstPage();
                              })
                            }
                          >
                            <RotateCw size={14} />
                          </button>
                        }
                        {result.state !== "released" && (
                          <button
                            className="release-result"
                            disabled={pending || !onManage}
                            title="查看引用关系并删除此查询结果"
                            onClick={() =>
                              onManage?.(
                                { kind: "query_result", id: result.id },
                                "remove",
                              )
                            }
                          >
                            删除…
                          </button>
                        )}
                      </>
                    )}
                  </div>
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
        </details>
      </div>
      <ResizeGrip
        label="查询面板高度"
        orientation="horizontal"
        value={height}
        minimum={190}
        maximum={640}
        onChange={onHeight}
        onReset={() => onHeight(350)}
      />
    </section>
  );
}
