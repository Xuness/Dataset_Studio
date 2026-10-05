import { useEffect, useRef, useState } from "react";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { Database, Plus, RefreshCw } from "lucide-react";
import {
  Button,
  DraftStatus,
  ErrorDetails,
  Workbench,
  WorkbenchDialog,
  WorkbenchPreferences,
  useWorkbenchLayout,
  WorkbenchDialogMode,
} from "@studio/ui";
import type { ApplicationModuleContext, WorkbenchLayout } from "@studio/ui";
import type { Schema } from "@studio/contracts";
import {
  lakeKey,
  useLakePreference,
  useLakeRefresh,
  useLakeStatus,
} from "./queries.js";
import {
  active,
  actions,
  actionLabels,
  dateLabel,
  policyLabel,
  pendingCount,
  processedCount,
  problemCount,
  rangeLabel,
  sites,
  states,
} from "./model.js";
import { imagePolicyLabel } from "./imagePolicy.js";
import {
  collectionAction,
  collectionActions,
  collectionActive,
  collectionRange,
  collectionStates,
  availableCollectionActions,
} from "./collectionModel.js";
import type { CollectionDefinition } from "./collectionModel.js";
import { NewUpdateComposer, updateLakes } from "./NewUpdateComposer.js";
import { JobDetails } from "./JobDetails.js";
import { CollectionJobDetails } from "./CollectionJobDetails.js";
import { ScheduleDetails } from "./ScheduleDetails.js";
import { CollectionScheduleDetails } from "./CollectionScheduleDetails.js";
import { ScopePreparations } from "./ScopePreparations.js";
import { PipelineSettings } from "./PipelineSettings.js";
import { CollectionPipelineSettings } from "./CollectionPipelineSettings.js";
import "./lake-updates.css";

type Family = "update" | "collection";
type View = {
  lakeId: string;
  jobId: string;
  family: Family;
  scheduleId: string;
  view: "jobs" | "schedules" | "preparations";
};
const initial: View = {
  lakeId: "",
  jobId: "",
  family: "update",
  scheduleId: "",
  view: "jobs",
};
function decode(value: unknown): View | null {
  if (!value || typeof value !== "object") return null;
  const v = value as View;
  return typeof v.lakeId === "string" &&
    typeof v.jobId === "string" &&
    typeof v.scheduleId === "string" &&
    ["jobs", "schedules", "preparations"].includes(v.view)
    ? { ...v, family: v.family === "collection" ? "collection" : "update" }
    : null;
}
const layoutDefaults: WorkbenchLayout = {
  panels: { lakes: "left", details: "right" },
  active: {},
  leftWidth: 260,
  rightWidth: 380,
  bottomHeight: 260,
};
const allSites: Record<string, string> = { ...sites, pixiv: "Pixiv" };
const allStates: Record<string, string> = { ...states, ...collectionStates };
export type LakeInvocation = {
  sequence: number;
  lakeId?: string;
  jobId?: string;
  family?: Family;
  view?: "jobs" | "preparations";
};

