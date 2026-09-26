import {
  isRankingOperator,
  operatorFor,
  orderLabel,
  v2Defaults,
  v2OperatorId,
} from "./v2.js";
import { useEffect, useId, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Calculator, Play, RotateCw } from "lucide-react";
import {
  Button,
  Dialog,
  DraftStatus,
  ErrorDetails,
  Field,
  ResizeGrip,
  isJobActive,
  useDraft,
  MoreMenu,
} from "@studio/ui";
import type { ModuleContext } from "@studio/ui";
import type {
  Artifact,
  Collection,
  RankingFilter,
  RankingRow,
  RankingParameters,
  Schema,
  ScopeRef,
  Job,
  OperatorRun,
} from "@studio/contracts";
import type { ObjectListOptions } from "@studio/client";
import { useObjectList } from "../management/useObjectList.js";
import { PresetControls } from "../tools/PresetControls.js";
import { RankingConfig } from "./RankingConfig.js";
import { RankingDetails } from "./RankingDetails.js";
import { RankingOverview } from "./RankingOverview.js";
import { RankingDiagnostics } from "./RankingDiagnostics.js";
import { RankingJob } from "./RankingJob.js";
import {
  decode,
  defaultFilter,
  eligibilityNames,
  flagNames,
  initial,
  number,
  parameterIssue,
  routeNames,
  score,
} from "./types.js";
import "./ranking.css";

function canonical(value: unknown): string {
  return JSON.stringify(value, (_key, v: unknown) =>
    v && typeof v === "object" && !Array.isArray(v)
      ? Object.fromEntries(
          Object.entries(v).sort(([a], [b]) => a.localeCompare(b)),
        )
      : v,
  );
}
function scopeEqual(a: ScopeRef | null, b: ScopeRef | undefined) {
  return !!a && !!b && canonical(a) === canonical(b);
}

