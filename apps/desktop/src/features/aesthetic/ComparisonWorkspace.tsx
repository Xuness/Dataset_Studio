import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  ArrowLeftRight,
  GitCompareArrows,
  ListOrdered,
  RefreshCw,
  X,
} from "lucide-react";
import {
  Button,
  DraftStatus,
  ErrorDetails,
  useDraft,
  useDropZone,
  useObjectDragging,
  useWorkbenchLayout,
  Workbench,
  WorkbenchPreferences,
} from "@studio/ui";
import type { ModuleContext, WorkbenchLayout } from "@studio/ui";
import type { Asset, Schema } from "@studio/contracts";
import { AssetImage } from "../browser/AssetImage.js";
import {
  analysisActive,
  analysisState,
  comparisonDeltaLabel as deltaLabel,
} from "./analysisPresentation.js";
import {
  AnalysisJobRow,
  acceptsSnapshot,
  isSnapshot,
  useAnalysisJobEditing,
} from "./AnalysisJobList.js";

const initial = {
  left: "",
  right: "",
  name: "快照对照",
  key: "",
  jobId: "",
  after: "",
  past: [] as string[],
  selected: "",
};
function decode(value: unknown): typeof initial | null {
  if (!value || typeof value !== "object") return null;
  const v = value as typeof initial;
  return [v.left, v.right, v.name, v.key, v.jobId, v.after, v.selected].every(
    (s) => typeof s === "string" && s.length <= 16384,
  ) &&
    Array.isArray(v.past) &&
    v.past.length <= 64 &&
    v.past.every((s) => typeof s === "string" && s.length <= 16384)
    ? v
    : null;
}
const initialLayout: WorkbenchLayout = {
  panels: { comparisons: "left", configuration: "right", detail: "right" },
  active: {},
  leftWidth: 220,
  rightWidth: 380,
  bottomHeight: 260,
};
type Job = Schema["AestheticAnalysisJob"];
type Row = Schema["AestheticComparisonRow"];
type Side = "left" | "right";
const sideLabel = { left: "基准快照 A", right: "对照快照 B" } as const;
const identity = (row: Row) => `${row.key.source_id}:${row.key.asset_id}`;
const percentile = (value: number | null | undefined) =>
  value == null ? "—" : `${(value * 100).toFixed(2)}%`;
