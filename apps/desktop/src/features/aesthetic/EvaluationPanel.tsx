import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { imageInputLabel } from "./ImageInputSettings.js";
import type { Schema } from "@studio/contracts";
import {
  ErrorDetails,
  useDraft,
  DraftStatus,
  Workbench,
  WorkbenchPanelPortal,
  WorkbenchPreferences,
  useWorkbenchLayout,
  WorkbenchDialog,
} from "@studio/ui";
import type { ModuleContext, WorkbenchLayout } from "@studio/ui";
import {
  BatchDetail,
  CandidateCard,
  EvidenceViewer,
  stateLabel,
} from "./Evidence.js";
import { StageCreationDialog } from "./StageCreationDialog.js";
import { CandidateQueue } from "./CandidateQueue.js";
import "./aesthetic.css";
import { SamplingPanel } from "./SamplingPanel.js";
import { ExecutionSettingsDialog } from "./ExecutionSettings.js";
import { BatchRecoveryDialog } from "./BatchRecoveryDialog.js";
import { FitDialog } from "./AnalysisActions.js";
import { aestheticTime } from "./analysisPresentation.js";
import { UsageSummary } from "./UsageSummary.js";

const evaluationLayout: WorkbenchLayout = {
  panels: {
    stages: "left",
    "stage-status": "right",
    "stage-config": "right",
    "candidate-decision": "right",
  },
  active: {},
  leftWidth: 220,
  rightWidth: 380,
  bottomHeight: 240,
};
const sessionInitial = {
  selectedId: "",
  view: "batches" as "batches" | "protected" | "candidates" | "exceptions",
};
function decodeSession(value: unknown): typeof sessionInitial | null {
  if (!value || typeof value !== "object") return null;
  const v = value as typeof sessionInitial;
  return typeof v.selectedId === "string" &&
    ["batches", "protected", "candidates", "exceptions"].includes(v.view)
    ? v
    : null;
}
type Stage = Schema["AestheticStage"];
export default function EvaluationPanel(
  context: ModuleContext & {
    creating: boolean;
    onCloseCreation: () => void;
    onRankings: (stageId: string) => Promise<void>;
    onAnalysisJob: (job: Schema["AestheticAnalysisJob"]) => void;
    toolbarStart: ReactNode;
    openStageId?: string | undefined;
  },
) {
  const panelLayout = useWorkbenchLayout(
    context.client,
    "aesthetic-evaluation",
    evaluationLayout,
  );
  const session = useDraft(
    context.client,
    context.projectId,
    "core.aesthetic",
    sessionInitial,
    decodeSession,
    "evaluation-session",
  );
  const { client, projectId } = context;
  const appliedTarget = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (
      session.editable &&
      context.openStageId &&
      appliedTarget.current !== context.openStageId
    ) {
      appliedTarget.current = context.openStageId;
      session.controller.set({
        selectedId: context.openStageId,
        view: "batches",
      });
    }
  }, [session.editable, session.controller, context.openStageId]);
  const [stages, setStages] = useState<Schema["AestheticStages"] | null>(null);
  const [stageAfter, setStageAfter] = useState<string>();
  const [stagePast, setStagePast] = useState<(string | undefined)[]>([]);
  const [archived, setArchived] = useState(false);
  const [search, setSearch] = useState("");
  const [stageState, setStageState] = useState("");
  const selectedId = session.value.selectedId || null;
  const setSelectedId = (id: string | null) =>
    session.controller.set((value) => ({ ...value, selectedId: id ?? "" }));
  const [selected, setSelected] = useState<Stage | null>(null);
  const [batchAfter, setBatchAfter] = useState<string>();
  const [batchPast, setBatchPast] = useState<(string | undefined)[]>([]);
  const [batchState, setBatchState] = useState("");
  const [batchTarget, setBatchTarget] = useState<number>();
  const [batches, setBatches] = useState<Schema["AestheticBatches"] | null>(
    null,
  );
  const [candidates, setCandidates] = useState<
    Schema["AestheticCandidates"] | null
  >(null);
  const [candidateAfter, setCandidateAfter] = useState<string>();
  const view = session.value.view;
  const setView = (view: typeof sessionInitial.view) =>
    session.controller.set((value) => ({ ...value, view }));
  const [metrics, setMetrics] = useState<Schema["AestheticMetrics"] | null>(
    null,
  );
  const [error, setError] = useState<unknown>(null);
  const [refreshError, setRefreshError] = useState<unknown>(null);
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const [revision, setRevision] = useState(0);
  const [dialog, setDialog] = useState<
    "execution" | "retry" | "defer" | "cancel" | "manage" | "fit" | null
  >(null);
  const [stageName, setStageName] = useState("");
  const [cloneStage, setCloneStage] = useState<Stage>();
  const [imageCandidate, setImageCandidate] =
    useState<Schema["AestheticCandidate"]>();
  useEffect(() => {
    const abort = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    async function refresh() {
      try {
        const [list, m, s, b, c] = await Promise.all([
          client.aesthetic.stages(projectId, stageAfter, abort.signal, {
            archived,
            search,
            ...(stageState ? { state: stageState } : {}),
          }),
          client.aesthetic.metrics(projectId, abort.signal),
          selectedId
            ? client.aesthetic.stage(projectId, selectedId, abort.signal)
            : null,
          selectedId && view === "batches"
            ? client.aesthetic.batches(
                projectId,
                selectedId,
                batchAfter,
                abort.signal,
                {
                  ...(batchState ? { state: batchState } : {}),
                  ...(batchTarget ? { sequence: batchTarget } : {}),
                },
              )
            : null,
          selectedId && (view === "candidates" || view === "protected")
            ? client.aesthetic.candidates(
                projectId,
                selectedId,
                view === "protected",
                candidateAfter,
                abort.signal,
              )
            : null,
        ]);
        if (abort.signal.aborted) return;
        setStages(list);
        setMetrics(m);
        setSelected(s);
        setBatches(b);
        setCandidates(c);
        setRefreshError(null);
      } catch (e) {
        if (!abort.signal.aborted) setRefreshError(e);
      }
      if (!abort.signal.aborted)
        timer = setTimeout(() => {
          void refresh();
        }, 2000);
    }
    void refresh();
    return () => {
      abort.abort();
      clearTimeout(timer);
    };
  }, [
    client,
    projectId,
    stageAfter,
    selectedId,
    batchAfter,
    candidateAfter,
    view,
    revision,
    archived,
    search,
    stageState,
    batchState,
    batchTarget,
  ]);

  async function perform(action: () => Promise<unknown>) {
    setBusy(true);
    setError(null);
    setNotice("");
    try {
      await action();
      setRevision((v) => v + 1);
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }
  function select(id: string) {
    setSelectedId(id);
    setSelected(null);
    setBatchAfter(undefined);
    setBatchPast([]);
    setBatchTarget(undefined);
    setCandidateAfter(undefined);
    setError(null);
    setRefreshError(null);
  }
  const active =
    selected &&
    ["preparing", "running", "pausing", "cancelling"].includes(selected.state);
  const canRecover =
    !!selected &&
    !selected.archived &&
    ["ready", "paused", "needs_attention", "failed"].includes(selected.state);
  const canConfigure =
    !!selected &&
    !selected.archived &&
    [
      "ready",
      "paused",
      "needs_attention",
      "failed",
      "completed",
      "completed_with_exclusions",
    ].includes(selected.state);
  const hasIssues =
    !!selected &&
    (selected.progress?.failed ?? selected.invalid + selected.unknown) +
      (selected.progress?.retry_waiting ?? 0) >
      0;
  const stageTree = (
    <div className="evaluation-stage-tree">
      {" "}
      <h3>评审阶段</h3>
      <label className="wb-sr-only" htmlFor="aesthetic-stage-search">
        搜索评审阶段
      </label>
      <input
        id="aesthetic-stage-search"
        aria-label="搜索评审阶段"
        placeholder="搜索阶段名称"
        value={search}
        onChange={(e) => {
          setSearch(e.target.value);
          setStageAfter(undefined);
          setStagePast([]);
        }}
      />
      <label>
        显示
        <select
          aria-label="阶段归档范围"
          value={archived ? "archived" : "active"}
          onChange={(e) => {
            setArchived(e.target.value === "archived");
            setStageAfter(undefined);
            setStagePast([]);
          }}
        >
          <option value="active">未归档</option>
          <option value="archived">已归档</option>
        </select>
      </label>
      <select
        aria-label="阶段状态筛选"
        value={stageState}
        onChange={(e) => {
          setStageState(e.target.value);
          setStageAfter(undefined);
          setStagePast([]);
        }}
      >
        <option value="">全部状态</option>
        {[
          "ready",
          "running",
          "paused",
          "needs_attention",
          "completed",
          "completed_with_exclusions",
          "cancelled",
          "failed",
        ].map((s) => (
          <option key={s} value={s}>
            {stateLabel(s)}
          </option>
        ))}
      </select>
      <div className="aesthetic-stage-list">
        {stages?.items.map((s) => (
          <button
            key={s.id}
            disabled={!session.editable || busy}
            className={s.id === selectedId ? "active" : ""}
            onClick={() => select(s.id)}
          >
            <strong>{s.name}</strong>
            <span>
              {stateLabel(s.state)} · {s.accepted} 批有效
            </span>
            <small>{aestheticTime(s.created_at)}</small>
          </button>
        ))}
      </div>
      {!stages?.items.length && (
        <p>
          {archived || search || stageState
            ? "没有符合筛选条件的阶段。"
            : "尚未创建阶段。"}
        </p>
      )}
      <div className="aesthetic-actions">
        <button
          disabled={!stageAfter}
          onClick={() => {
            setStageAfter(undefined);
            setStagePast([]);
          }}
        >
          首页
        </button>
        <button
          disabled={!stagePast.length}
          onClick={() => {
            setStageAfter(stagePast.at(-1));
            setStagePast((p) => p.slice(0, -1));
          }}
        >
          上一页阶段
        </button>
        <button
          disabled={!stages?.next_cursor}
          onClick={() => {
            setStagePast((p) => [...p, stageAfter].slice(-64));
            setStageAfter(stages?.next_cursor ?? undefined);
          }}
        >
          下一页
        </button>
      </div>
    </div>
  );
  function inspectCandidate() {
    panelLayout.update((old) => {
      const position =
        old.panels["candidate-decision"] &&
        old.panels["candidate-decision"] !== "hidden"
          ? old.panels["candidate-decision"]
          : "right";
      return {
        ...old,
        panels: { ...old.panels, "candidate-decision": position },
        active: { ...old.active, [position]: "candidate-decision" },
      };
    });
  }
  return (
    <>
      {(error ?? refreshError) != null && (
        <ErrorDetails error={error ?? refreshError} />
      )}
      {notice && (
        <p className="aesthetic-notice" role="status">
          {notice}
        </p>
      )}
      <Workbench
        title="评审执行工作台"
        layout={{
          ...panelLayout.value,
          panels: { ...evaluationLayout.panels, ...panelLayout.value.panels },
        }}
        onLayout={panelLayout.update}
        disabled={!panelLayout.editable}
        panels={[
          { id: "stages", title: "评审阶段", content: stageTree },
          {
            id: "candidate-decision",
            title: "候选处置",
            portal: true,
            defaultPosition: "right",
          },
          {
            id: "stage-status",
            title: "阶段状态",
            portal: true,
            defaultPosition: "right",
          },
          {
            id: "stage-config",
            title: "冻结标准",
            portal: true,
            defaultPosition: "right",
          },
        ]}
        toolbar={
          <>
            {context.toolbarStart}
            <span className="grow" />
            <button
              type="button"
              disabled={busy || !selected}
              onClick={() => {
                if (selected)
                  void perform(() => context.onRankings(selected.id));
              }}
            >
              查看排名快照
            </button>
            <button
              type="button"
              disabled={busy || !selected?.accepted}
              onClick={() => setDialog("fit")}
            >
              生成当前证据快照
            </button>
            <button
              type="button"
              onClick={() => setRevision((value) => value + 1)}
            >
              刷新配置
            </button>
            <WorkbenchPreferences state={panelLayout} />
          </>
        }
        status={<DraftStatus controller={session.controller} quiet />}
      >
        {" "}
        <main className="aesthetic-main">
          {view !== "exceptions" && (
            <WorkbenchPanelPortal id="candidate-decision">
              <p className="aesthetic-help">
                打开“异常候选”并选择图片，处理重评或排除。
              </p>
            </WorkbenchPanelPortal>
          )}
          {selected ? (
            <>
              <div className="aesthetic-header">
                <div>
                  <h3>{selected.name}</h3>
                  <span>
                    {stateLabel(selected.state)} ·{" "}
                    {selected.config.model.remote_model_id}
                    {selected.archived && " · 已归档"}
                  </span>
                </div>
                <div className="aesthetic-actions">
                  <button
                    disabled={
                      busy ||
                      selected.archived ||
                      ![
                        "ready",
                        "paused",
                        "needs_attention",
                        "failed",
                      ].includes(selected.state)
                    }
                    onClick={() =>
                      void perform(() =>
                        client.aesthetic.control(
                          projectId,
                          selected.id,
                          "start",
                        ),
                      )
                    }
                  >
                    开始评审
                  </button>
                  <button
                    disabled={
                      busy || !["preparing", "running"].includes(selected.state)
                    }
                    onClick={() =>
                      void perform(() =>
                        client.aesthetic.control(
                          projectId,
                          selected.id,
                          "pause",
                        ),
                      )
                    }
                  >
                    暂停派发
                  </button>
                  <button
                    disabled={
                      busy ||
                      [
                        "completed",
                        "completed_with_exclusions",
                        "cancelled",
                      ].includes(selected.state)
                    }
                    onClick={() => setDialog("cancel")}
                  >
                    结束阶段
                  </button>
                  <button
                    disabled={busy || !canConfigure}
                    onClick={() => setDialog("execution")}
                  >
                    执行设置
                  </button>
                  <button
                    disabled={busy}
                    onClick={() => {
                      setStageName(selected.name);
                      setDialog("manage");
                    }}
                  >
                    管理阶段
                  </button>
                  <button
                    disabled={busy}
                    onClick={() => setCloneStage(selected)}
                  >
                    复制配置
                  </button>
                </div>
              </div>
              <div className="aesthetic-live-progress" role="status">
                {selected.sampling &&
                (selected.progress?.round_planned ?? 0) > 0 ? (
                  <span>
                    第 {selected.sampling.round || 1} 轮 · 计划{" "}
                    {selected.progress?.round_planned ?? "—"} 组 · 未派发{" "}
                    {selected.progress?.round_unclaimed ?? "—"} 组 · 已接受{" "}
                    {selected.progress?.round_accepted ?? 0} 批
                  </span>
                ) : (
                  <span>
                    累计已接受 {selected.accepted} 批有效评审
                    {selected.state === "running"
                      ? " · 正在检查下一步评审计划"
                      : ""}
                  </span>
                )}
                <span>
                  待派发 {selected.progress?.queued ?? 0} · 准备{" "}
                  {selected.progress?.preparing ?? 0} · 在途{" "}
                  {selected.progress?.in_flight ?? 0} · 等待重试{" "}
                  {selected.progress?.retry_waiting ?? 0} · 异常{" "}
                  {selected.progress?.failed ??
                    selected.invalid + selected.unknown}
                </span>
                <span>
                  至少曝光一次{" "}
                  {selected.progress?.exposed_once ?? selected.comparable} /{" "}
                  {selected.eligible} · 最低曝光达标{" "}
                  {selected.progress?.covered ?? 0} / {selected.eligible}
                </span>
                {!!selected.progress?.deferred && (
                  <span>
                    历史暂缓 {selected.progress.deferred} 批（未计入有效评审）
                  </span>
                )}
              </div>
              {(selected.unknown > 0 ||
                selected.invalid > 0 ||
                (selected.progress?.failed ?? 0) > 0) && (
                <div className="aesthetic-notice aesthetic-recovery-notice">
                  <span>
                    已有批次需要处理。未决候选包含正常等待曝光的图片，并不等于异常图片数量。
                  </span>
                  <button
                    onClick={() => {
                      setView("batches");
                      setBatchState("issues");
                      setBatchAfter(undefined);
                      setBatchPast([]);
                      setBatchTarget(undefined);
                    }}
                  >
                    查看异常批次
                  </button>
                  <button
                    disabled={busy || !canRecover || !hasIssues}
                    onClick={() => setDialog("retry")}
                  >
                    批量重试异常批次
                  </button>
                </div>
              )}
              <WorkbenchPanelPortal id="stage-status">
                <div className="evaluation-status-panel">
                  <SamplingPanel
                    context={context}
                    stage={selected}
                    disabled={busy}
                    onChanged={() => setRevision((v) => v + 1)}
                  />
                  <div className="aesthetic-counters">
                    <span>
                      候选冻结{" "}
                      <b>
                        {selected.frozen.toLocaleString()} /{" "}
                        {selected.total.toLocaleString()}
                      </b>
                    </span>
                    <span>
                      Rating 可用 <b>{selected.eligible.toLocaleString()}</b>
                    </span>
                    <span>
                      调用尝试{" "}
                      <b>
                        {selected.attempts} /{" "}
                        {selected.sampling?.call_limit ??
                          selected.config.request.max_calls}
                      </b>
                    </span>
                    <span>
                      有效评审 <b>{selected.accepted}</b>
                    </span>
                    <span>
                      可比较候选 <b>{selected.comparable.toLocaleString()}</b>
                    </span>
                    <span>
                      已排除 <b>{selected.excluded.toLocaleString()}</b>
                    </span>
                    <span>
                      待补曝光或处置{" "}
                      <b>{selected.unresolved.toLocaleString()}</b>
                    </span>
                    <span>
                      无效批次 <b>{selected.invalid}</b>
                    </span>
                    <span>
                      结果不明 <b>{selected.unknown}</b>
                    </span>
                    <span>
                      保护候选 <b>{selected.protected}</b>
                    </span>
                  </div>
                  <p className="aesthetic-help">
                    已知用量：输入 {selected.input_tokens.toLocaleString()} /
                    输出 {selected.output_tokens.toLocaleString()} tokens；
                    {selected.usage_unknown}{" "}
                    份返回缺少完整用量。目标曝光次数不代表排名已经稳定。
                  </p>
                  {selected.usage_summary && (
                    <UsageSummary value={selected.usage_summary} />
                  )}
                </div>
              </WorkbenchPanelPortal>
              {selected.error && (
                <p className="aesthetic-notice" role="status">
                  {selected.error}
                </p>
              )}
              <WorkbenchPanelPortal id="stage-config">
                <p className="aesthetic-help">
                  后续调用图片：
                  {imageInputLabel(
                    selected.execution_settings?.policy.image_max_edge,
                  )}
                </p>
                <details className="wb-fold evaluation-config-panel">
                  <summary>固定配置与审美标准</summary>
                  <pre>{JSON.stringify(selected.config, null, 2)}</pre>
                </details>
              </WorkbenchPanelPortal>
              <div className="aesthetic-actions aesthetic-tabs">
                {(
                  [
                    ["batches", "批次与梯队"],
                    ["protected", "保护候选"],
                    ["candidates", "冻结候选"],
                    ["exceptions", "异常候选"],
                  ] as const
                ).map(([key, title]) => (
                  <button
                    key={key}
                    aria-pressed={view === key}
                    disabled={busy}
                    onClick={() => {
                      setView(key);
                      if (key === "exceptions") inspectCandidate();
                      setCandidateAfter(undefined);
                    }}
                  >
                    {title}
                  </button>
                ))}
                <button
                  disabled={busy || !!active}
                  onClick={() =>
                    void perform(() =>
                      client.aesthetic.control(projectId, selected.id, "parse"),
                    )
                  }
                >
                  处理待解析返回
                </button>
              </div>
              {view === "exceptions" ? (
                <CandidateQueue
                  key={selected.id}
                  context={context}
                  stage={selected}
                  onBatch={(sequence) => {
                    setView("batches");
                    setBatchTarget(sequence);
                    setBatchState("");
                    setBatchAfter(undefined);
                    setBatchPast([]);
                  }}
                  onInspect={inspectCandidate}
                  onBusy={setBusy}
                  onChanged={() => setRevision((v) => v + 1)}
                />
              ) : view === "batches" ? (
                <>
                  <div className="aesthetic-actions">
                    <label>
                      批次状态
                      <select
                        aria-label="批次状态"
                        value={batchState}
                        onChange={(e) => {
                          setBatchState(e.target.value);
                          setBatchAfter(undefined);
                          setBatchPast([]);
                          setBatchTarget(undefined);
                        }}
                      >
                        <option value="">全部</option>
                        {[
                          ["issues", "仅异常与重试"],
                          ["accepted", "有效评审"],
                          ["sent", "等待模型"],
                          ["retry_wait", "等待自动重试"],
                          ["queued", "等待派发"],
                          ["deferred", "已暂缓"],
                        ].map(([key, label]) => (
                          <option key={key} value={key}>
                            {label}
                          </option>
                        ))}
                      </select>
                    </label>
                    {batchTarget != null && (
                      <button onClick={() => setBatchTarget(undefined)}>
                        显示全部批次
                      </button>
                    )}
                    <button
                      disabled={busy || !canRecover || !hasIssues}
                      onClick={() => setDialog("retry")}
                    >
                      批量重试
                    </button>
                    <button
                      disabled={busy || !canRecover || !hasIssues}
                      onClick={() => setDialog("defer")}
                    >
                      暂缓异常批次
                    </button>
                  </div>
                  {batches?.items.map((batch) => (
                    <BatchDetail
                      key={batch.sequence}
                      context={context}
                      stage={selected}
                      batch={batch}
                      disabled={busy || !!active}
                      perform={perform}
                      defaultOpen={batchTarget === batch.sequence}
                    />
                  ))}
                  {!batches?.items.length && (
                    <p>
                      开始评审后，这里会记录图片展示顺序、梯队结果和调用尝试。
                    </p>
                  )}
                  <div className="aesthetic-actions">
                    <button
                      disabled={!batchAfter}
                      onClick={() => {
                        setBatchAfter(undefined);
                        setBatchPast([]);
                      }}
                    >
                      批次首页
                    </button>
                    <button
                      disabled={!batchPast.length}
                      onClick={() => {
                        setBatchAfter(batchPast.at(-1));
                        setBatchPast((p) => p.slice(0, -1));
                      }}
                    >
                      上一页批次
                    </button>
                    <button
                      disabled={!batches?.next_cursor}
                      onClick={() => {
                        setBatchPast((p) => [...p, batchAfter].slice(-64));
                        setBatchAfter(batches?.next_cursor ?? undefined);
                      }}
                    >
                      下一页批次
                    </button>
                  </div>
                </>
              ) : (
                <>
                  <p>
                    {view === "protected"
                      ? "顶级提名在此受到保护，仍需强模型或人工复核；不会自动加分或成为最终精选。"
                      : "Rating 未知或冲突的图片保留在候选中，不参与混合比较。年份使用来源帖子的创建年份代理。"}
                  </p>
                  <div className="aesthetic-images">
                    {candidates?.items.map((c) => (
                      <CandidateCard
                        key={c.ordinal}
                        context={context}
                        candidate={c}
                        onOpen={() => setImageCandidate(c)}
                      />
                    ))}
                  </div>
                  <div className="aesthetic-actions">
                    <button
                      disabled={!candidateAfter}
                      onClick={() => setCandidateAfter(undefined)}
                    >
                      候选首页
                    </button>
                    <button
                      disabled={!candidates?.next_cursor}
                      onClick={() =>
                        setCandidateAfter(candidates?.next_cursor ?? undefined)
                      }
                    >
                      下一页候选
                    </button>
                  </div>
                </>
              )}
            </>
          ) : (
            <div className="aesthetic-empty">
              <h3>从一个工作集开始</h3>
              <p>
                选择模型和审美标准，冻结配置后启动评审。在左侧选中已有阶段查看批次，或新建一个评审阶段。接受的证据可在“排名浏览”中生成离线快照。
              </p>
            </div>
          )}
          {metrics && (
            <footer>
              全局在途 {metrics.active_requests} · 预留内存{" "}
              {(metrics.reserved_request_bytes / 1048576).toFixed(0)} MiB ·
              上传准入预算 28 Mbps · 待写入{" "}
              {(metrics.queued_write_bytes / 1048576).toFixed(1)} MiB{" "}
              <button
                disabled={busy}
                onClick={() =>
                  void perform(async () => {
                    const result =
                      await client.aesthetic.recoveryPackage(projectId);
                    setNotice(
                      `项目恢复包已保存：${result.relative_path}（包含两库和项目成果，外部数据湖仍需保留）`,
                    );
                  })
                }
              >
                创建项目恢复包
              </button>
            </footer>
          )}
        </main>
      </Workbench>
      {(context.creating || cloneStage) && (
        <StageCreationDialog
          context={context}
          sourceStage={cloneStage}
          onClose={() => {
            setCloneStage(undefined);
            context.onCloseCreation();
          }}
          onCreated={async (stage) => {
            select(stage.id);
            setView("batches");
            setRevision((v) => v + 1);
            setNotice("候选冻结中；准备完成后点击“开始评审”才会调用模型。");
            await session.controller.flush();
          }}
        />
      )}
      {selected && dialog === "execution" && (
        <ExecutionSettingsDialog
          context={context}
          stage={selected}
          onClose={() => setDialog(null)}
          onChanged={() => setRevision((v) => v + 1)}
        />
      )}
      {selected && (dialog === "retry" || dialog === "defer") && (
        <BatchRecoveryDialog
          context={context}
          stage={selected}
          action={dialog}
          onClose={() => setDialog(null)}
          onChanged={(message) => {
            setNotice(message);
            setRevision((v) => v + 1);
          }}
        />
      )}
      {selected && dialog === "fit" && (
        <FitDialog
          context={context}
          initialStageId={selected.id}
          onClose={() => setDialog(null)}
          onCreated={context.onAnalysisJob}
        />
      )}
      {selected && dialog === "cancel" && (
        <WorkbenchDialog
          title="结束评审阶段"
          onClose={() => {
            if (!busy) setDialog(null);
          }}
        >
          <p>
            结束“{selected.name}
            ”后不能继续派发。已接受证据和调用历史会保留，仍可生成排名快照。
          </p>
          <p>需要稍后继续当前阶段时，请使用暂停派发。</p>
          {error != null && <ErrorDetails error={error} />}
          <div className="wb-dialog-actions">
            <button disabled={busy} onClick={() => setDialog(null)}>
              返回
            </button>
            <button
              disabled={busy}
              onClick={() => {
                void perform(async () => {
                  await client.aesthetic.control(
                    projectId,
                    selected.id,
                    "cancel",
                  );
                  setDialog(null);
                });
              }}
            >
              确认结束阶段
            </button>
          </div>
        </WorkbenchDialog>
      )}
      {selected && dialog === "manage" && (
        <WorkbenchDialog
          title="管理评审阶段"
          onClose={() => {
            if (!busy) setDialog(null);
          }}
        >
          <form
            className="wb-field-list"
            onSubmit={(e) => {
              e.preventDefault();
              void perform(async () => {
                await client.aesthetic.stageMetadata(projectId, selected.id, {
                  name: stageName,
                  archived: selected.archived,
                });
                setDialog(null);
              });
            }}
          >
            <label>
              阶段名称
              <input
                required
                maxLength={120}
                value={stageName}
                disabled={busy}
                onChange={(e) => setStageName(e.target.value)}
              />
            </label>
            <button type="submit" disabled={busy}>
              保存名称
            </button>
          </form>
          <p>归档只整理阶段列表，保留评审证据、快照和复核记录。</p>
          {error != null && <ErrorDetails error={error} />}
          <button
            disabled={busy || !!active}
            onClick={() => {
              void perform(async () => {
                await client.aesthetic.stageMetadata(projectId, selected.id, {
                  name: selected.name,
                  archived: !selected.archived,
                });
                setDialog(null);
                setSelectedId(null);
                setSelected(null);
              });
            }}
          >
            {selected.archived ? "恢复阶段" : "归档阶段"}
          </button>
        </WorkbenchDialog>
      )}
      {imageCandidate && (
        <EvidenceViewer
          context={context}
          candidates={candidates?.items ?? [imageCandidate]}
          initialOrdinal={imageCandidate.ordinal}
          onClose={() => setImageCandidate(undefined)}
        />
      )}
    </>
  );
}