export default function LakeWorkspace({
  client,
  openSettings,
  invocation,
  project,
}: ApplicationModuleContext & { invocation: LakeInvocation | null }) {
  const status = useLakeStatus(client, true),
    refresh = useLakeRefresh(client),
    key = lakeKey(client);
  const state = useLakePreference(
      client,
      "studio.lake-updates.workspace",
      initial,
      decode,
    ),
    v = state.value;
  const layout = useWorkbenchLayout(client, "lake-updates", layoutDefaults);
  const [after, setAfter] = useState<string[]>([""]),
    [filter, setFilter] = useState("");
  const [compose, setCompose] = useState(false),
    [preset, setPreset] = useState<CollectionDefinition | undefined>(undefined),
    [composeKey, setComposeKey] = useState(0);
  // Width to restore when the composer closes, if it widened the column.
  const widened = useRef<{ from: number; to: number } | null>(null);
  /** The composer is a dock panel, so the task list stays usable beside it. */
  function openComposer(spec?: CollectionDefinition) {
    setPreset(spec);
    setCompose(true);
    setComposeKey((k) => k + 1);
    layout.update((old) => {
      const saved = old.panels.composer;
      const position = saved && saved !== "hidden" ? saved : "right";
      const widen = position === "right" && old.rightWidth < 460;
      if (widen && !widened.current)
        widened.current = { from: old.rightWidth, to: 460 };
      return {
        ...old,
        panels: { ...old.panels, composer: position },
        active: { ...old.active, [position]: "composer" },
        ...(widen ? { rightWidth: 460 } : {}),
      };
    });
  }
  useEffect(() => {
    const restore = widened.current;
    if (compose || !restore) return;
    widened.current = null;
    layout.update((old) =>
      old.rightWidth === restore.to
        ? { ...old, rightWidth: restore.from }
        : old,
    );
  }, [compose]);
  const [register, setRegister] = useState(false),
    [pipelineSettings, setPipelineSettings] = useState(false);
  const [error, setError] = useState<unknown>(null),
    [pending, setPending] = useState(false);
  const [target, setTarget] = useState({
    site: "pixiv",
    index_root: "",
    media_root: "",
    create: true,
    requestKey: crypto.randomUUID(),
  });
  const configured = !!status.data?.configured;
  const lakesQuery = useInfiniteQuery({
    queryKey: [...key, "workspace-lake-pages"],
    initialPageParam: undefined as string | undefined,
    queryFn: ({ signal, pageParam }) =>
      client.sourceCollections.workspaceLakes({
        signal,
        cursor: pageParam,
        limit: 200,
      }),
    getNextPageParam: (result) => result.next_cursor ?? undefined,
    enabled: configured,
  });
  const lakes = lakesQuery.data?.pages.flatMap((p) => p.items) ?? [];
  const oldLakes = updateLakes(lakes);
  const capabilities = useQuery({
    queryKey: [...key, "capabilities"],
    queryFn: ({ signal }) => client.lakeUpdates.capabilities(signal),
    enabled: configured,
  });
  const jobs = useQuery({
    queryKey: [...key, "workspace-jobs", v.lakeId, filter, after.at(-1)],
    queryFn: ({ signal }) =>
      client.sourceCollections.workspaceJobs({
        signal,
        library_id: v.lakeId || undefined,
        state: filter || undefined,
        cursor: after.at(-1) || undefined,
        limit: 50,
      }),
    enabled: configured && v.view === "jobs",
    refetchInterval: 4000,
  });
  const job = useQuery<Schema["LakeWorkspaceJob"]>({
    queryKey: [...key, "workspace-job", v.family, v.jobId],
    queryFn: async ({ signal }) =>
      v.family === "collection"
        ? {
            family: "collection",
            job: await client.sourceCollections.job(v.jobId, signal),
          }
        : {
            family: "update",
            job: await client.lakeUpdates.job(v.jobId, signal),
          },
    enabled: configured && !!v.jobId && v.view === "jobs",
    refetchInterval: (q) =>
      q.state.data &&
      (q.state.data.family === "collection"
        ? collectionActive(q.state.data.job)
        : active(q.state.data.job) ||
          (q.state.data.job.cleanup &&
            q.state.data.job.cleanup.phase !== "complete"))
        ? 2500
        : false,
  });
  const schedules = useQuery({
    queryKey: [...key, "workspace-schedules", v.lakeId, after.at(-1)],
    queryFn: ({ signal }) =>
      client.sourceCollections.workspaceSchedules({
        signal,
        library_id: v.lakeId || undefined,
        cursor: after.at(-1) || undefined,
        limit: 50,
      }),
    enabled: configured && v.view === "schedules",
    refetchInterval: 10000,
  });
  const lake = lakes.find((l) => l.id === v.lakeId),
    schedule = schedules.data?.items.find(
      (s) => s.schedule.id === v.scheduleId,
    );
  useEffect(() => {
    if (!state.editable || !invocation) return;
    state.controller.set((old) => ({
      ...old,
      view: invocation.view ?? "jobs",
      lakeId: invocation.lakeId ?? "",
      jobId: invocation.jobId ?? "",
      family: invocation.family ?? "update",
    }));
    setAfter([""]);
  }, [state.editable, state.controller, invocation]);
  function select(values: Partial<View>) {
    state.controller.set({ ...v, ...values });
  }
  async function act(run: () => Promise<unknown>) {
    setPending(true);
    setError(null);
    try {
      await run();
      await refresh();
    } catch (e) {
      setError(e);
    } finally {
      setPending(false);
    }
  }
  function recheck(spec: CollectionDefinition) {
    openComposer(spec);
  }
  function chooseJob(id: string, family: Family) {
    select({ view: "jobs", jobId: id, family, scheduleId: "" });
  }
  const chosen = job.data;
  const detail =
    v.view === "jobs" && v.jobId ? (
      chosen ? (
        chosen.family === "collection" ? (
          <CollectionJobDetails
            key={chosen.job.id}
            client={client}
            job={chosen.job}
            onSettings={openSettings}
            onRecheck={recheck}
          />
        ) : (
          <JobDetails
            key={chosen.job.id}
            client={client}
            job={chosen.job}
            onSettings={openSettings}
          />
        )
      ) : job.error ? (
        <ErrorDetails error={job.error} />
      ) : (
        <p>正在读取任务…</p>
      )
    ) : v.view === "schedules" && schedule ? (
      schedule.family === "collection" ? (
        <CollectionScheduleDetails
          key={schedule.schedule.id}
          client={client}
          schedule={schedule.schedule}
          onJob={(id) => chooseJob(id, "collection")}
        />
      ) : (
        <ScheduleDetails
          key={schedule.schedule.id}
          client={client}
          schedule={schedule.schedule}
          refresh={refresh}
        />
      )
    ) : lake ? (
      <div className="lake-details">
        <details open>
          <summary>{allSites[lake.site] ?? lake.site}</summary>
          <dl className="wb-property-list">
            <dt>湖身份</dt>
            <dd>{lake.id}</dd>
            <dt>在线目录</dt>
            <dd>{lake.index_root}</dd>
            <dt>图片归档</dt>
            <dd>{lake.media}</dd>
            <dt>登记时间</dt>
            <dd>{dateLabel(lake.registered_at)}</dd>
          </dl>
          <Button onClick={openSettings}>API 与凭据设置</Button>
        </details>
        <p className="lake-hint">所有引用此湖的项目共享已发布数据。</p>
      </div>
    ) : (
      <p className="lake-empty">选择数据湖或任务查看详情。</p>
    );
  const nextCursor =
    v.view === "jobs" ? jobs.data?.next_cursor : schedules.data?.next_cursor;
  const fetching = v.view === "jobs" ? jobs.isFetching : schedules.isFetching;
  return (
    <section className="lake-workspace" aria-label="数据湖更新工作台">
      <h2 className="wb-sr-only">数据湖</h2>
      <DraftStatus controller={state.controller} quiet />
      {status.error ? (
        <ErrorDetails error={status.error} />
      ) : !configured ? (
        <div className="lake-banner">
          {status.isPending ? "正在连接更新服务…" : "更新运行环境尚未配置。"}
          <Button onClick={openSettings}>打开数据湖 API 设置</Button>
        </div>
      ) : !status.data?.worker_recent ? (
        <div className="lake-banner" role="status">
          运行器尚无近期心跳，任务显示为上次记录。
          <Button onClick={() => void refresh()}>重新检查</Button>
        </div>
      ) : null}
      <Workbench
        title="数据湖工作台"
        layout={{
          ...layout.value,
          panels: {
            ...layout.value.panels,
            composer: compose
              ? layout.value.panels.composer &&
                layout.value.panels.composer !== "hidden"
                ? layout.value.panels.composer
                : "right"
              : "hidden",
          },
        }}
        onLayout={(next) => {
          layout.update(next);
          if (compose && next.panels.composer === "hidden") setCompose(false);
        }}
        disabled={!layout.editable}
        toolbar={
          <>
            <Button
              className="primary"
              disabled={!configured || !lakes.length}
              onClick={() => {
                openComposer();
              }}
            >
              <Plus size={14} />
              新建更新
            </Button>
            {v.view === "jobs" &&
              chosen &&
              (chosen.family === "collection"
                ? availableCollectionActions(chosen.job)
                    .filter((a) =>
                      ["pause", "resume", "retry_failed"].includes(a),
                    )
                    .map((a) => (
                      <Button
                        key={a}
                        disabled={pending}
                        onClick={() =>
                          void act(() =>
                            collectionAction(client, chosen.job.id, a),
                          )
                        }
                      >
                        {collectionActions[a]}
                      </Button>
                    ))
                : actions(chosen.job)
                    .filter((a) => ["pause", "resume", "retry"].includes(a))
                    .map((a) => (
                      <Button
                        key={a}
                        disabled={pending}
                        onClick={() =>
                          void act(() =>
                            client.lakeUpdates.action(chosen.job.id, a),
                          )
                        }
                      >
                        {actionLabels[a]}
                      </Button>
                    )))}
            <select
              aria-label="数据湖工作视图"
              value={v.view}
              disabled={!state.editable}
              onChange={(e) => {
                select({
                  view: e.target.value as View["view"],
                  jobId: "",
                  scheduleId: "",
                });
                setAfter([""]);
              }}
            >
              <option value="jobs">更新任务</option>
              <option value="schedules">定时计划</option>
              <option value="preparations">项目范围准备</option>
            </select>
            {v.view === "jobs" && (
              <select
                aria-label="任务状态筛选"
                value={filter}
                onChange={(e) => {
                  setFilter(e.target.value);
                  setAfter([""]);
                }}
              >
                <option value="">全部状态</option>
                <option value="active">活动任务</option>
                <option value="attention">需要处理</option>
                {Object.entries(allStates).map(([id, label]) => (
                  <option key={id} value={id}>
                    {label}
                  </option>
                ))}
              </select>
            )}
            <span className="grow" />
            <Button onClick={() => setPipelineSettings(true)}>
              下载与调度
            </Button>
            <Button onClick={openSettings}>API 设置</Button>
            <Button disabled={!configured} onClick={() => setRegister(true)}>
              登记数据湖
            </Button>
            <Button aria-label="刷新数据湖状态" onClick={() => void refresh()}>
              <RefreshCw size={14} />
            </Button>
          </>
        }
        panels={[
          {
            id: "lakes",
            title: "数据湖大纲",
            content: (
              <div className="lake-outline">
                <button
                  className={!v.lakeId ? "active" : ""}
                  onClick={() => {
                    select({ lakeId: "", jobId: "", scheduleId: "" });
                    setAfter([""]);
                  }}
                >
                  <Database size={14} />
                  全部数据湖
                </button>
                {lakes.map((l) => (
                  <button
                    key={l.id}
                    className={v.lakeId === l.id ? "active" : ""}
                    title={l.media}
                    onClick={() => {
                      select({ lakeId: l.id, jobId: "", scheduleId: "" });
                      setAfter([""]);
                    }}
                  >
                    <Database size={14} />
                    <span>{allSites[l.site] ?? l.site}</span>
                    <small>{l.id.slice(0, 8)}</small>
                  </button>
                ))}
                {lakesQuery.hasNextPage && (
                  <Button
                    disabled={lakesQuery.isFetchingNextPage}
                    onClick={() => void lakesQuery.fetchNextPage()}
                  >
                    更多数据湖
                  </Button>
                )}
                {lakesQuery.error && <ErrorDetails error={lakesQuery.error} />}
              </div>
            ),
          },
          { id: "details", title: "详情", content: detail },
          ...(compose
            ? [
                {
                  id: "composer",
                  title: "新建更新",
                  icon: <Plus size={13} />,
                  defaultPosition: "right" as const,
                  content: (
                    <WorkbenchDialogMode.Provider value="panel">
                      <NewUpdateComposer
                        key={composeKey}
                        client={client}
                        lakes={lakes}
                        initialLake={v.lakeId}
                        preset={preset}
                        capabilities={capabilities.data?.items ?? []}
                        onClose={() => setCompose(false)}
                        onCreated={(id, kind, family) => {
                          select(
                            kind === "job"
                              ? { view: "jobs", jobId: id, family, lakeId: "" }
                              : {
                                  view: "schedules",
                                  scheduleId: id,
                                  family,
                                  lakeId: "",
                                },
                          );
                          setAfter([""]);
                        }}
                      />
                    </WorkbenchDialogMode.Provider>
                  ),
                },
              ]
            : []),
        ]}
        status={
          <>
            <span>
              {lake ? (allSites[lake.site] ?? lake.site) : "全部数据湖"} ·
              全局更新
            </span>
            <span className="grow" />
            <WorkbenchPreferences state={layout} />
          </>
        }
      >
        <div className="lake-center">
          {error != null && <ErrorDetails error={error} />}
          {v.view === "preparations" ? (
            lake?.site === "pixiv" ? (
              <p className="lake-empty">
                Pixiv 使用作者或作品范围，请在“新建更新”中设置。
              </p>
            ) : (
              <ScopePreparations
                client={client}
                project={project}
                lakes={oldLakes}
                onUse={() => {
                  openComposer();
                }}
              />
            )
          ) : v.view === "jobs" ? (
            <>
              {jobs.error && <ErrorDetails error={jobs.error} />}
              <div className="lake-table-scroll">
                <table className="lake-table">
                  <thead>
                    <tr>
                      <th>任务 / 范围</th>
                      <th>数据湖</th>
                      <th>状态</th>
                      <th>已完成</th>
                      <th>待处理</th>
                      <th>异常 / 未获取</th>
                      <th>创建时间</th>
                    </tr>
                  </thead>
                  <tbody>
                    {jobs.data?.items.map((row) => {
                      const j = row.job,
                        isCollection = row.family === "collection";
                      const id =
                        row.family === "collection"
                          ? row.job.library_id
                          : row.job.lake_id;
                      const targetLake = lakes.find((l) => l.id === id);
                      const done =
                        row.family === "collection"
                          ? `${row.job.progress.works.details} 作品 · ${row.job.progress.media.published + row.job.progress.media.retained} 媒体`
                          : processedCount(row.job).toLocaleString();
                      const left =
                        row.family === "collection"
                          ? row.job.progress.media.planned == null
                            ? "仍在发现"
                            : Math.max(
                                0,
                                row.job.progress.media.planned -
                                  row.job.progress.media.published -
                                  row.job.progress.media.retained -
                                  row.job.progress.media.gaps,
                              ).toLocaleString()
                          : pendingCount(row.job).toLocaleString();
                      return (
                        <tr
                          key={j.id}
                          className={v.jobId === j.id ? "selected" : ""}
                          aria-selected={v.jobId === j.id}
                        >
                          <td>
                            <button
                              className="lake-row-link"
                              onClick={() => chooseJob(j.id, row.family)}
                            >
                              {row.family === "collection"
                                ? collectionRange(row.job.definition)
                                : rangeLabel(row.job.definition)}
                            </button>
                            <small>
                              {row.family === "collection"
                                ? imagePolicyLabel(
                                    row.job.definition.media.image_policy,
                                  )
                                : policyLabel(row.job.definition)}
                            </small>
                          </td>
                          <td>
                            {targetLake
                              ? (allSites[targetLake.site] ?? targetLake.site)
                              : id.slice(0, 8)}
                          </td>
                          <td>
                            {isCollection &&
                            j.state === "completed" &&
                            row.family === "collection" &&
                            row.job.progress.access_mode === "anonymous"
                              ? "公开范围完成"
                              : allStates[j.state]}
                            {j.execution_active &&
                              ["paused", "cancelled"].includes(j.state) && (
                                <small>等待当前批次退出</small>
                              )}
                          </td>
                          <td>{done}</td>
                          <td>{left}</td>
                          <td>
                            {row.family === "collection"
                              ? row.job.progress.task_gaps
                              : problemCount(row.job)}
                          </td>
                          <td>{dateLabel(j.created_at)}</td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
                {!jobs.isPending && !jobs.data?.items.length && (
                  <p className="lake-empty">
                    没有符合筛选的更新任务。使用“新建更新”选择范围和保存策略。
                  </p>
                )}
              </div>
            </>
          ) : (
            <>
              {schedules.error && <ErrorDetails error={schedules.error} />}
              <div className="lake-table-scroll">
                <table className="lake-table">
                  <thead>
                    <tr>
                      <th>更新范围</th>
                      <th>数据湖</th>
                      <th>规则</th>
                      <th>下次执行</th>
                      <th>状态</th>
                    </tr>
                  </thead>
                  <tbody>
                    {schedules.data?.items.map((row) => {
                      const s = row.schedule,
                        targetLake = lakes.find(
                          (l) => l.id === s.definition.library_id,
                        );
                      return (
                        <tr
                          key={s.id}
                          className={v.scheduleId === s.id ? "selected" : ""}
                        >
                          <td>
                            <button
                              className="lake-row-link"
                              onClick={() =>
                                select({ scheduleId: s.id, family: row.family })
                              }
                            >
                              {row.family === "collection"
                                ? collectionRange(row.schedule.definition)
                                : rangeLabel(row.schedule.definition)}
                            </button>
                          </td>
                          <td>
                            {targetLake
                              ? allSites[targetLake.site]
                              : s.definition.library_id.slice(0, 8)}
                          </td>
                          <td>
                            {s.every_seconds
                              ? `每 ${s.every_seconds / 3600} 小时`
                              : "单次预约"}
                          </td>
                          <td>{dateLabel(s.next_run_at)}</td>
                          <td>{s.enabled ? "已启用" : "未启用"}</td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
                {!schedules.isPending && !schedules.data?.items.length && (
                  <p className="lake-empty">
                    尚无计划。在“新建更新”中设置周期。
                  </p>
                )}
              </div>
            </>
          )}
          {v.view !== "preparations" && (
            <footer className="lake-pagination">
              <span>第 {after.length} 页 · 每页最多 50 条</span>
              <span className="grow" />
              <Button
                disabled={after.length === 1 || fetching}
                onClick={() => setAfter((a) => a.slice(0, -1))}
              >
                上一页
              </Button>
              <Button
                disabled={!nextCursor || fetching}
                onClick={() => setAfter((a) => [...a, nextCursor!])}
              >
                下一页
              </Button>
            </footer>
          )}
        </div>
      </Workbench>
      {pipelineSettings && (
        <WorkbenchDialog
          title="下载与调度"
          onClose={() => setPipelineSettings(false)}
        >
          <div className="lake-composer">
            <PipelineSettings client={client} />
            <CollectionPipelineSettings client={client} />
          </div>
        </WorkbenchDialog>
      )}
      {register && (
        <WorkbenchDialog
          title="登记可更新的数据湖"
          onClose={() => setRegister(false)}
        >
          <div className="lake-fields">
            <p>
              {target.site === "pixiv" && target.create
                ? "在两个独立的空目录中创建 Pixiv 数据湖。"
                : "选择已有在线库与图片归档，登记时会核验同一湖身份。"}
            </p>
            <label>
              站点
              <select
                value={target.site}
                onChange={(e) =>
                  setTarget({
                    ...target,
                    site: e.target.value,
                    requestKey: crypto.randomUUID(),
                  })
                }
              >
                {Object.entries(allSites).map(([id, label]) => (
                  <option key={id} value={id}>
                    {label}
                  </option>
                ))}
              </select>
            </label>
            {target.site === "pixiv" && (
              <label className="lake-check">
                <input
                  type="checkbox"
                  checked={target.create}
                  onChange={(e) =>
                    setTarget({
                      ...target,
                      create: e.target.checked,
                      requestKey: crypto.randomUUID(),
                    })
                  }
                />
                创建新的 Pixiv 空湖
              </label>
            )}
            <label>
              在线库目录
              <input
                value={target.index_root}
                onChange={(e) =>
                  setTarget({
                    ...target,
                    index_root: e.target.value,
                    requestKey: crypto.randomUUID(),
                  })
                }
              />
            </label>
            <label>
              图片归档目录
              <input
                value={target.media_root}
                onChange={(e) =>
                  setTarget({
                    ...target,
                    media_root: e.target.value,
                    requestKey: crypto.randomUUID(),
                  })
                }
              />
            </label>
            {error != null && <ErrorDetails error={error} />}
            <Button
              disabled={
                pending ||
                !target.index_root.trim() ||
                !target.media_root.trim()
              }
              onClick={() =>
                void act(async () => {
                  if (target.site === "pixiv") {
                    const args = {
                      request_key: target.requestKey,
                      site: "pixiv",
                      index_root: target.index_root.trim(),
                      media_root: target.media_root.trim(),
                    };
                    if (target.create)
                      await client.sourceCollections.createLake(args);
                    else await client.sourceCollections.registerLake(args);
                  } else
                    await client.lakeUpdates.register({
                      library_id: "",
                      site: target.site as Schema["RegisterUpdateLake"]["site"],
                      index_root: target.index_root.trim(),
                      media_root: target.media_root.trim(),
                    });
                  setRegister(false);
                })
              }
            >
              {target.site === "pixiv" && target.create
                ? "创建 Pixiv 数据湖"
                : "核验并登记"}
            </Button>
          </div>
        </WorkbenchDialog>
      )}
    </section>
  );
}
