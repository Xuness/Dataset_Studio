import { ImagePolicySummary } from "./ImagePolicySummary.js";
import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Database, Plus, RefreshCw } from "lucide-react";
import {
  Button,
  DraftStatus,
  ErrorDetails,
  Workbench,
  WorkbenchDialog,
  WorkbenchPreferences,
  useWorkbenchLayout,
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
  lakeLabel,
  dateLabel,
  policyLabel,
  pendingCount,
  processedCount,
  problemCount,
  rangeLabel,
  sites,
  states,
} from "./model.js";
import type { Schedule } from "./model.js";
import { UpdateComposer } from "./UpdateComposer.js";
import { JobDetails } from "./JobDetails.js";
import { ScopePreparations } from "./ScopePreparations.js";
import { PipelineSettings } from "./PipelineSettings.js";
import "./lake-updates.css";

type View = {
  lakeId: string;
  jobId: string;
  scheduleId: string;
  view: "jobs" | "schedules" | "preparations";
};
const initial: View = { lakeId: "", jobId: "", scheduleId: "", view: "jobs" };
function decode(v: unknown): View | null {
  if (!v || typeof v !== "object") return null;
  const p = v as View;
  return typeof p.lakeId === "string" &&
    typeof p.jobId === "string" &&
    typeof p.scheduleId === "string" &&
    ["jobs", "schedules", "preparations"].includes(p.view)
    ? p
    : null;
}
const layoutDefaults: WorkbenchLayout = {
  panels: { lakes: "left", details: "right" },
  active: {},
  leftWidth: 260,
  rightWidth: 380,
  bottomHeight: 260,
};
export type LakeInvocation = {
  sequence: number;
  lakeId?: string;
  jobId?: string;
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
    [register, setRegister] = useState(false);
  const [pipelineSettings, setPipelineSettings] = useState(false);
  const [error, setError] = useState<unknown>(null),
    [pending, setPending] = useState(false);
  const [target, setTarget] = useState<Schema["RegisterUpdateLake"]>({
    library_id: "",
    site: "danbooru",
    index_root: "",
    media_root: "",
  });
  const configured = !!status.data?.configured;
  const lakes = useQuery({
    queryKey: [...key, "lakes"],
    queryFn: ({ signal }) => client.lakeUpdates.lakes(signal),
    enabled: configured,
    select: (result) => ({
      ...result,
      items: [...result.items].sort(
        (a, b) =>
          Object.keys(sites).indexOf(a.site) -
            Object.keys(sites).indexOf(b.site) || a.id.localeCompare(b.id),
      ),
    }),
  });
  const capabilities = useQuery({
    queryKey: [...key, "capabilities"],
    queryFn: ({ signal }) => client.lakeUpdates.capabilities(signal),
    enabled: configured,
  });
  const jobs = useQuery({
    queryKey: [...key, "jobs", v.lakeId, filter, after.at(-1)],
    queryFn: ({ signal }) =>
      client.lakeUpdates.jobs({
        signal,
        lake_id: v.lakeId || undefined,
        status: filter || undefined,
        after: after.at(-1),
        limit: 50,
      }),
    enabled: configured && v.view === "jobs",
    refetchInterval: 4000,
  });
  const job = useQuery({
    queryKey: [...key, "job", v.jobId],
    queryFn: ({ signal }) => client.lakeUpdates.job(v.jobId, signal),
    enabled: configured && !!v.jobId && v.view === "jobs",
    refetchInterval: (q) =>
      q.state.data &&
      (active(q.state.data) ||
        (q.state.data.cleanup && q.state.data.cleanup.phase !== "complete"))
        ? 2500
        : false,
  });
  const schedules = useQuery({
    queryKey: [...key, "schedules"],
    queryFn: ({ signal }) => client.lakeUpdates.schedules(signal),
    enabled: configured && v.view === "schedules",
    refetchInterval: 10000,
  });
  const lake = lakes.data?.items.find((l) => l.id === v.lakeId),
    schedule = schedules.data?.items.find((s) => s.id === v.scheduleId);
  useEffect(() => {
    if (!state.editable || !invocation) return;
    state.controller.set((old) => ({
      ...old,
      view: invocation.view ?? "jobs",
      lakeId: invocation.lakeId ?? "",
      jobId: invocation.jobId ?? "",
    }));
    setAfter([""]);
  }, [state.editable, state.controller, invocation]);
  function select(values: Partial<View>) {
    state.controller.set({ ...v, ...values });
  }
  const counts = status.data?.activity?.counts ?? [];
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
  const detail =
    v.view === "jobs" && v.jobId ? (
      job.data ? (
        <JobDetails
          key={job.data.id}
          client={client}
          job={job.data}
          onSettings={openSettings}
        />
      ) : (
        <p>
          {job.error ? <ErrorDetails error={job.error} /> : "正在读取任务…"}
        </p>
      )
    ) : v.view === "schedules" && schedule ? (
      <ScheduleDetails
        key={schedule.id}
        schedule={schedule}
        client={client}
        refresh={refresh}
      />
    ) : lake ? (
      <div className="lake-details">
        <details open>
          <summary>{sites[lake.site]}</summary>
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
        <p className="lake-hint">
          更新属于全局数据湖，所有引用此湖的项目共享已发布数据。
        </p>
      </div>
    ) : (
      <p className="lake-empty">选择数据湖或任务查看详情。</p>
    );
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
        layout={layout.value}
        onLayout={layout.update}
        disabled={!layout.editable}
        toolbar={
          <>
            <Button
              className="primary"
              disabled={!configured || !lakes.data?.items.length}
              onClick={() => setCompose(true)}
            >
              <Plus size={14} />
              新建更新
            </Button>
            {v.view === "jobs" &&
              job.data &&
              actions(job.data)
                .filter((a) => ["pause", "resume", "retry"].includes(a))
                .map((action) => (
                  <Button
                    key={action}
                    disabled={pending}
                    onClick={() =>
                      void act(() =>
                        client.lakeUpdates.action(job.data!.id, action),
                      )
                    }
                  >
                    {actionLabels[action]}
                  </Button>
                ))}
            <select
              aria-label="数据湖工作视图"
              value={v.view}
              disabled={!state.editable}
              onChange={(e) => select({ view: e.target.value as View["view"] })}
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
                {Object.entries(states).map(([id, label]) => (
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
                {lakes.data?.items.map((l) => (
                  <button
                    key={l.id}
                    className={v.lakeId === l.id ? "active" : ""}
                    onClick={() => {
                      select({ lakeId: l.id, jobId: "", scheduleId: "" });
                      setAfter([""]);
                    }}
                  >
                    <Database size={14} />
                    <span>{sites[l.site]}</span>
                    <small>
                      {counts
                        .filter(
                          (c) =>
                            c.lake_id === l.id &&
                            ["running", "queued", "waiting_retry"].includes(
                              c.state,
                            ),
                        )
                        .reduce((n, c) => n + c.n, 0)}{" "}
                      活动
                    </small>
                  </button>
                ))}
                {lakes.error && <ErrorDetails error={lakes.error} />}
              </div>
            ),
          },
          { id: "details", title: "详情", content: detail },
        ]}
        status={
          <>
            <span>
              {v.lakeId
                ? lake
                  ? sites[lake.site]
                  : "所选数据湖"
                : "全部数据湖"}{" "}
              · 全局更新
            </span>
            <span className="grow" />
            <WorkbenchPreferences state={layout} />
          </>
        }
      >
        <div className="lake-center">
          {error != null && <ErrorDetails error={error} />}
          {v.view === "preparations" ? (
            <ScopePreparations
              client={client}
              project={project}
              lakes={lakes.data?.items ?? []}
              onUse={() => setCompose(true)}
            />
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
                    {jobs.data?.items.map((j) => (
                      <tr
                        key={j.id}
                        className={v.jobId === j.id ? "selected" : ""}
                        aria-selected={v.jobId === j.id}
                      >
                        <td>
                          <button
                            className="lake-row-link"
                            onClick={() => select({ jobId: j.id })}
                          >
                            {rangeLabel(j.definition)}
                          </button>
                          <small>{policyLabel(j.definition)}</small>
                        </td>
                        <td>{lakeLabel(lakes.data?.items ?? [], j.lake_id)}</td>
                        <td>
                          {states[j.state]}
                          {j.execution_active &&
                            ["paused", "cancelled"].includes(j.state) && (
                              <small>等待当前批次退出</small>
                            )}
                        </td>
                        <td>{processedCount(j).toLocaleString()}</td>
                        <td>{pendingCount(j).toLocaleString()}</td>
                        <td>{problemCount(j).toLocaleString()}</td>
                        <td>{dateLabel(j.created_at)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                {!jobs.isPending && !jobs.data?.items.length && (
                  <p className="lake-empty">
                    没有符合筛选的更新任务。使用“新建更新”选择范围和保存策略。
                  </p>
                )}
              </div>
              <footer className="lake-pagination">
                <span>第 {after.length} 页 · 每页最多 50 条</span>
                <span className="grow" />
                <Button
                  disabled={after.length === 1 || jobs.isFetching}
                  onClick={() => setAfter((v) => v.slice(0, -1))}
                >
                  上一页
                </Button>
                <Button
                  disabled={!jobs.data?.next_cursor || jobs.isFetching}
                  onClick={() =>
                    setAfter((v) => [...v, jobs.data!.next_cursor!])
                  }
                >
                  下一页
                </Button>
              </footer>
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
                    {schedules.data?.items
                      .filter(
                        (s) =>
                          !v.lakeId || s.definition.library_id === v.lakeId,
                      )
                      .map((s) => (
                        <tr
                          key={s.id}
                          className={v.scheduleId === s.id ? "selected" : ""}
                        >
                          <td>
                            <button
                              className="lake-row-link"
                              onClick={() => select({ scheduleId: s.id })}
                            >
                              {rangeLabel(s.definition)}
                            </button>
                          </td>
                          <td>
                            {lakeLabel(
                              lakes.data?.items ?? [],
                              s.definition.library_id,
                            )}
                          </td>
                          <td>
                            {s.every_seconds
                              ? `每 ${s.every_seconds / 3600} 小时`
                              : "单次预约"}
                          </td>
                          <td>{dateLabel(s.next_run_at)}</td>
                          <td>{s.enabled ? "已启用" : "未启用"}</td>
                        </tr>
                      ))}
                  </tbody>
                </table>
                {!schedules.data?.items.length && (
                  <p className="lake-empty">
                    尚无计划。在“新建更新”中选择预约或固定间隔。
                  </p>
                )}
              </div>
            </>
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
          </div>
        </WorkbenchDialog>
      )}
      {compose && (
        <UpdateComposer
          client={client}
          lakes={lakes.data?.items ?? []}
          capabilities={capabilities.data?.items ?? []}
          onClose={() => setCompose(false)}
          onCreated={(id, kind) => {
            select(
              kind === "job"
                ? { view: "jobs", jobId: id, lakeId: "" }
                : { view: "schedules", scheduleId: id, lakeId: "" },
            );
            setAfter([""]);
          }}
        />
      )}
      {register && (
        <WorkbenchDialog
          title="登记可更新的数据湖"
          onClose={() => setRegister(false)}
        >
          <div className="lake-fields">
            <p>选择已经转换好的在线库与图片归档，登记时会核验同一湖身份。</p>
            <label>
              站点
              <select
                value={target.site}
                onChange={(e) =>
                  setTarget({
                    ...target,
                    site: e.target.value as typeof target.site,
                  })
                }
              >
                {Object.entries(sites).map(([id, name]) => (
                  <option key={id} value={id}>
                    {name}
                  </option>
                ))}
              </select>
            </label>
            <label>
              在线库目录
              <input
                value={target.index_root}
                onChange={(e) =>
                  setTarget({ ...target, index_root: e.target.value })
                }
              />
            </label>
            <label>
              图片归档目录
              <input
                value={target.media_root}
                onChange={(e) =>
                  setTarget({ ...target, media_root: e.target.value })
                }
              />
            </label>
            {error != null && <ErrorDetails error={error} />}
            <Button
              disabled={pending || !target.index_root || !target.media_root}
              onClick={() =>
                void act(async () => {
                  await client.lakeUpdates.register(target);
                  setRegister(false);
                })
              }
            >
              核验并登记
            </Button>
          </div>
        </WorkbenchDialog>
      )}
    </section>
  );
}

function ScheduleDetails({
  client,
  schedule: s,
  refresh,
}: {
  client: ApplicationModuleContext["client"];
  schedule: Schedule;
  refresh: () => Promise<unknown>;
}) {
  const [hours, setHours] = useState(String((s.every_seconds ?? 86400) / 3600));
  const [periodic, setPeriodic] = useState(s.every_seconds != null),
    [enabled, setEnabled] = useState(s.enabled);
  const [when, setWhen] = useState(() => {
    const d = new Date(s.next_run_at);
    return new Date(d.getTime() - d.getTimezoneOffset() * 60000)
      .toISOString()
      .slice(0, 16);
  });
  const [revision, setRevision] = useState(s.revision),
    [pending, setPending] = useState(false),
    [error, setError] = useState<unknown>(null),
    [notice, setNotice] = useState("");
  async function save(remove = false) {
    setPending(true);
    setError(null);
    try {
      if (remove) {
        await client.lakeUpdates.removeSchedule(s.id, revision);
        setNotice("计划已移除");
      } else {
        const seconds = periodic ? Number(hours) * 3600 : null;
        if (
          seconds !== null &&
          (!Number.isSafeInteger(seconds) || seconds < 60 || seconds > 31622400)
        )
          throw new Error("间隔需为整数秒，范围 1 分钟至 366 天");
        const r = await client.lakeUpdates.saveSchedule({
          identity: s.id,
          revision,
          spec: s.definition,
          every_seconds: seconds,
          first_run_at: new Date(when).toISOString(),
          enabled,
        });
        setRevision(r.revision);
        setNotice("计划已保存");
      }
      await refresh();
    } catch (e) {
      setError(e);
    } finally {
      setPending(false);
    }
  }
  return (
    <div className="lake-details">
      <details open>
        <summary>计划设置</summary>
        <p>{rangeLabel(s.definition)}</p>
        <p>{policyLabel(s.definition)}</p>
        <ImagePolicySummary policy={s.definition.media} />
        <div className="lake-fields">
          <label className="lake-check">
            <input
              type="checkbox"
              checked={enabled}
              onChange={(e) => setEnabled(e.target.checked)}
            />
            启用计划
          </label>
          <label>
            下一次执行（本机时间）
            <input
              type="datetime-local"
              value={when}
              onChange={(e) => setWhen(e.target.value)}
            />
          </label>
          <label className="lake-check">
            <input
              type="checkbox"
              checked={periodic}
              onChange={(e) => setPeriodic(e.target.checked)}
            />
            按固定间隔重复
          </label>
          {periodic && (
            <label>
              间隔（小时）
              <input value={hours} onChange={(e) => setHours(e.target.value)} />
            </label>
          )}
          <p className="lake-hint">
            需要运行器保持运行。固定日期范围会重复执行同一范围；漏跑周期合并到最近一次。
          </p>
          <div className="lake-actions">
            <Button disabled={pending} onClick={() => void save()}>
              保存计划
            </Button>
            <Button
              disabled={pending}
              onClick={() => {
                if (window.confirm("移除此定时计划？已经创建的任务不受影响。"))
                  void save(true);
              }}
            >
              移除计划
            </Button>
          </div>
        </div>
        {error != null && <ErrorDetails error={error} />}
        {notice && <p role="status">{notice}</p>}
        <small>
          编辑修订 {revision} · 当前修订 {s.revision}
        </small>
      </details>
    </div>
  );
}
