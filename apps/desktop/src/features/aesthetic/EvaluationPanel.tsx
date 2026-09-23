import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import type { Schema } from "@studio/contracts";
import {
  ErrorDetails,
  useDraft,
  DraftStatus,
  Workbench,
  WorkbenchPanelPortal,
  WorkbenchPreferences,
  useWorkbenchLayout,
} from "@studio/ui";
import type { ModuleContext, WorkbenchLayout } from "@studio/ui";
import { BatchDetail, CandidateCard, stateLabel } from "./Evidence.js";
import { StageCreationDialog } from "./StageCreationDialog.js";
import { CandidateQueue } from "./CandidateQueue.js";
import "./aesthetic.css";

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
    onRankings: () => void;
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
  const selectedId = session.value.selectedId || null;
  const setSelectedId = (id: string | null) =>
    session.controller.set((value) => ({ ...value, selectedId: id ?? "" }));
  const [selected, setSelected] = useState<Stage | null>(null);
  const [batchAfter, setBatchAfter] = useState<string>();
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
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    const abort = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    async function refresh() {
      try {
        const [list, m, s, b, c] = await Promise.all([
          client.aesthetic.stages(projectId, stageAfter, abort.signal),
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
      } catch (e) {
        if (!abort.signal.aborted) setError(e);
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
    setCandidateAfter(undefined);
  }
  const active =
    selected &&
    ["preparing", "running", "pausing", "cancelling"].includes(selected.state);
  const stageTree = (
    <div className="evaluation-stage-tree">
      {" "}
      <h3>评审阶段</h3>
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
          </button>
        ))}
      </div>
      {!stages?.items.length && <p>尚未创建阶段。</p>}
      <div className="aesthetic-actions">
        <button disabled={!stageAfter} onClick={() => setStageAfter(undefined)}>
          首页
        </button>
        <button
          disabled={!stages?.next_cursor}
          onClick={() => setStageAfter(stages?.next_cursor ?? undefined)}
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
      {error !== null && <ErrorDetails error={error} />}
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
            <button type="button" onClick={context.onRankings}>
              查看排名快照
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
                  </span>
                </div>
                <div className="aesthetic-actions">
                  <button
                    disabled={
                      busy ||
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
                    onClick={() =>
                      void perform(() =>
                        client.aesthetic.control(
                          projectId,
                          selected.id,
                          "cancel",
                        ),
                      )
                    }
                  >
                    取消阶段
                  </button>
                </div>
              </div>
              <WorkbenchPanelPortal id="stage-status">
                <div className="evaluation-status-panel">
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
                        {selected.config.request.max_calls}
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
                      未决候选 <b>{selected.unresolved.toLocaleString()}</b>
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
                    份返回缺少完整用量。费用尚未折算，结果不明的请求也可能已计费。目标曝光次数不代表排名已经稳定。
                  </p>
                </div>
              </WorkbenchPanelPortal>
              {selected.error && (
                <p className="aesthetic-notice" role="status">
                  {selected.error}
                </p>
              )}
              <WorkbenchPanelPortal id="stage-config">
                <details className="wb-fold evaluation-config-panel" open>
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
                  onInspect={inspectCandidate}
                  onBusy={setBusy}
                  onChanged={() => setRevision((v) => v + 1)}
                />
              ) : view === "batches" ? (
                <>
                  {batches?.items.map((batch) => (
                    <BatchDetail
                      key={batch.sequence}
                      context={context}
                      stage={selected}
                      batch={batch}
                      disabled={busy || !!active}
                      perform={perform}
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
                      onClick={() => setBatchAfter(undefined)}
                    >
                      批次首页
                    </button>
                    <button
                      disabled={!batches?.next_cursor}
                      onClick={() =>
                        setBatchAfter(batches?.next_cursor ?? undefined)
                      }
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
      {context.creating && (
        <StageCreationDialog
          context={context}
          onClose={context.onCloseCreation}
          onCreated={async (stage) => {
            select(stage.id);
            setView("batches");
            setRevision((v) => v + 1);
            setNotice("候选冻结中；准备完成后点击“开始评审”才会调用模型。");
            await session.controller.flush();
          }}
        />
      )}
    </>
  );
}
