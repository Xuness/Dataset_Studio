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
} from "@studio/ui";
import type { BrowseScope, ModuleScopeOption } from "@studio/ui";
import type { StudioClient } from "@studio/client";
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
  sourceId: string;
  conditions: QuerySpec["conditions"];
  rule: QuerySpec["observation_rule"];
  order: QuerySpec["order"];
  inputScope: ScopeRef | null;
};
const initialDraft: QueryDraft = {
  definition: null,
  name: "资料筛选",
  sourceId: "",
  conditions: [],
  rule: "current_post",
  order: "post_id_desc",
  inputScope: null,
};
function decodeDraft(value: unknown): QueryDraft | null {
  if (!value || typeof value !== "object") return null;
  const v = value as QueryDraft;
  if (
    typeof v.name !== "string" ||
    typeof v.sourceId !== "string" ||
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
}) {
  const cache = useQueryClient();
  const draft = useDraft(
    client,
    projectId,
    "core.query",
    initialDraft,
    decodeDraft,
  );
  const { definition, name, sourceId, conditions, rule, order, inputScope } =
    draft.value;
  const update = (patch: Partial<QueryDraft>) =>
    draft.controller.set((v) => ({ ...v, ...patch }));
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [notice, setNotice] = useState("");
  const [showReleased, setShowReleased] = useState(false);
  const [lastRun, setLastRun] = useState<string | null>(null);
  const submittedScope = useRef<string | null>(null);
  const multiSource = !!definition && definition.spec.source_ids.length > 1;
  const listed = model.definitions.data?.items ?? [];
  const savedDefinitions =
    definition && !listed.some((q) => q.id === definition.id)
      ? [definition, ...listed]
      : listed;
  useEffect(() => {
    if (draft.editable && !sourceId) {
      const preferred =
        sources.find(
          (s) =>
            browserScope.kind === "source" &&
            s.id === browserScope.id &&
            s.available,
        ) ??
        sources.find((s) => s.kind === "danbooru" && s.available) ??
        sources.find((s) => s.available);
      if (preferred)
        draft.controller.set((v) => ({ ...v, sourceId: preferred.id }));
    }
  }, [draft.editable, draft.controller, sourceId, sources, browserScope]);
  const directory = useQuery({
    queryKey: ["project", projectId, "fields", sourceId],
    queryFn: ({ signal }) => client.queries.fields(projectId, sourceId, signal),
    enabled: !!sourceId,
  });
  const fields =
    directory.data?.fields.filter((f) => f.operators.length > 0) ?? [];
  const invalid =
    !multiSource &&
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
  const spec = (): QuerySpec =>
    multiSource && definition
      ? definition.spec
      : {
          version: 3,
          source_ids: [sourceId],
          conditions,
          observation_rule: rule,
          order,
          ...(inputScope ? { input_scope: inputScope } : {}),
        };
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
      sourceId: query.spec.source_ids[0] ?? "",
      conditions: query.spec.conditions,
      rule: query.spec.observation_rule,
      order: query.spec.order,
      inputScope: query.spec.input_scope ?? null,
    });
    setError(null);
    setNotice("");
  }
  function fresh() {
    const source = browserScope.kind === "source" ? browserScope.id : sourceId;
    draft.controller.set({ ...initialDraft, sourceId: source });
    setError(null);
    setNotice("");
  }
  async function save() {
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
    browserScope.kind === "source" && browserScope.id !== sourceId;
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
              <label>
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
              </label>
              <label>
                <span>查询名称</span>
                <input
                  aria-label="查询名称"
                  disabled={multiSource}
                  value={name}
                  onChange={(e) => update({ name: e.target.value })}
                  maxLength={120}
                  required
                />
              </label>
              <label>
                <span>查询数据湖</span>
                <select
                  aria-label="查询数据湖"
                  disabled={multiSource}
                  value={sourceId}
                  required
                  onChange={(e) =>
                    update({
                      sourceId: e.target.value,
                      inputScope: null,
                      conditions: [],
                    })
                  }
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
              </label>
              <label>
                <span>范围限制</span>
                <select
                  aria-label="查询范围限制"
                  disabled={multiSource}
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
                当前查询：{sources.find((s) => s.id === sourceId)?.name}
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
            {multiSource ? (
              <p className="query-rule-hint">
                此定义包含多个数据湖，可按保存的条件重新计算。
              </p>
            ) : (
              <QueryConditions
                fields={fields}
                conditions={conditions}
                onChange={(conditions) => update({ conditions })}
              />
            )}
            <div className="query-run-row">
              <select
                aria-label="观察判定规则"
                disabled={multiSource}
                value={rule}
                onChange={(e) =>
                  update({
                    rule: e.target.value as QuerySpec["observation_rule"],
                  })
                }
              >
                <option value="current_post">当前帖子记录</option>
                <option value="any_observation">任一关联历史记录</option>
              </select>
              <select
                aria-label="查询排序"
                disabled={multiSource}
                value={order}
                onChange={(e) =>
                  update({ order: e.target.value as QuerySpec["order"] })
                }
              >
                <option value="post_id_desc">Danbooru ID 从新到旧</option>
                <option value="post_id_asc">Danbooru ID 从旧到新</option>
                <option value="asset_key_asc">图像身份升序</option>
                <option value="asset_key_desc">图像身份降序</option>
              </select>
              <span className="grow" />
              <Button
                type="button"
                disabled={
                  pending ||
                  multiSource ||
                  invalid ||
                  !sourceId ||
                  !name.trim() ||
                  directory.isPending ||
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
                  !sourceId ||
                  directory.isPending ||
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
                            disabled={pending || visible}
                            title={
                              visible
                                ? "先切换到其他范围，再释放当前结果"
                                : "释放此结果引用；复用缓存可在读取与缓存中清理"
                            }
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