function asset(row: Row): Asset {
  return {
    key: row.key,
    name: row.key.asset_id,
    bytes: "0",
    extension: "",
    source_name: "",
    selected: false,
    summary: null,
  };
}
export function ComparisonWorkspace({
  context,
  toolbarStart,
  openComparison,
}: {
  context: ModuleContext;
  toolbarStart: ReactNode;
  /** A full pair starts a new comparison; one side replaces only that side. */
  openComparison?: { left?: string; right?: string } | undefined;
}) {
  const { client, projectId } = context;
  const draft = useDraft(
    client,
    projectId,
    "core.aesthetic",
    initial,
    decode,
    "snapshot-comparison",
  );
  const layout = useWorkbenchLayout(
    client,
    "aesthetic-comparison",
    initialLayout,
  );
  const saved = draft.value;
  const applied = useRef<typeof openComparison>(undefined);
  useEffect(() => {
    if (
      draft.editable &&
      openComparison &&
      applied.current !== openComparison
    ) {
      applied.current = openComparison;
      const { left, right } = openComparison;
      if (left !== undefined && right !== undefined)
        draft.controller.set({ ...initial, left, right });
      else
        draft.controller.set((old) => ({
          ...old,
          ...(left !== undefined ? { left } : {}),
          ...(right !== undefined ? { right } : {}),
          jobId: "",
          key: "",
          after: "",
          past: [],
          selected: "",
        }));
      layout.update((old) => {
        const position =
          old.panels.configuration && old.panels.configuration !== "hidden"
            ? old.panels.configuration
            : "right";
        return {
          ...old,
          panels: { ...old.panels, configuration: position },
          active: { ...old.active, [position]: "configuration" },
        };
      });
    }
  }, [draft.editable, draft.controller, openComparison, layout.update]);
  const [jobsAfter, setJobsAfter] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const lock = useRef(false);
  const edit = useAnalysisJobEditing(context, (removed) =>
    draft.controller.set((old) => ({
      ...old,
      ...(old.left === removed.id ? { left: "" } : {}),
      ...(old.right === removed.id ? { right: "" } : {}),
      ...(old.jobId === removed.id
        ? { jobId: "", key: "", after: "", past: [], selected: "" }
        : {}),
    })),
  );
  const jobs = useQuery({
    queryKey: ["project", projectId, "aesthetic", "analysis-jobs", jobsAfter],
    queryFn: ({ signal }) =>
      client.aesthetic.analysis.jobs(
        projectId,
        { ...(jobsAfter ? { after: jobsAfter } : {}), limit: 32 },
        signal,
      ),
    refetchInterval: (q) =>
      q.state.data?.items.some((j) => analysisActive(j.state)) ? 2000 : false,
  });
  const job = useQuery({
    queryKey: [
      "project",
      projectId,
      "aesthetic",
      "comparison-job",
      saved.jobId,
    ],
    queryFn: ({ signal }) =>
      client.aesthetic.analysis.job(projectId, saved.jobId, signal),
    enabled: !!saved.jobId,
    refetchInterval: (q) =>
      q.state.data && analysisActive(q.state.data.state) ? 1000 : false,
  });
  const left = useQuery({
    queryKey: ["project", projectId, "aesthetic", "snapshot", saved.left],
    queryFn: ({ signal }) =>
      client.aesthetic.analysis.snapshot(projectId, saved.left, signal),
    enabled: !!saved.left,
  });
  const right = useQuery({
    queryKey: ["project", projectId, "aesthetic", "snapshot", saved.right],
    queryFn: ({ signal }) =>
      client.aesthetic.analysis.snapshot(projectId, saved.right, signal),
    enabled: !!saved.right,
  });
  const rows = useQuery({
    queryKey: [
      "project",
      projectId,
      "aesthetic",
      "comparison-rows",
      saved.jobId,
      saved.after,
    ],
    queryFn: ({ signal }) =>
      client.aesthetic.analysis.comparison(
        projectId,
        saved.jobId,
        { ...(saved.after ? { after: saved.after } : {}), limit: 32 },
        signal,
      ),
    enabled: job.data?.state === "completed",
    staleTime: Infinity,
  });
  const selected =
    rows.data?.items.find((r) => identity(r) === saved.selected) ??
    rows.data?.items[0];
  const groups =
    job.data?.result?.kind === "compare" ? job.data.result.groups : [];
  const group = groups.find((g) => g.rating === selected?.rating);
  const snapshots = [
    ...new Map(
      [
        ...(jobs.data?.items.filter(edit.visible) ?? []),
        ...(left.data ? [left.data] : []),
        ...(right.data ? [right.data] : []),
      ]
        .filter(isSnapshot)
        .map((j) => [j.id, j]),
    ).values(),
  ];
  const listed = edit.arrange(jobs.data?.items.filter(edit.visible) ?? []);
  const listedSnapshots = listed.filter(isSnapshot);
  const comparisons = listed.filter((j) => j.request.spec.kind === "compare");
  function assign(side: Side, id: string) {
    if (!draft.editable || busy) return;
    draft.controller.set((old) => ({
      ...old,
      [side]: id,
      jobId: "",
      key: "",
      after: "",
      past: [],
      selected: "",
    }));
  }
  function swap() {
    if (!draft.editable || busy) return;
    draft.controller.set((old) => ({
      ...old,
      left: old.right,
      right: old.left,
      jobId: "",
      key: "",
      after: "",
      past: [],
      selected: "",
    }));
  }
  const slot = (side: Side, variant: "field" | "zone") => (
    <SnapshotSlot
      side={side}
      variant={variant}
      value={saved[side]}
      job={(side === "left" ? left : right).data}
      snapshots={snapshots}
      disabled={busy || !draft.editable}
      onChange={(id) => assign(side, id)}
    />
  );
  const dragging = useObjectDragging();
  async function create() {
    if (
      lock.current ||
      !draft.editable ||
      !saved.left ||
      !saved.right ||
      saved.left === saved.right ||
      !saved.name.trim()
    )
      return;
    lock.current = true;
    setBusy(true);
    setError(null);
    try {
      const key = saved.key || crypto.randomUUID();
      draft.controller.set((old) => ({ ...old, key }));
      await draft.controller.flush();
      const created = await client.aesthetic.analysis.create(projectId, {
        name: saved.name.trim(),
        idempotency_key: key,
        spec: { kind: "compare", left: saved.left, right: saved.right },
      });
      draft.controller.set((old) =>
        old.key === key && old.left === saved.left && old.right === saved.right
          ? {
              ...old,
              jobId: created.id,
              key: "",
              after: "",
              past: [],
              selected: "",
            }
          : old,
      );
      await jobs.refetch();
    } catch (e) {
      setError(e);
    } finally {
      lock.current = false;
      setBusy(false);
    }
  }
  function choose(j: Job) {
    if (j.request.spec.kind !== "compare") return;
    draft.controller.set((old) => ({
      ...old,
      left: j.request.spec.kind === "compare" ? j.request.spec.left : "",
      right: j.request.spec.kind === "compare" ? j.request.spec.right : "",
      jobId: j.id,
      name: j.request.name,
      key: "",
      after: "",
      past: [],
      selected: "",
    }));
  }
  async function control(action: "cancel" | "resume") {
    if (lock.current) return;
    lock.current = true;
    setBusy(true);
    setError(null);
    try {
      await client.aesthetic.analysis.control(projectId, saved.jobId, action);
      await job.refetch();
    } catch (e) {
      setError(e);
    } finally {
      lock.current = false;
      setBusy(false);
    }
  }
  const paging = (
    <div className="aesthetic-actions">
      <button
        type="button"
        disabled={!jobsAfter}
        onClick={() => setJobsAfter(undefined)}
      >
        任务首页
      </button>
      <button
        type="button"
        disabled={!jobs.data?.next_cursor || jobs.isFetching}
        onClick={() => setJobsAfter(jobs.data!.next_cursor!)}
      >
        下一页任务
      </button>
    </div>
  );
  const outline = (
    <div className="ranking-outline">
      <details className="wb-fold" open>
        <summary>
          排名快照 <small>拖到 A / B 对比区</small>
        </summary>
        <div className="ranking-snapshot-list">
          {listedSnapshots.map((j) => (
            <AnalysisJobRow
              key={j.id}
              job={j}
              siblings={listedSnapshots}
              edit={edit}
              list="snapshots"
              icon={<ListOrdered size={15} />}
              badge={
                j.id === saved.left
                  ? "A"
                  : j.id === saved.right
                    ? "B"
                    : undefined
              }
              pressed={j.id === saved.left || j.id === saved.right}
              disabled={busy || !draft.editable}
              onOpen={() => assign(saved.left ? "right" : "left", j.id)}
              actions={[
                {
                  label: "设为基准 A",
                  checked: j.id === saved.left,
                  action: () => assign("left", j.id),
                },
                {
                  label: "设为对照 B",
                  checked: j.id === saved.right,
                  action: () => assign("right", j.id),
                },
              ]}
            />
          ))}
        </div>
      </details>
      <details className="wb-fold" open>
        <summary>
          对照记录 <small>当前任务页</small>
        </summary>
        <div className="ranking-snapshot-list">
          {comparisons.map((j) => (
            <AnalysisJobRow
              key={j.id}
              job={j}
              siblings={comparisons}
              edit={edit}
              list="comparisons"
              icon={<GitCompareArrows size={15} />}
              detail={analysisState(j.state)}
              pressed={j.id === saved.jobId}
              disabled={busy || !draft.editable}
              onOpen={() => choose(j)}
              actions={[{ label: "打开对照", action: () => choose(j) }]}
            />
          ))}
        </div>
        {paging}
      </details>
    </div>
  );
  const configuration = (
    <div className="comparison-configuration">
      <p className="aesthetic-help">
        从左侧把两份已发布快照拖入 A /
        B，或在下方选择，对照模型、评审标准或估计器产生的结果。
      </p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void create();
        }}
      >
        <fieldset disabled={busy || !draft.editable}>
          <label>
            对照名称
            <input
              aria-label="对照名称"
              maxLength={120}
              required
              value={saved.name}
              onChange={(e) =>
                draft.controller.set((old) => ({
                  ...old,
                  name: e.target.value,
                  key: "",
                }))
              }
            />
          </label>
          {slot("left", "field")}
          <button
            type="button"
            className="comparison-swap"
            disabled={!saved.left && !saved.right}
            onClick={swap}
          >
            <ArrowLeftRight size={13} />
            交换 A / B
          </button>
          {slot("right", "field")}
          <Button
            type="submit"
            className="primary"
            disabled={
              !saved.left ||
              !saved.right ||
              saved.left === saved.right ||
              !saved.name.trim()
            }
          >
            <GitCompareArrows size={15} />
            生成离线对照
          </Button>
        </fieldset>
      </form>
      {saved.left && saved.left === saved.right && (
        <p className="aesthetic-help">请选择两份不同的快照。</p>
      )}
      <p className="aesthetic-help">
        选择器只加载当前任务页；可翻页找更早快照。已选快照会保留。
      </p>
      {paging}
      <SnapshotBasis context={context} label="A · 基准" job={left.data} />
      <SnapshotBasis context={context} label="B · 对照" job={right.data} />
    </div>
  );
  const detail = selected ? (
    <div className="comparison-detail">
      <AssetImage
        client={client}
        projectId={projectId}
        asset={asset(selected)}
        edge={1440}
      />
      <details className="wb-fold" open>
        <summary>这张图片 · {selected.rating.toUpperCase()}</summary>
        <dl className="wb-property-list">
          <dt>A 百分位</dt>
          <dd>{percentile(selected.left_percentile)}</dd>
          <dt>B 百分位</dt>
          <dd>{percentile(selected.right_percentile)}</dd>
          <dt>相对变化</dt>
          <dd>{deltaLabel(selected)}</dd>
          <dt>A / B 提名</dt>
          <dd>
            {selected.left_protected ? "有" : "无"} /{" "}
            {selected.right_protected == null
              ? "未知"
              : selected.right_protected
                ? "有"
                : "无"}
          </dd>
        </dl>
        <code className="ranking-asset-id">{selected.key.asset_id}</code>
      </details>
      {group && (
        <details className="wb-fold" open>
          <summary>{group.rating.toUpperCase()} 类统计</summary>
          {group.comparable ? (
            <dl className="wb-property-list">
              <dt>名次相关</dt>
              <dd>{group.rank_correlation?.toFixed(3) ?? "未知 / 无秩方差"}</dd>
              <dt>平均绝对偏移</dt>
              <dd>
                {group.mean_absolute_percentile_delta == null
                  ? "未知"
                  : `${(group.mean_absolute_percentile_delta * 100).toFixed(2)} 个百分点`}
              </dd>
              <dt title="两侧 Top 20% 图片交集占并集的比例">Top 20% 交并比</dt>
              <dd>{percentile(group.top20_jaccard)}</dd>
              <dt>提名分歧</dt>
              <dd>{group.elite_disagreements} 张</dd>
            </dl>
          ) : (
            <p className="aesthetic-help">{group.reason ?? "不具备可比条件"}</p>
          )}
        </details>
      )}
    </div>
  ) : (
    <p className="aesthetic-help">选择一行图片，查看变化与可比性。</p>
  );
  const errors = [
    draft.error,
    error,
    edit.error,
    jobs.error,
    job.error,
    left.error,
    right.error,
    rows.error,
  ].filter(Boolean);
  return (
    <Workbench
      title="实验对照工作台"
      layout={layout.value}
      onLayout={layout.update}
      disabled={!layout.editable}
      panels={[
        { id: "comparisons", title: "对照记录", content: outline },
        {
          id: "configuration",
          title: "对照设置",
          content: configuration,
          defaultPosition: "right",
        },
        {
          id: "detail",
          title: "图片详情",
          content: detail,
          defaultPosition: "right",
        },
      ]}
      toolbar={
        <>
          {toolbarStart}
          <span className="wb-muted">实验对照</span>
          <span className="grow" />
          <button
            type="button"
            className="icon-button"
            aria-label="刷新对照记录"
            onClick={() => {
              void jobs.refetch();
              if (saved.jobId) void job.refetch();
            }}
          >
            <RefreshCw size={14} />
          </button>
          <WorkbenchPreferences state={layout} />
        </>
      }
      status={
        <>
          <span>离线对照 · 不调用模型</span>
          <span>百分位越小越靠前；仅在可比范围计算变化</span>
          <DraftStatus controller={draft.controller} quiet />
        </>
      }
    >
      {dragging &&
        acceptsSnapshot(dragging) &&
        job.data?.state === "completed" && (
          <div className="comparison-drop-overlay">
            {slot("left", "zone")}
            {slot("right", "zone")}
          </div>
        )}
      {errors.length > 0 && <ErrorDetails error={errors[0]} />}
      {job.data && (
        <div className="ranking-task-state">
          <strong>{job.data.request.name}</strong>
          <span>
            {analysisState(job.data.state)} · {job.data.progress} /{" "}
            {job.data.total}
          </span>
          {analysisActive(job.data.state) && (
            <Button disabled={busy} onClick={() => void control("cancel")}>
              取消对照
            </Button>
          )}
          {["failed", "cancelled", "interrupted"].includes(job.data.state) && (
            <Button disabled={busy} onClick={() => void control("resume")}>
              恢复对照
            </Button>
          )}
          {job.data.error && <span>{job.data.error}</span>}
        </div>
      )}
      {job.data?.state === "completed" ? (
        <>
          <div className="comparison-summary" aria-label="对照可比性">
            {groups.map((g) => (
              <span key={g.rating}>
                <b>{g.rating.toUpperCase()}</b> · {g.matched} 张匹配 ·{" "}
                {g.comparable ? "可比较" : (g.reason ?? "不可比较")}
              </span>
            ))}
          </div>
          <div className="comparison-table-scroll">
            <table className="comparison-table">
              <caption className="wb-sr-only">
                快照图片对照；百分位越小越靠前
              </caption>
              <thead>
                <tr>
                  <th>图片</th>
                  <th>Rating</th>
                  <th>A 百分位</th>
                  <th>B 百分位</th>
                  <th>相对变化</th>
                  <th>A / B 提名</th>
                </tr>
              </thead>
              <tbody>
                {rows.data?.items.map((r) => (
                  <tr
                    key={identity(r)}
                    aria-selected={identity(r) === identity(selected ?? r)}
                  >
                    <td>
                      <button
                        type="button"
                        className="comparison-select-image"
                        aria-label={`查看对照图片 ${r.position}`}
                        onClick={() => {
                          draft.controller.set((old) => ({
                            ...old,
                            selected: identity(r),
                          }));
                          layout.update((old) => {
                            const position =
                              old.panels.detail &&
                              old.panels.detail !== "hidden"
                                ? old.panels.detail
                                : "right";
                            return {
                              ...old,
                              panels: { ...old.panels, detail: position },
                              active: { ...old.active, [position]: "detail" },
                            };
                          });
                        }}
                      >
                        <AssetImage
                          client={client}
                          projectId={projectId}
                          asset={asset(r)}
                          edge={160}
                        />
                        <span title={r.key.asset_id}>{r.key.asset_id}</span>
                      </button>
                    </td>
                    <td>{r.rating.toUpperCase()}</td>
                    <td>{percentile(r.left_percentile)}</td>
                    <td>{percentile(r.right_percentile)}</td>
                    <td className={r.comparable ? "" : "wb-muted"}>
                      {deltaLabel(r)}
                    </td>
                    <td>
                      {r.left_protected ? "有" : "无"} /{" "}
                      {r.right_protected == null
                        ? "—"
                        : r.right_protected
                          ? "有"
                          : "无"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            {rows.isPending && (
              <p className="aesthetic-help">正在读取对照结果…</p>
            )}
            {rows.data?.items.length === 0 && (
              <p className="aesthetic-help">没有对照图片。</p>
            )}
          </div>
          <div className="ranking-pagebar">
            <span>本页 {rows.data?.items.length ?? 0} 项 · 按基准快照顺序</span>
            <span className="grow" />
            <button
              type="button"
              disabled={!saved.past.length || rows.isFetching}
              onClick={() =>
                draft.controller.set((old) => ({
                  ...old,
                  after: old.past.at(-1)!,
                  past: old.past.slice(0, -1),
                  selected: "",
                }))
              }
            >
              上一页对照
            </button>
            <button
              type="button"
              disabled={!rows.data?.next_cursor || rows.isFetching}
              onClick={() =>
                draft.controller.set((old) => ({
                  ...old,
                  after: rows.data!.next_cursor!,
                  past: [...old.past, old.after].slice(-64),
                  selected: "",
                }))
              }
            >
              下一页对照
            </button>
          </div>
        </>
      ) : (
        <div className="wb-empty">
          <GitCompareArrows size={32} />
          <h3>
            {job.data && analysisActive(job.data.state)
              ? "正在生成对照…"
              : "从两份排名快照开始"}
          </h3>
          <p>把左侧快照拖入下方对比区，查看相对位置变化和顶级提名分歧。</p>
          <div className="comparison-slots">
            {slot("left", "zone")}
            <button
              type="button"
              className="icon-button"
              aria-label="交换 A / B"
              disabled={busy || (!saved.left && !saved.right)}
              onClick={swap}
            >
              <ArrowLeftRight size={16} />
            </button>
            {slot("right", "zone")}
          </div>
          <Button
            className="primary"
            disabled={
              busy ||
              !saved.left ||
              !saved.right ||
              saved.left === saved.right ||
              !saved.name.trim() ||
              (!!job.data && analysisActive(job.data.state))
            }
            onClick={() => void create()}
          >
            <GitCompareArrows size={15} />
            生成离线对照
          </Button>
        </div>
      )}
      {edit.dialog}
    </Workbench>
  );
}

/** One side of a comparison: a drop zone for a dragged snapshot plus a picker. */
function SnapshotSlot({
  side,
  variant,
  value,
  job,
  snapshots,
  disabled,
  onChange,
}: {
  side: Side;
  variant: "field" | "zone";
  value: string;
  job: Job | undefined;
  snapshots: Job[];
  disabled: boolean;
  onChange: (id: string) => void;
}) {
  const label = sideLabel[side];
  const drop = useDropZone(
    acceptsSnapshot,
    (object) => onChange(object.id),
    disabled,
  );
  return (
    <div
      {...drop.props}
      className={"comparison-slot comparison-slot-" + variant}
      data-side={side}
      data-filled={value ? true : undefined}
      data-active={drop.active || undefined}
      data-over={drop.over || undefined}
    >
      <span className="comparison-slot-label">
        {side === "left" ? "A · 基准" : "B · 对照"}
      </span>
      {variant === "zone" ? (
        <strong>
          {drop.over
            ? `放入“${drop.dragging?.label}”`
            : value
              ? (job?.request.name ?? "读取中…")
              : "拖入排名快照"}
        </strong>
      ) : (
        <select
          aria-label={label}
          value={value}
          required
          disabled={disabled}
          onChange={(e) => onChange(e.target.value)}
        >
          <option value="">
            {drop.active ? "松开以放入…" : "选择或拖入快照…"}
          </option>
          {value && !snapshots.some((j) => j.id === value) && (
            <option value={value}>{job?.request.name ?? value}</option>
          )}
          {snapshots.map((j) => (
            <option key={j.id} value={j.id}>
              {j.request.name}
            </option>
          ))}
        </select>
      )}
      {value && (
        <button
          type="button"
          className="icon-button"
          aria-label={"清除" + label}
          disabled={disabled}
          onClick={() => onChange("")}
        >
          <X size={13} />
        </button>
      )}
    </div>
  );
}

function SnapshotBasis({
  context,
  label,
  job,
}: {
  context: ModuleContext;
  label: string;
  job: Job | undefined;
}) {
  const stage = useQuery({
    queryKey: [
      "project",
      context.projectId,
      "aesthetic",
      "snapshot-stage",
      job?.input.stage_id,
    ],
    queryFn: ({ signal }) =>
      context.client.aesthetic.stage(
        context.projectId,
        job!.input.stage_id,
        signal,
      ),
    enabled: !!job,
  });
  if (!job) return null;
  return (
    <details className="wb-fold">
      <summary>{label} · 冻结依据</summary>
      <dl className="wb-property-list">
        <dt>评审阶段</dt>
        <dd>{stage.data?.name ?? "读取中…"}</dd>
        <dt>模型</dt>
        <dd>{stage.data?.config.model.remote_model_id ?? "—"}</dd>
        <dt>标准版本</dt>
        <dd>{stage.data?.config.model.system_prompt_revision ?? "—"}</dd>
        <dt>证据水位</dt>
        <dd>{job.input.evidence_watermark}</dd>
      </dl>
      {stage.error && <ErrorDetails error={stage.error} />}
    </details>
  );
}
