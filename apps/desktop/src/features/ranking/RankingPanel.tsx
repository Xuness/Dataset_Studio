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
} from "@studio/contracts";
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
  operatorId,
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
  const submissionLock = useRef(false);
  const [jobAction, setJobAction] = useState<"cancel" | "retry" | null>(null);
  const [error, setError] = useState("");
  const [cursor, setCursor] = useState<string | null>(null);
  const [history, setHistory] = useState<(string | null)[]>([]);
  const [count, setCount] = useState<number | null>(null);
  const [focused, setFocused] = useState<RankingRow | null>(null);
  const [worksetOpen, setWorksetOpen] = useState(false);
  const [savingWorkset, setSavingWorkset] = useState(false);
  const [saved, setSaved] = useState<Collection | null>(null);
  const hasDanbooru = context.sources.some((s) => s.kind === "danbooru");
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
    (j) => j.operator === operatorId && isJobActive(j),
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
    queryFn: ({ signal }) =>
      client.ranking.rows(
        projectId,
        d.artifactId,
        { filter: d.filter, cursor, limit: 48 },
        signal,
      ),
    enabled:
      !!summary.data &&
      d.tab === "results" &&
      d.filterArtifact === d.artifactId,
    retry: false,
    gcTime: 0,
  });
  const artifacts = useQuery({
    queryKey: ["project", projectId, "ranking-artifact-options"],
    queryFn: ({ signal }) =>
      client.tools.artifacts(projectId, { limit: 64, signal }),
    enabled: draft.editable,
    refetchInterval:
      job &&
      ["queued", "preparing", "running", "waiting_input"].includes(job.status)
        ? 5000
        : false,
    select: (data) =>
      data.items.filter(
        (a) => a.kind === "ranking_table" && a.state === "ready",
      ),
  });
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
      invocation.args.operatorId !== operatorId
    )
      return;
    draft.controller.set((v) => ({
      ...v,
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
  }, [results.data]);
  function changeParameters(parameters: RankingParameters) {
    draft.controller.set((v) => ({ ...v, parameters, submission: null }));
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
        operator_id: operatorId,
        operator_version: 1,
        parameters_version: 1,
        parameters: d.parameters,
      };
      const signature = canonical({ run, scope: d.scope });
      const key =
        d.submission?.signature === signature
          ? d.submission.key
          : crypto.randomUUID();
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
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(false);
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
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSavingWorkset(false);
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
  const available: Artifact[] = [...(artifacts.data ?? [])];
  if (artifact.data && !available.some((a) => a.id === artifact.data!.id))
    available.unshift(artifact.data);
  const filterDescription = [
    d.filter.rating ? `${d.filter.rating.toUpperCase()} 分级` : "全部分级",
    d.filter.eligibility ? eligibilityNames[d.filter.eligibility] : "全部资格",
    d.filter.route ? routeNames[d.filter.route] : null,
    d.filter.selected_only ? "仅已入选" : null,
    d.filter.missing_only ? "仅有字段提示" : null,
    d.filter.top
      ? `${d.filter.order === "rescue" ? "补救排名" : "主排名"}每分级前 ${number(d.filter.top)} 名`
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
            Danbooru 元数据排名 <small>MetaRecall v1</small>
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
              {new Date(Number(a.created_at)).toLocaleString("zh-CN")} ·{" "}
              {number(a.count)} 项
            </option>
          ))}
        </select>
      </nav>
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
                  此工具使用 Danbooru 元数据。请在项目中接入 Danbooru
                  数据湖，或切换到基础工具。
                </div>
              )}
              {issue && <p className="ranking-notice">{issue}</p>}
              <RankingConfig
                id={formId}
                parameters={d.parameters}
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
                    <option value="main">主排名</option>
                    <option value="rescue">补救排名</option>
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
                <span>匹配 {number(count)} 项 · 每页 48 项 · 分级内排名</span>
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
                      <th>主排名</th>
                      <th>帖子</th>
                      <th>主分 S</th>
                      <th>补救分 R</th>
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
                        <td>{row.scores.main_rank ?? "—"}</td>
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
                {results.isFetching && (
                  <p className="ranking-loading">正在加载榜单…</p>
                )}
                {!results.isFetching && !results.data?.items.length && (
                  <p className="ranking-loading">当前过滤条件没有匹配图片。</p>
                )}
              </div>
              <div className="ranking-paging">
                <Button
                  disabled={!cursor || results.isFetching}
                  onClick={() => {
                    setCursor(null);
                    setHistory([]);
                    setFocused(null);
                  }}
                >
                  首批
                </Button>
                <Button
                  disabled={!history.length || results.isFetching}
                  onClick={() => {
                    setCursor(history[history.length - 1] ?? null);
                    setHistory((v) => v.slice(0, -1));
                    setFocused(null);
                  }}
                >
                  上一批
                </Button>
                <Button
                  disabled={!results.data?.next_cursor || results.isFetching}
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
            {focused && summary.data && d.tab === "results" ? (
              <RankingDetails
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
              <Button type="button" onClick={() => setWorksetOpen(false)}>
                取消
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