export default function RankingPanel(context: ModuleContext) {
  const { client, projectId } = context;
  const cache = useQueryClient();
  const formId = useId();
  const draft = useDraft(
    client,
    projectId,
    "core.tools",
    initial,
    decode,
    "ranking",
  );
  const d = draft.value;
  const applied = useRef<number | null>(null);
  const [pending, setPending] = useState(false);
  const [submissionOperation, setSubmissionOperation] = useState<string | null>(
    null,
  );
  const submissionProgress = useQuery({
    queryKey: ["project", projectId, "member-write", submissionOperation],
    queryFn: ({ signal }) =>
      client.ranking.saveProgress(projectId, submissionOperation!, signal),
    enabled: pending && !!submissionOperation,
    gcTime: 0,
    refetchInterval: 400,
  });
  const submissionLock = useRef(false);
  const [jobAction, setJobAction] = useState<"cancel" | "retry" | null>(null);
  const [error, setError] = useState("");
  const [cursor, setCursor] = useState<string | null>(null);
  const [history, setHistory] = useState<(string | null)[]>([]);
  const [count, setCount] = useState<number | null>(null);
  const filterKey = canonical(d.filter);
  const [settledFilter, setSettledFilter] = useState(filterKey);
  useEffect(() => {
    const timer = setTimeout(() => setSettledFilter(filterKey), 180);
    return () => clearTimeout(timer);
  }, [filterKey]);
  const pageKey = JSON.stringify([d.artifactId, filterKey, cursor]);
  const preparingPage = useRef<{ key: string; cursor: string } | null>(null);
  const [focused, setFocused] = useState<RankingRow | null>(null);
  const [worksetOpen, setWorksetOpen] = useState(false);
  const [savingWorkset, setSavingWorkset] = useState(false);
  const [saveOperation, setSaveOperation] = useState<string | null>(null);
  const saveProgress = useQuery({
    queryKey: ["project", projectId, "member-write", saveOperation],
    queryFn: ({ signal }) =>
      client.ranking.saveProgress(projectId, saveOperation!, signal),
    enabled: savingWorkset && !!saveOperation,
    gcTime: 0,
    refetchInterval: 400,
  });
  const [saved, setSaved] = useState<Collection | null>(null);
  const requirements = useQuery({
    queryKey: [
      "project",
      projectId,
      "ranking-source-requirements",
      d.scope,
      context.sources.map((s) => [s.id, s.revision]),
    ],
    queryFn: ({ signal }) =>
      client.sourceAccess.requirements(
        projectId,
        d.scope!,
        ["danbooru_ranking_v1"],
        signal,
      ),
    enabled: draft.editable && !!d.scope,
    retry: false,
  });
  const hasDanbooru = requirements.data?.supported === true;
  const option = context.inputOptions.find((o) => o.value === d.scopeId);
  const scopeCheck = useQuery({
    queryKey: ["project", projectId, "ranking-scope", d.scope],
    queryFn: ({ signal }) =>
      client.tools.validateScope(projectId, d.scope!, signal),
    enabled: draft.editable && !!d.scope && hasDanbooru,
    retry: false,
    gcTime: 0,
  });
  const stale =
    scopeCheck.isError ||
    (!!option && !!d.scope && !scopeEqual(d.scope, option.scope));
  const jobs = useQuery({
    queryKey: ["project", projectId, "jobs"],
    queryFn: () => client.jobs(projectId),
    enabled: draft.editable,
    refetchInterval: 2000,
    refetchIntervalInBackground: true,
  });
  const runningJob = jobs.data?.items.find(
    (j) => isRankingOperator(j.operator) && isJobActive(j),
  );
  const job = runningJob ?? jobs.data?.items.find((j) => j.id === d.lastJob);
  const trackedJobId = job?.id ?? d.lastJob;
  const active = isJobActive(runningJob);
  const jobArtifact = useQuery({
    queryKey: ["project", projectId, "ranking-job-artifact", trackedJobId],
    queryFn: ({ signal }) =>
      client.ranking.jobResult(projectId, trackedJobId!, signal),
    enabled: !!trackedJobId && job?.status === "succeeded",
    retry: false,
  });
  const artifact = useQuery({
    queryKey: ["project", projectId, "artifact", d.artifactId],
    queryFn: ({ signal }) =>
      client.tools.artifact(projectId, d.artifactId, signal),
    enabled: !!d.artifactId && draft.editable,
  });
  const summary = useQuery({
    queryKey: ["project", projectId, "ranking-summary", d.artifactId],
    queryFn: ({ signal }) =>
      client.ranking.summary(projectId, d.artifactId, signal),
    enabled: artifact.data?.state === "ready",
    retry: false,
  });
  const results = useQuery({
    queryKey: [
      "project",
      projectId,
      "ranking-rows",
      d.artifactId,
      d.filter,
      cursor,
    ],
    queryFn: async ({ signal }) => {
      const continuation =
        preparingPage.current?.key === pageKey
          ? preparingPage.current.cursor
          : cursor;
      const page = await client.ranking.rows(
        projectId,
        d.artifactId,
        { filter: d.filter, cursor: continuation, limit: 48 },
        signal,
      );
      if (!signal.aborted)
        preparingPage.current =
          page.preparing && page.next_cursor
            ? { key: pageKey, cursor: page.next_cursor }
            : null;
      return page;
    },
    enabled:
      !!summary.data &&
      d.tab === "results" &&
      d.filterArtifact === d.artifactId &&
      settledFilter === filterKey,
    retry: false,
    gcTime: 0,
    refetchInterval: (q) =>
      q.state.status !== "error" && q.state.data?.preparing ? 30 : false,
  });
  const counting = useQuery({
    queryKey: ["project", projectId, "ranking-count", d.artifactId, d.filter],
    queryFn: ({ signal }) =>
      client.ranking.count(projectId, d.artifactId, d.filter, signal),
    enabled:
      !!summary.data &&
      d.tab === "results" &&
      settledFilter === filterKey &&
      results.data?.count === null,
    retry: false,
    gcTime: 0,
    refetchInterval: (q) =>
      q.state.status !== "error" && q.state.data?.count === null ? 100 : false,
  });
  const rankingList = useObjectList(
    client,
    projectId,
    "artifact",
    { subtype: "ranking_table", state: "ready" },
    active ? 5000 : false,
  );
  const artifacts = rankingList.query;
  const [parameterNotice, setParameterNotice] = useState("");
  function applyRun(run: OperatorRun) {
    const value = decode({ ...initial, parameters: run.parameters });
    if (
      !isRankingOperator(run.operator_id) ||
      run.operator_version !== 1 ||
      run.parameters_version !== 1 ||
      !value
    ) {
      setError("这份参数的格式或工具版本暂不兼容。");
      return;
    }
    draft.controller.set((old) => ({
      ...old,
      parameters: value.parameters,
      v2Saved: value.parameters.v2 ?? old.v2Saved ?? null,
      submission: null,
      tab: "config",
    }));
    setParameterNotice(
      "已载入参数，输入范围仍以当前配置为准。确认后再启动计算。",
    );
    context.management?.showProperties();
  }
  useEffect(() => {
    if (draft.editable && runningJob && d.lastJob !== runningJob.id)
      draft.controller.set((v) => ({ ...v, lastJob: runningJob.id }));
  }, [draft.editable, draft.controller, d.lastJob, runningJob]);
  useEffect(() => {
    if (!draft.editable || d.scope || d.scopeId || !context.inputOptions.length)
      return;
    const initialOption =
      context.inputOptions.find((o) => o.value === context.defaultInput) ??
      context.inputOptions[0];
    if (initialOption)
      draft.controller.set((v) => ({
        ...v,
        scope: initialOption.scope,
        scopeId: initialOption.value,
      }));
  }, [
    draft.controller,
    draft.editable,
    d.scope,
    d.scopeId,
    context.inputOptions,
    context.defaultInput,
  ]);
  useEffect(() => {
    const invocation = context.invocation;
    if (
      !draft.editable ||
      !invocation ||
      applied.current === invocation.sequence ||
      !isRankingOperator(invocation.args.operatorId)
    )
      return;
    if (invocation.args.reuseRun) {
      try {
        applyRun(JSON.parse(invocation.args.reuseRun) as OperatorRun);
      } catch {
        setError("历史参数无法读取，原配置已保留。");
      }
      applied.current = invocation.sequence;
      return;
    }
    draft.controller.set((v) => ({
      ...v,
      ...(!invocation.args.jobId && !invocation.args.artifactId
        ? {
            parameters: {
              ...v.parameters,
              v2:
                invocation.args.operatorId === v2OperatorId
                  ? (v.parameters.v2 ?? v.v2Saved ?? v2Defaults())
                  : null,
            },
            v2Saved: v.parameters.v2 ?? v.v2Saved ?? null,
            tab: "config" as const,
          }
        : {}),
      ...(invocation.args.artifactId
        ? {
            artifactId: invocation.args.artifactId,
            lastJob: null,
            tab: "results",
          }
        : {}),
      ...(invocation.args.jobId
        ? { lastJob: invocation.args.jobId, artifactId: "", tab: "results" }
        : {}),
    }));
    applied.current = invocation.sequence;
  }, [context.invocation, draft.controller, draft.editable]);
  useEffect(() => {
    if (draft.editable && artifact.data?.state === "released" && d.artifactId) {
      draft.controller.set((old) => ({
        ...old,
        artifactId: "",
        lastJob: null,
        filterArtifact: "",
      }));
      setParameterNotice("这份排名成果已删除，可从列表选择其他成果。");
    }
  }, [draft.editable, draft.controller, artifact.data?.state, d.artifactId]);
  useEffect(() => {
    const item = jobArtifact.data;
    if (draft.editable && item && d.lastJob === item.job_id && !d.artifactId)
      draft.controller.set((v) => ({ ...v, artifactId: item.id }));
  }, [
    jobArtifact.data,
    draft.controller,
    draft.editable,
    d.lastJob,
    d.artifactId,
  ]);
  useEffect(() => {
    if (draft.editable && summary.data && d.filterArtifact !== d.artifactId) {
      draft.controller.set((v) => ({
        ...v,
        filterArtifact: v.artifactId,
        filter: {
          ...defaultFilter,
          rating: summary.data!.ratings[0]?.rating ?? null,
          selected_only: summary.data!.parameters.mode === "select",
        },
      }));
      setCursor(null);
      setHistory([]);
      setFocused(null);
      setCount(null);
    }
  }, [
    summary.data,
    draft.controller,
    draft.editable,
    d.artifactId,
    d.filterArtifact,
  ]);
  useEffect(() => {
    if (results.data?.count !== null && results.data?.count !== undefined)
      setCount(results.data.count);
    else if (
      counting.data?.count !== null &&
      counting.data?.count !== undefined
    )
      setCount(counting.data.count);
  }, [results.data, counting.data]);
  function changeParameters(parameters: RankingParameters) {
    draft.controller.set((v) => ({
      ...v,
      parameters,
      v2Saved: parameters.v2 ?? v.v2Saved ?? null,
      submission: null,
    }));
  }
  function changeScope(id: string) {
    const selected = context.inputOptions.find((o) => o.value === id);
    if (selected)
      draft.controller.set((v) => ({
        ...v,
        scope: selected.scope,
        scopeId: id,
        submission: null,
      }));
  }
  async function rebind() {
    await cache.invalidateQueries({
      queryKey: ["project", projectId, "sources"],
    });
    const sourceData = cache.getQueryData<Schema["Sources"]>([
      "project",
      projectId,
      "sources",
    ]);
    let scope = option?.scope;
    if (scope?.target.kind === "source") {
      const id = scope.target.source_id;
      const source = sourceData?.items.find((s) => s.id === id);
      if (source?.revision)
        scope = {
          project_id: projectId,
          target: { kind: "source", source_id: id, revision: source.revision },
        };
    }
    if (scope) draft.controller.set((v) => ({ ...v, scope, submission: null }));
    await scopeCheck.refetch();
  }
  function setFilter(filter: RankingFilter) {
    if (canonical(filter) === canonical(d.filter)) return;
    draft.controller.set((v) => ({ ...v, filter }));
    setCursor(null);
    setHistory([]);
    setCount(null);
    setFocused(null);
  }
  function focusRow(row: RankingRow) {
    setFocused(row);
    context.inspector?.setVisible(true);
  }
  async function submit() {
    if (
      submissionLock.current ||
      active ||
      !d.scope ||
      stale ||
      !draft.editable ||
      parameterIssue(d.parameters)
    )
      return;
    submissionLock.current = true;
    setPending(true);
    setError("");
    try {
      const run = {
        operator_id: operatorFor(d.parameters),
        operator_version: 1,
        parameters_version: 1,
        parameters: d.parameters,
      };
      const signature = canonical({ run, scope: d.scope });
      const key =
        d.submission?.signature === signature
          ? d.submission.key
          : crypto.randomUUID();
      setSubmissionOperation(key);
      draft.controller.set((v) => ({ ...v, submission: { key, signature } }));
      await draft.controller.flush();
      const accepted = await client.tools.submit(projectId, {
        run,
        scope: d.scope,
        idempotency_key: key,
        delay_ms: 0,
      });
      rememberJob(accepted);
      draft.controller.set((v) => ({
        ...v,
        lastJob: accepted.id,
        submission: null,
        artifactId: "",
      }));
      await draft.controller.flush();
      void cache.invalidateQueries({
        queryKey: ["project", projectId, "jobs"],
      });
      context.onJob(accepted, { revealTasks: false });
    } catch (e) {
      if ((e as { code?: string }).code === "CANCELLED")
        draft.controller.set((v) => ({ ...v, submission: null }));
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(false);
      setSubmissionOperation(null);
      submissionLock.current = false;
    }
  }
  function rememberJob(accepted: Job) {
    cache.setQueryData<Schema["Jobs"]>(
      ["project", projectId, "jobs"],
      (data) => ({
        items: [
          accepted,
          ...(data?.items ?? []).filter((j) => j.id !== accepted.id),
        ],
      }),
    );
  }
  async function changeJob(action: "cancel" | "retry") {
    if (!job || jobAction) return;
    setJobAction(action);
    setError("");
    try {
      const updated =
        action === "cancel"
          ? await client.cancelJob(projectId, job.id)
          : await client.tools.retry(projectId, job.id);
      rememberJob(updated);
      if (action === "retry")
        draft.controller.set((v) => ({
          ...v,
          lastJob: updated.id,
          artifactId: "",
        }));
      void jobs.refetch();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setJobAction(null);
    }
  }
  async function saveWorkset() {
    if (!d.artifactId || !d.worksetName.trim()) return;
    setSavingWorkset(true);
    setError("");
    try {
      const signature = canonical({
        artifactId: d.artifactId,
        name: d.worksetName.trim(),
        filter: d.filter,
      });
      const key =
        d.worksetSubmission?.signature === signature
          ? d.worksetSubmission.key
          : crypto.randomUUID();
      setSaveOperation(key);
      draft.controller.set((v) => ({
        ...v,
        worksetSubmission: { key, signature },
      }));
      await draft.controller.flush();
      const collection = await client.ranking.workset(projectId, d.artifactId, {
        idempotency_key: key,
        name: d.worksetName.trim(),
        filter: d.filter,
      });
      draft.controller.set((v) => ({ ...v, worksetSubmission: null }));
      setSaved(collection);
      setWorksetOpen(false);
      await cache.invalidateQueries({
        queryKey: ["project", projectId, "collections"],
      });
    } catch (e) {
      if (e instanceof Error && "code" in e && e.code === "CANCELLED") {
        draft.controller.set((v) => ({ ...v, worksetSubmission: null }));
        setWorksetOpen(false);
        setError("");
      } else setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSavingWorkset(false);
      setSaveOperation(null);
    }
  }
  const issue = parameterIssue(d.parameters);
  const options = (
    d.scope && !option
      ? [
          {
            value: d.scopeId,
            scope: d.scope,
            count: null,
            label: "已保存的输入范围",
          },
          ...context.inputOptions,
        ]
      : context.inputOptions
  ).map((o) =>
    o.count == null &&
    job?.input_members_frozen &&
    scopeEqual(job.input_scope ?? null, o.scope)
      ? { ...o, count: job.total }
      : o,
  );
  const errorMessage =
    error ||
    artifact.error?.message ||
    summary.error?.message ||
    results.error?.message ||
    jobArtifact.error?.message;
  const available: Pick<Artifact, "id" | "name" | "count" | "created_at">[] = (
    artifacts.data?.items ?? []
  ).map((a) => ({
    id: a.id,
    name: a.name,
    count: a.count ?? null,
    created_at: a.created_at ?? "0",
  }));
  if (
    artifact.data?.state === "ready" &&
    !available.some((a) => a.id === artifact.data!.id)
  )
    available.unshift(artifact.data);
  const filterDescription = [
    d.filter.rating ? `${d.filter.rating.toUpperCase()} 分级` : "全部分级",
    d.filter.eligibility ? eligibilityNames[d.filter.eligibility] : "全部资格",
    d.filter.route ? routeNames[d.filter.route] : null,
    d.filter.selected_only ? "仅已入选" : null,
    d.filter.missing_only ? "仅有字段提示" : null,
    d.filter.top
      ? `${orderLabel(d.filter.order, !!summary.data?.parameters.v2)}每分级前 ${number(d.filter.top)} 名`
      : "不限名次",
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <section className="ranking-view" aria-label="Danbooru 元数据排名">
      <header className="ranking-header">
        <div>
          <h2>
            <Calculator size={18} />
            Danbooru 元数据排名{" "}
            <small>
              {(
                d.tab === "config"
                  ? d.parameters.v2
                  : summary.data?.parameters.v2
              )
                ? "MetaRecall v2 · 元数据模式"
                : "MetaRecall v1"}
            </small>
          </h2>
        </div>
        <span className="grow" />
        {d.tab === "config" && available.length > 0 && (
          <Button
            onClick={() =>
              draft.controller.set((v) => ({
                ...v,
                tab: "results",
                artifactId: v.artifactId || available[0]!.id,
              }))
            }
          >
            查看已有排名
          </Button>
        )}
        {d.tab === "config" ? (
          <Button
            type="submit"
            form={formId}
            className="primary"
            disabled={
              !draft.editable ||
              pending ||
              active ||
              jobs.isPending ||
              jobs.isError ||
              !hasDanbooru ||
              !d.scope ||
              stale ||
              !!issue ||
              option?.count === 0
            }
          >
            <Play size={13} />
            {pending
              ? "正在提交…"
              : active
                ? "排名任务执行中"
                : d.parameters.mode === "rank"
                  ? "计算排名"
                  : "生成候选集"}
          </Button>
        ) : (
          <Button
            onClick={() =>
              draft.controller.set((v) => ({ ...v, tab: "config" }))
            }
          >
            {active ? "查看计算参数" : "配置新任务"}
          </Button>
        )}
      </header>
      <DraftStatus controller={draft.controller} quiet />
      <nav className="ranking-tabs" aria-label="排名视图">
        {(
          [
            ["config", "参数配置"],
            ["results", "结果榜单"],
            ["diagnostics", "统计诊断"],
          ] as const
        ).map(([key, label]) => (
          <button
            key={key}
            className={d.tab === key ? "active" : ""}
            onClick={() => draft.controller.set((v) => ({ ...v, tab: key }))}
            disabled={!draft.editable}
          >
            {label}
          </button>
        ))}
        <span className="grow" />
        <input
          type="search"
          className="ranking-result-search"
          aria-label="搜索排名成果"
          placeholder="搜索排名名称或备注"
          value={rankingList.search}
          maxLength={120}
          onChange={(event) => rankingList.searchFor(event.target.value)}
        />
        <select
          aria-label="排名成果排序"
          value={rankingList.order}
          onChange={(event) =>
            rankingList.sortBy(event.target.value as ObjectListOptions["order"])
          }
        >
          <option value="created_desc">最近创建</option>
          <option value="created_asc">最早创建</option>
          <option value="name_asc">名称升序</option>
          <option value="name_desc">名称降序</option>
        </select>
        <select
          aria-label="查看排名成果"
          value={d.artifactId}
          onChange={(e) =>
            draft.controller.set((v) => ({
              ...v,
              artifactId: e.target.value,
              tab: "results",
            }))
          }
        >
          <option value="">选择已发布成果</option>
          {available.map((a) => (
            <option key={a.id} value={a.id}>
              {a.name} ·{" "}
              {new Date(Number(a.created_at)).toLocaleString("zh-CN")} ·{" "}
              {number(a.count)} 项
            </option>
          ))}
        </select>
        {rankingList.cursor && (
          <Button
            disabled={artifacts.isFetching}
            onClick={rankingList.previous}
          >
            上一组
          </Button>
        )}
        {artifacts.data?.next_cursor && (
          <Button disabled={artifacts.isFetching} onClick={rankingList.next}>
            更多排名
          </Button>
        )}
        {d.artifactId && (
          <MoreMenu
            label="当前排名"
            items={[
              {
                label: "管理与引用关系",
                action: () =>
                  context.management?.open({
                    kind: "artifact",
                    id: d.artifactId,
                  }),
              },
              {
                label: "重命名与备注…",
                action: () =>
                  context.management?.open(
                    { kind: "artifact", id: d.artifactId },
                    "rename",
                  ),
              },
              {
                label: "复用这份成果的参数",
                disabled: !artifact.data?.provenance.run || active,
                action: () => {
                  if (artifact.data?.provenance.run)
                    applyRun(artifact.data.provenance.run);
                },
              },
              {
                label: "删除此排名结果…",
                danger: true,
                action: () =>
                  context.management?.open(
                    { kind: "artifact", id: d.artifactId },
                    "remove",
                  ),
              },
            ]}
          />
        )}
      </nav>
      {artifacts.error && <ErrorDetails error={artifacts.error} compact />}
      {parameterNotice && (
        <p className="ranking-notice" role="status">
          {parameterNotice}
        </p>
      )}
      {(job || trackedJobId || pending || jobs.isError) && (
        <RankingJob
          job={pending ? undefined : job}
          submitting={pending}
          loading={jobs.isPending}
          syncError={jobs.error?.message}
          action={jobAction}
          resultReady={!!jobArtifact.data}
          resultError={jobArtifact.isError}
          onCancel={() => void changeJob("cancel")}
          onRetry={() => void changeJob("retry")}
          onRefresh={() => {
            void jobs.refetch();
            if (job?.status === "succeeded") void jobArtifact.refetch();
          }}
          onResult={() =>
            draft.controller.set((v) => ({
              ...v,
              tab: "results",
              artifactId: jobArtifact.data?.id ?? v.artifactId,
            }))
          }
        />
      )}
      {pending && submissionOperation && (
        <div className="ranking-dialog-actions">
          <span role="status">
            {submissionProgress.data?.state === "cancelling"
              ? "正在取消提交…"
              : submissionProgress.data?.state === "saving"
                ? `正在固定任务成员：${number(submissionProgress.data.completed)} / ${number(submissionProgress.data.total)}`
                : "正在准备任务输入…"}
          </span>
          <Button
            disabled={submissionProgress.data?.state === "cancelling"}
            onClick={() =>
              void client.ranking
                .cancelSave(projectId, submissionOperation)
                .catch((e: unknown) =>
                  setError(e instanceof Error ? e.message : String(e)),
                )
            }
          >
            取消提交
          </Button>
        </div>
      )}
      {errorMessage && (
        <div className="ranking-error">
          <ErrorDetails error={errorMessage} />
          <Button
            onClick={() => {
              setError("");
              void cache.invalidateQueries({
                queryKey: ["project", projectId],
              });
            }}
          >
            <RotateCw size={12} />
            刷新
          </Button>
        </div>
      )}
      {saved && (
        <div className="ranking-saved">
          已保存工作集「{saved.name}」· {number(saved.count)} 项
          <Button
            onClick={() => {
              context.browser.onScope({
                kind: "collection",
                id: saved.id,
                name: saved.name,
              });
              context.activateView("core.browser");
            }}
          >
            打开工作集
          </Button>
        </div>
      )}
      <div
        className="ranking-body"
        style={
          {
            "--ranking-inspector-width": `${Math.min(600, Math.max(260, context.inspector?.width ?? 300))}px`,
          } as CSSProperties
        }
      >
        <div className={"ranking-scroll ranking-content-" + d.tab}>
          {d.tab === "config" && (
            <>
              {!hasDanbooru && (
                <div className="ranking-notice">
                  {requirements.data?.sources
                    .filter((s) => !s.supported)
                    .map((s) => `${s.name}：${s.reason}`)
                    .join("；") || "请选择支持此排名投影的输入范围。"}
                  此工具使用 Danbooru 数据湖，或切换到基础工具。
                </div>
              )}
              {issue && <p className="ranking-notice">{issue}</p>}
              <div className="ranking-presets">
                <PresetControls
                  client={client}
                  projectId={projectId}
                  run={{
                    operator_id: operatorFor(d.parameters),
                    operator_version: 1,
                    parameters_version: 1,
                    parameters: d.parameters,
                  }}
                  onApply={applyRun}
                  disabled={!draft.editable || pending || active}
                />
              </div>
              <RankingConfig
                id={formId}
                parameters={d.parameters}
                v2Saved={d.v2Saved ?? null}
                onChange={changeParameters}
                options={options}
                scopeId={d.scopeId}
                onScope={changeScope}
                disabled={!draft.editable || pending || active || !hasDanbooru}
                onSubmit={(e) => {
                  e.preventDefault();
                  void submit();
                }}
                scopeMessage={
                  stale
                    ? (scopeCheck.error?.message ??
                      "已绑定的范围版本发生变化。")
                    : ""
                }
                onRebind={() => void rebind().catch((e) => setError(String(e)))}
              />
            </>
          )}
          {d.tab !== "config" && !summary.data && (
            <div className="ranking-empty">
              <h3>
                {summary.isFetching
                  ? "正在加载排名成果…"
                  : active || pending
                    ? "排名计算正在进行"
                    : job?.status === "failed"
                      ? "本次排名未完成"
                      : job?.status === "cancelled"
                        ? "本次排名已取消"
                        : job?.status === "succeeded"
                          ? "正在准备排名结果…"
                          : "尚未选择可用的排名成果"}
              </h3>
              <p>
                {active || pending
                  ? "进度显示在上方。任务完成后，榜单与统计诊断会自动载入。"
                  : job?.status === "failed" || job?.status === "cancelled"
                    ? "执行状态与后续操作显示在上方。也可以选择已有成果查看。"
                    : "运行工具后可在这里查看榜单与诊断，也可以选择已有成果。"}
              </p>
              <Button
                onClick={() =>
                  draft.controller.set((v) => ({ ...v, tab: "config" }))
                }
              >
                设置计算参数
              </Button>
            </div>
          )}
          {d.tab === "diagnostics" && summary.data && (
            <RankingDiagnostics
              client={client}
              projectId={projectId}
              artifactId={d.artifactId}
              summary={summary.data}
              onEligibility={(eligibility) => {
                setFilter({
                  ...defaultFilter,
                  rating: null,
                  eligibility,
                  order: "input",
                });
                draft.controller.set((v) => ({ ...v, tab: "results" }));
              }}
            />
          )}
          {d.tab === "results" && summary.data && (
            <>
              <div className="ranking-result-controls">
                <Field label="分级">
                  <select
                    aria-label="分级"
                    value={d.filter.rating ?? ""}
                    onChange={(e) =>
                      setFilter({ ...d.filter, rating: e.target.value || null })
                    }
                  >
                    <option value="">各分级分别列出</option>
                    {["g", "s", "q", "e"].map((r) => (
                      <option key={r} value={r}>
                        {r.toUpperCase()}
                      </option>
                    ))}
                  </select>
                </Field>
                <Field label="查看范围">
                  <select
                    aria-label="查看范围"
                    value={d.filter.eligibility ?? ""}
                    onChange={(e) =>
                      setFilter({
                        ...d.filter,
                        eligibility: (e.target.value ||
                          null) as RankingFilter["eligibility"],
                        selected_only: false,
                        route: null,
                      })
                    }
                  >
                    <option value="">全部输入</option>
                    {Object.entries(eligibilityNames).map(([key, label]) => (
                      <option value={key} key={key}>
                        {label}
                      </option>
                    ))}
                  </select>
                </Field>
                {summary.data.parameters.mode === "select" && (
                  <Field label="入选通道">
                    <select
                      aria-label="入选通道"
                      value={
                        d.filter.selected_only
                          ? "selected"
                          : (d.filter.route ?? "")
                      }
                      onChange={(e) =>
                        setFilter({
                          ...d.filter,
                          selected_only: e.target.value === "selected",
                          route: (["", "selected"].includes(e.target.value)
                            ? null
                            : e.target.value) as RankingFilter["route"],
                        })
                      }
                    >
                      <option value="">全部通道</option>
                      <option value="selected">全部已入选</option>
                      {["main", "rescue", "audit", "budget_rejected"].map(
                        (key) => (
                          <option key={key} value={key}>
                            {routeNames[key as keyof typeof routeNames]}
                          </option>
                        ),
                      )}
                    </select>
                  </Field>
                )}
                <Field label="排序依据">
                  <select
                    aria-label="排序依据"
                    value={d.filter.order ?? "main"}
                    onChange={(e) =>
                      setFilter({
                        ...d.filter,
                        order: e.target.value as RankingFilter["order"],
                      })
                    }
                  >
                    <option value="main">
                      {orderLabel("main", !!summary.data?.parameters.v2)}
                    </option>
                    <option value="rescue">
                      {orderLabel("rescue", !!summary.data?.parameters.v2)}
                    </option>
                    {summary.data?.parameters.v2 && (
                      <>
                        <option value="direct">直算排名</option>
                        <option value="fused">融合排名</option>
                      </>
                    )}
                    <option value="input">输入顺序</option>
                  </select>
                </Field>
                <Field label="每个分级前 N 名">
                  <input
                    aria-label="每个分级前 N 名"
                    type="number"
                    min={1}
                    step={1}
                    placeholder="不限"
                    value={d.filter.top ?? ""}
                    onChange={(e) =>
                      setFilter({
                        ...d.filter,
                        top: e.target.value
                          ? Math.max(1, Math.floor(Number(e.target.value)))
                          : null,
                      })
                    }
                  />
                </Field>
                <span className="grow" />
                <Button
                  onClick={() => setWorksetOpen(true)}
                  disabled={
                    savingWorkset ||
                    count == null ||
                    count === 0 ||
                    results.isFetching ||
                    results.isError
                  }
                >
                  保存筛选为工作集
                </Button>
              </div>
              <div className="ranking-result-description">
                <span>
                  {count === null
                    ? "正在统计匹配数量…"
                    : `匹配 ${number(count)} 项`}{" "}
                  · 每页 48 项 · 分级内排名
                </span>
                <label>
                  <input
                    type="checkbox"
                    checked={d.filter.missing_only ?? false}
                    onChange={(e) =>
                      setFilter({ ...d.filter, missing_only: e.target.checked })
                    }
                  />
                  仅显示有字段提示的图片
                </label>
              </div>
              <div className="ranking-table-scroll">
                <table className="ranking-table" aria-label="元数据排名榜单">
                  <thead>
                    <tr>
                      <th>分级</th>
                      <th>
                        {orderLabel(
                          d.filter.order,
                          !!summary.data?.parameters.v2,
                        )}
                      </th>
                      <th>帖子</th>
                      <th>
                        {summary.data?.parameters.v2 ? "优先级 J" : "主分 S"}
                      </th>
                      <th>
                        {summary.data?.parameters.v2
                          ? "年代相对分"
                          : "补救分 R"}
                      </th>
                      <th>通道 / 资格</th>
                      <th>元数据提示</th>
                      <th />
                    </tr>
                  </thead>
                  <tbody>
                    {results.data?.items.map((row) => (
                      <tr
                        key={row.input.ordinal}
                        onClick={() => focusRow(row)}
                        className={
                          focused?.input.ordinal === row.input.ordinal
                            ? "focused"
                            : ""
                        }
                      >
                        <td>{row.input.rating?.toUpperCase() ?? "未知"}</td>
                        <td>
                          {(d.filter.order === "rescue"
                            ? row.scores.rescue_rank
                            : d.filter.order === "direct"
                              ? row.scores.v2?.direct_rank
                              : d.filter.order === "fused"
                                ? row.scores.v2?.fused_rank
                                : d.filter.order === "input"
                                  ? row.input.ordinal + 1
                                  : row.scores.main_rank) ?? "—"}
                        </td>
                        <td>{row.input.post_id ?? "无对应记录"}</td>
                        <td className="ranking-main-score">
                          {score(row.scores.main_score)}
                        </td>
                        <td>{score(row.scores.rescue_score)}</td>
                        <td>
                          <span
                            className={
                              "ranking-route " + row.scores.selected_route
                            }
                          >
                            {row.scores.eligibility === "eligible"
                              ? routeNames[row.scores.selected_route]
                              : eligibilityNames[row.scores.eligibility]}
                          </span>
                        </td>
                        <td
                          className="ranking-row-flags"
                          title={row.scores.missing_flags
                            .map((f) => flagNames[f] ?? f)
                            .join("、")}
                        >
                          {row.scores.missing_flags
                            .slice(0, 2)
                            .map((f) => flagNames[f] ?? f)
                            .join("、") || "—"}
                          {row.scores.missing_flags.length > 2
                            ? ` +${row.scores.missing_flags.length - 2}`
                            : ""}
                        </td>
                        <td>
                          <Button onClick={() => focusRow(row)}>
                            评分依据
                          </Button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                {(results.isFetching ||
                  results.isPending ||
                  results.data?.preparing) && (
                  <p className="ranking-loading">正在加载榜单…</p>
                )}
                {counting.error && <ErrorDetails error={counting.error} />}
                {!results.isFetching &&
                  !results.isPending &&
                  !results.data?.preparing &&
                  !results.data?.items.length && (
                    <p className="ranking-loading">
                      当前过滤条件没有匹配图片。
                    </p>
                  )}
              </div>
              <div className="ranking-paging">
                <Button
                  disabled={
                    !cursor || results.isFetching || !!results.data?.preparing
                  }
                  onClick={() => {
                    setCursor(null);
                    setHistory([]);
                    setFocused(null);
                  }}
                >
                  首批
                </Button>
                <Button
                  disabled={
                    !history.length ||
                    results.isFetching ||
                    !!results.data?.preparing
                  }
                  onClick={() => {
                    setCursor(history[history.length - 1] ?? null);
                    setHistory((v) => v.slice(0, -1));
                    setFocused(null);
                  }}
                >
                  上一批
                </Button>
                <Button
                  disabled={
                    !results.data?.next_cursor ||
                    results.isFetching ||
                    !!results.data?.preparing
                  }
                  onClick={() => {
                    setHistory((v) => [...v, cursor].slice(-64));
                    setCursor(results.data!.next_cursor!);
                    setFocused(null);
                  }}
                >
                  下一批
                </Button>
                <span className="grow" />
                <span className="subtle">
                  元数据分表示筛选优先级，未入选不等于视觉低质量。
                </span>
              </div>
            </>
          )}
        </div>
        {(context.inspector?.visible ?? true) && (
          <div className="ranking-inspector">
            {context.management?.header}
            {context.management?.tab === "management" ? (
              context.management.content
            ) : focused && summary.data && d.tab === "results" ? (
              <RankingDetails
                artifactId={d.artifactId}
                row={focused}
                summary={summary.data}
                context={context}
                onClose={() => setFocused(null)}
              />
            ) : (
              <RankingOverview
                parameters={d.parameters}
                summary={summary.data}
                scopeName={option?.label ?? "已保存的输入范围"}
                count={
                  option?.count ??
                  (job?.input_members_frozen &&
                  scopeEqual(d.scope, job.input_scope ?? undefined)
                    ? job.total
                    : null)
                }
                configuration={d.tab === "config" || !summary.data}
              />
            )}
            {context.inspector && (
              <ResizeGrip
                label="排名属性面板宽度"
                orientation="vertical"
                reverse
                value={context.inspector.width}
                minimum={260}
                maximum={600}
                onChange={context.inspector.resize}
                onReset={() => context.inspector?.resize(320)}
              />
            )}
          </div>
        )}
      </div>
      {worksetOpen && (
        <Dialog
          title="保存榜单筛选为工作集"
          onClose={() => {
            if (!savingWorkset) setWorksetOpen(false);
          }}
          className="ranking-workset-dialog"
        >
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void saveWorkset();
            }}
          >
            <Field label="工作集名称">
              <input
                aria-label="工作集名称"
                autoFocus
                value={d.worksetName}
                maxLength={100}
                required
                disabled={savingWorkset}
                onChange={(e) =>
                  draft.controller.set((v) => ({
                    ...v,
                    worksetName: e.target.value,
                  }))
                }
              />
            </Field>
            <p>当前榜单条件：{filterDescription}。</p>
            <p>
              将保存全部匹配结果，共 {number(count)} 项，包含尚未加载的页面。
            </p>
            <div className="ranking-dialog-actions">
              {savingWorkset && (
                <span role="status">
                  {saveProgress.data?.state === "cancelling"
                    ? "正在取消…"
                    : `已保存 ${number(saveProgress.data?.completed ?? 0)} / ${number(saveProgress.data?.total ?? count)} 项`}
                </span>
              )}
              <Button
                type="button"
                disabled={
                  savingWorkset &&
                  (!saveOperation || saveProgress.data?.state === "cancelling")
                }
                onClick={() => {
                  if (savingWorkset && saveOperation) {
                    void client.ranking
                      .cancelSave(projectId, saveOperation)
                      .catch((error: unknown) =>
                        setError(
                          error instanceof Error
                            ? error.message
                            : String(error),
                        ),
                      );
                  } else setWorksetOpen(false);
                }}
              >
                {savingWorkset ? "取消保存" : "取消"}
              </Button>
              <Button
                type="submit"
                className="primary"
                disabled={savingWorkset || !d.worksetName.trim()}
              >
                {savingWorkset ? "正在保存…" : "保存工作集"}
              </Button>
            </div>
          </form>
        </Dialog>
      )}
    </section>
  );
}
