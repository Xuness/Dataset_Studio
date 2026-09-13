import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Filter, Play, X, Save, LoaderCircle } from "lucide-react";
import {
  Button,
  Dialog,
  ErrorDetails,
  FiltersEditor,
  DraftStatus,
  useDraft,
  emptyFilters,
  filterConditions,
  filterError,
  queryCacheLabel,
  browseScopeIdentity,
} from "@studio/ui";
import type { BrowseFilters, BrowseScope, ModuleContext } from "@studio/ui";
import type { QuerySpec, QueryResult } from "@studio/contracts";
import type { StudioClient } from "@studio/client";
import { rankableScope } from "./rankingBrowse.js";

type FilterDraft = {
  ratingUpgrade?: boolean;
  filters: BrowseFilters;
  baseScope: BrowseScope | null;
  resultId: string | null;
  submitted: string;
  retiredResult: string | null;
};
const initial: FilterDraft = {
  filters: emptyFilters,
  baseScope: null,
  resultId: null,
  submitted: "",
  retiredResult: null,
};
const scopeKey = (scope: BrowseScope) => JSON.stringify(scope);
function decode(value: unknown): FilterDraft | null {
  if (!value || typeof value !== "object") return null;
  const v = value as FilterDraft;
  if (
    !v.filters ||
    !Array.isArray(v.filters.ratings) ||
    v.filters.ratings.some((r) => typeof r !== "string") ||
    typeof v.filters.include !== "string" ||
    typeof v.filters.exclude !== "string" ||
    !["all", "any"].includes(v.filters.tagMode) ||
    (v.resultId !== null && typeof v.resultId !== "string") ||
    typeof v.submitted !== "string"
  )
    return null;
  if (
    v.baseScope !== null &&
    (!v.baseScope ||
      !["all", "source", "collection", "result", "selection"].includes(
        v.baseScope.kind,
      ))
  )
    return null;
  return { ...v, retiredResult: v.retiredResult ?? null };
}
function scopeName(scope: BrowseScope) {
  return scope.kind === "all" ? "全部项目数据" : scope.name;
}
export function adoptRefreshedFilter(
  client: StudioClient,
  projectId: string,
  previousId: string,
  result: QueryResult,
) {
  const controller = client.edits.open(
    {
      projectId,
      moduleId: "core.browser",
      instanceId: "default",
      schemaVersion: 1,
    },
    initial,
    decode,
  );
  const snapshot = controller.getSnapshot();
  if (
    snapshot.value.resultId !== previousId ||
    ["loading", "unsupported", "conflict"].includes(snapshot.status)
  )
    return;
  controller.set((v) => ({ ...v, resultId: result.id, retiredResult: null }));
}
export function QuickFilters({ context }: { context: ModuleContext }) {
  const { client, projectId, browser, sources, inputOptions } = context;
  const cache = useQueryClient();
  const draft = useDraft(client, projectId, "core.browser", initial, decode);
  const value = draft.value;
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [saveName, setSaveName] = useState<string | null>(null);
  const [notice, setNotice] = useState("");
  const [expanded, setExpanded] = useState(true);
  const ownedView =
    browser.scope.kind === "result" &&
    (browser.scope.id === value.resultId ||
      browser.scope.id === value.retiredResult);
  const base = ownedView && value.baseScope ? value.baseScope : browser.scope;
  const rankingTarget = rankableScope(projectId, base);
  const rankingInfo = useQuery({
    queryKey: ["project", projectId, "quick-filter-ranking", rankingTarget],
    queryFn: ({ signal }) =>
      client.ranking.browseInfo(projectId, rankingTarget!, signal),
    enabled: !!rankingTarget,
    staleTime: 15000,
  });
  const rankingArtifact =
    typeof rankingInfo.data?.ranking?.current_rating_filter === "boolean"
      ? rankingInfo.data.ranking.artifact_id
      : undefined;
  const checkingRating = !!rankingTarget && rankingInfo.isPending;
  const option =
    base.kind === "all"
      ? undefined
      : inputOptions.find(
          (o) =>
            o.value === (base.kind === "selection" ? "selection" : base.id),
        );
  const sourceIds = sources
    .filter(
      (s) =>
        s.available &&
        s.kind === "danbooru" &&
        (base.kind !== "source" || s.id === base.id),
    )
    .map((s) => s.id);
  const usable =
    sourceIds.length > 0 &&
    (base.kind === "all" || !!option) &&
    option?.count !== 0;
  const originalClauses = filterConditions(value.filters);
  const clauses = originalClauses.map((c) =>
    c.field === "rating" && rankingArtifact
      ? { ...c, field: `project.${rankingArtifact}.rating` }
      : c,
  );
  const signature = JSON.stringify(clauses);
  const issue = filterError(value.filters);
  const result = useQuery({
    queryKey: ["project", projectId, "quick-filter", value.resultId],
    queryFn: ({ signal }) =>
      client.queries.result(projectId, value.resultId!, signal),
    enabled: !!value.resultId,
    refetchInterval: (q) =>
      q.state.data && ["queued", "running"].includes(q.state.data.state)
        ? 1500
        : false,
  });
  const building =
    !!value.resultId &&
    (!result.data || ["queued", "running"].includes(result.data.state));
  const migrationInfo = useQuery({
    queryKey: ["project", projectId, "quick-filter-upgrade", value.resultId],
    queryFn: ({ signal }) =>
      client.ranking.browseInfo(
        projectId,
        {
          project_id: projectId,
          target: { kind: "query_result", result_id: value.resultId! },
        },
        signal,
      ),
    enabled: !!value.ratingUpgrade && result.data?.state === "ready",
    staleTime: Infinity,
  });
  const buildSpec = (): QuerySpec => ({
    version: 3,
    source_ids: sourceIds,
    conditions: clauses,
    observation_rule: "current_post",
    order: browser.order,
    ...(option ? { input_scope: option.scope } : {}),
  });
  async function release(id: string) {
    // Finished memberships remain available to the bounded reuse cache.
    try {
      await client.queries.cancel(projectId, id);
    } catch {
      /* Preserve referenced results. */
    }
  }
  useEffect(() => {
    if (
      !draft.editable ||
      !value.baseScope ||
      scopeKey(browser.scope) === scopeKey(value.baseScope) ||
      ownedView
    )
      return;
    const previous = value.resultId;
    draft.controller.set(initial);
    setError(null);
    setNotice("");
    if (previous) void release(previous);
  }, [
    draft.editable,
    draft.controller,
    value.baseScope,
    value.resultId,
    browser.scope,
    ownedView,
  ]);
  useEffect(() => {
    if (!result.data || result.data.state !== "ready" || !value.baseScope)
      return;
    if (
      scopeKey(browser.scope) === scopeKey(value.baseScope) ||
      (ownedView &&
        browser.scope.kind === "result" &&
        browser.scope.id !== result.data.id)
    ) {
      if (value.ratingUpgrade) {
        if (migrationInfo.isPending) return;
        const next = migrationInfo.data?.ranking;
        if (next && browser.rankedBrowse) {
          browser.onRankedBrowse({
            ...browser.rankedBrowse,
            scopeKey: next.view_key,
            sourceScopeKey: JSON.stringify(
              browseScopeIdentity({
                kind: "result",
                id: result.data.id,
                name: "筛选",
              }),
            ),
            startPostId: null,
            startCursor: null,
          });
        }
      }
      context.onResult(result.data, "筛选 · " + scopeName(value.baseScope));
      return;
    }
    if (
      browser.scope.kind === "result" &&
      browser.scope.id === result.data.id &&
      value.retiredResult &&
      value.retiredResult !== result.data.id
    ) {
      const id = value.retiredResult;
      draft.controller.set((v) => ({
        ...v,
        retiredResult: null,
        ratingUpgrade: false,
      }));
      void release(id);
    }
  }, [
    result.data,
    value.baseScope,
    value.retiredResult,
    browser.scope,
    context.onResult,
    draft.controller,
    ownedView,
    value.ratingUpgrade,
    migrationInfo.data,
    migrationInfo.isPending,
  ]);
  async function run(upgrade = false) {
    if (!clauses.length) {
      clear();
      return;
    }
    if (!usable || issue || checkingRating) return;
    setPending(true);
    setError(null);
    setNotice(
      upgrade ? "正在按评分分级更新旧筛选；完成后复用新的成员缓存。" : "",
    );
    try {
      if (building && value.resultId)
        await client.queries.cancel(projectId, value.resultId);
      const next = await client.queries.runLatest(projectId, buildSpec());
      if (value.resultId && !ownedView) void release(value.resultId);
      draft.controller.set((v) => ({
        ...v,
        baseScope: base,
        resultId: next.id,
        submitted: signature,
        retiredResult: ownedView ? (v.retiredResult ?? v.resultId) : null,
        ratingUpgrade: upgrade,
      }));
      await cache.invalidateQueries({
        queryKey: ["project", projectId, "query-results"],
      });
    } catch (e) {
      setError(e);
    } finally {
      setPending(false);
    }
  }
  const upgrading = useRef<string | null>(null);
  useEffect(() => {
    // Upgrade only an unchanged browser-owned legacy filter, retaining the old
    // immutable result. Unsubmitted user edits are never applied automatically.
    const legacy =
      JSON.stringify(originalClauses) === value.submitted &&
      result.data?.spec.conditions.some((c) => c.field === "rating");
    const interruptedUpgrade =
      result.data?.state === "interrupted" &&
      !!value.retiredResult &&
      signature === value.submitted &&
      result.data.spec.conditions.some(
        (c) => c.field === `project.${rankingArtifact}.rating`,
      );
    if (
      !ownedView ||
      !rankingArtifact ||
      !draft.editable ||
      pending ||
      building ||
      !value.resultId ||
      upgrading.current === value.resultId ||
      (!legacy && !interruptedUpgrade)
    )
      return;
    upgrading.current = value.resultId;
    void run(true);
  }, [
    ownedView,
    rankingArtifact,
    draft.editable,
    pending,
    building,
    value.resultId,
    value.submitted,
    signature,
    result.data,
  ]);
  function clear() {
    const id = value.resultId;
    draft.controller.set(initial);
    setError(null);
    setNotice("");
    if (ownedView && value.baseScope) browser.onScope(value.baseScope);
    if (id) void release(id);
    if (value.retiredResult && value.retiredResult !== id)
      void release(value.retiredResult);
  }
  async function save() {
    if (!saveName?.trim()) return;
    setPending(true);
    setError(null);
    try {
      const savedSpec = buildSpec();
      if (savedSpec.input_scope?.target.kind === "source")
        delete savedSpec.input_scope;
      await client.queries.save(projectId, {
        name: saveName.trim(),
        spec: savedSpec,
      });
      await cache.invalidateQueries({
        queryKey: ["project", projectId, "queries"],
      });
      setSaveName(null);
      setNotice("查询已保存，可在项目查询中重新打开。");
    } catch (e) {
      setError(e);
    } finally {
      setPending(false);
    }
  }
  const progress =
    result.data?.state === "ready"
      ? (result.data.count ?? 0).toLocaleString() + " 张匹配"
      : building
        ? result.data?.processed
          ? "筛选中 · 已处理 " +
            result.data.processed.toLocaleString() +
            " 条记录"
          : "正在筛选…"
        : "";
  return (
    <section className="quick-filters" aria-label="浏览筛选">
      <div className="quick-filter-heading">
        <button
          type="button"
          aria-expanded={expanded}
          onClick={() => setExpanded((v) => !v)}
        >
          <Filter size={15} />
          <strong>筛选</strong>
        </button>
        <span className="filter-scope">范围：{scopeName(base)}</span>
        <span className="grow" />
        {progress && (
          <span role="status">
            {building && <LoaderCircle className="loading-icon" size={12} />}{" "}
            {progress}
          </span>
        )}
        {value.resultId && (
          <button
            type="button"
            onClick={clear}
            disabled={!draft.editable || pending}
          >
            <X size={13} />
            清除筛选
          </button>
        )}
      </div>
      <DraftStatus controller={draft.controller} quiet />
      {rankingInfo.error && (
        <ErrorDetails error={rankingInfo.error} title="无法读取评分分级依据" />
      )}
      {expanded && (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void run();
          }}
        >
          <fieldset
            className="draft-fields"
            disabled={
              !draft.editable ||
              pending ||
              !usable ||
              checkingRating ||
              rankingInfo.isError
            }
          >
            <FiltersEditor
              value={value.filters}
              onChange={(filters) =>
                draft.controller.set((v) => ({ ...v, filters }))
              }
            />
            <div className="quick-filter-actions">
              <span className="subtle">
                {rankingArtifact
                  ? "分级按评分时的快照筛选；标签按当前元数据筛选。"
                  : "各组条件同时满足；分级多选为任一满足。"}
              </span>
              <span className="grow" />
              {value.resultId && signature !== value.submitted && (
                <span className="filter-unapplied">条件尚未应用</span>
              )}
              <Button
                type="button"
                disabled={!clauses.length || !!issue || building}
                onClick={() => setSaveName(scopeName(base) + " · 筛选")}
              >
                <Save size={13} />
                保存查询…
              </Button>
              <Button
                className="primary"
                type="submit"
                disabled={
                  building || !!issue || (!clauses.length && !value.resultId)
                }
              >
                <Play size={13} />
                {pending ? "提交中…" : "应用筛选"}
              </Button>
              {building && (
                <Button
                  type="button"
                  onClick={() =>
                    void client.queries
                      .cancel(projectId, value.resultId!)
                      .then(() => result.refetch())
                      .catch(setError)
                  }
                >
                  取消
                </Button>
              )}
            </div>
          </fieldset>
        </form>
      )}
      {!usable && (
        <p className="filter-note">
          {!sourceIds.length
            ? "此范围没有可用于 Rating / Tag 筛选的 Danbooru 来源。"
            : option?.count === 0
              ? "当前范围还没有图片。"
              : "范围暂不可用，请刷新来源后重试。"}
        </p>
      )}
      {issue && <p className="condition-error">{issue}</p>}
      {notice && (
        <p role="status" className="filter-note">
          {notice}
        </p>
      )}
      {result.data && (
        <p className="filter-note" role="status">
          {queryCacheLabel(result.data)}
        </p>
      )}
      {!!(
        error ||
        result.error ||
        (result.data?.state !== "released" && result.data?.error)
      ) && (
        <ErrorDetails
          compact
          error={error || result.error || result.data?.error}
        />
      )}
      {result.data &&
        ["cancelled", "interrupted"].includes(result.data.state) &&
        !result.data.error && (
          <p className="filter-note">筛选已停止，可调整条件重新执行。</p>
        )}
      {saveName !== null && (
        <Dialog title="保存筛选条件" onClose={() => setSaveName(null)}>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void save();
            }}
          >
            <label className="field">
              <span>查询名称</span>
              <input
                autoFocus
                aria-label="筛选查询名称"
                required
                maxLength={120}
                value={saveName}
                onChange={(e) => setSaveName(e.target.value)}
              />
            </label>
            <p className="subtle">保存范围与条件，不会重新执行筛选。</p>
            {!!error && <ErrorDetails error={error} />}
            <footer>
              <Button type="button" onClick={() => setSaveName(null)}>
                取消
              </Button>
              <Button type="submit" disabled={pending || !saveName.trim()}>
                保存
              </Button>
            </footer>
          </form>
        </Dialog>
      )}
    </section>
  );
}
