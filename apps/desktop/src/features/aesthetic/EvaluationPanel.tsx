import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import type { Schema } from "@studio/contracts";
import {
  ErrorDetails,
  useDraft,
  DraftStatus,
  Workbench,
  WorkbenchPanelPortal,
  WorkbenchDialog,
  WorkbenchPreferences,
  useWorkbenchLayout,
} from "@studio/ui";
import type { ModuleContext, WorkbenchLayout } from "@studio/ui";
import { BatchDetail, CandidateCard, stateLabel } from "./Evidence.js";
import "./aesthetic.css";

const initial = {
  name: "美学评审",
  collectionId: "",
  modelId: "",
  promptId: "",
  exposures: 1,
  maxCalls: 100,
  concurrency: 2,
};
function decode(value: unknown): typeof initial | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Record<string, unknown>;
  if (
    ["name", "collectionId", "modelId", "promptId"].some(
      (k) => typeof v[k] !== "string",
    ) ||
    ["exposures", "maxCalls", "concurrency"].some(
      (k) => typeof v[k] !== "number",
    )
  )
    return null;
  return v as typeof initial;
}
const evaluationLayout: WorkbenchLayout = {
  panels: { stages: "left", "stage-status": "right", "stage-config": "right" },
  active: {},
  leftWidth: 220,
  rightWidth: 300,
  bottomHeight: 240,
};
const sessionInitial = {
  selectedId: "",
  view: "batches" as "batches" | "protected" | "candidates",
};
function decodeSession(value: unknown): typeof sessionInitial | null {
  if (!value || typeof value !== "object") return null;
  const v = value as typeof sessionInitial;
  return typeof v.selectedId === "string" &&
    ["batches", "protected", "candidates"].includes(v.view)
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
  const draft = useDraft(
    client,
    projectId,
    "core.aesthetic",
    initial,
    decode,
    "configuration",
  );
  const [models, setModels] = useState<Schema["LlmModel"][]>([]);
  const [prompts, setPrompts] = useState<Schema["LlmSystemPrompt"][]>([]);
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
  const submission = useRef<{ signature: string; key: string } | null>(null);
  const inputs = context.inputOptions.filter(
    (v) => v.scope.target.kind === "workset",
  );

  useEffect(() => {
    const abort = new AbortController();
    void Promise.all([
      client.llm.providers.list(abort.signal).then(async (v) => {
        const lists = await Promise.all(
          v.items
            .filter((p) => p.config.enabled)
            .map((p) => client.llm.models.list(p.id, abort.signal)),
        );
        return lists.flatMap((list) => list.items);
      }),
      client.llm.systemPrompts.list(abort.signal),
    ])
      .then(([m, p]) => {
        if (!abort.signal.aborted) {
          setModels(m);
          setPrompts(p.items);
        }
      })
      .catch((e: unknown) => {
        if (!abort.signal.aborted) setError(e);
      });
    return () => abort.abort();
  }, [client, revision, context.creating]);

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
          selectedId && view !== "batches"
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
  async function create() {
    const v = draft.value;
    const data = {
      name: v.name,
      collection_id: v.collectionId,
      model_id: v.modelId,
      system_prompt_id: v.promptId,
      exposures: v.exposures,
      max_calls: v.maxCalls,
      concurrency: v.concurrency,
      overrides: {},
    };
    const signature = JSON.stringify(data);
    if (submission.current?.signature !== signature)
      submission.current = { signature, key: crypto.randomUUID() };
    const preflight = await client.aesthetic.preflight(projectId, {
      ...data,
      idempotency_key: submission.current.key,
    });
    if (!preflight.admitted)
      throw new Error(preflight.rejection_reason ?? "当前输入未通过预检");
    const value = await client.aesthetic.create(projectId, {
      ...data,
      idempotency_key: submission.current.key,
    });
    submission.current = null;
    select(value.id);
    context.onCloseCreation();
    setNotice("候选冻结中；准备完成后点击“开始评审”才会调用模型。");
  }
  const editable = !busy && draft.editable && session.editable;
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
            disabled={!session.editable}
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
                  ] as const
                ).map(([key, title]) => (
                  <button
                    key={key}
                    aria-pressed={view === key}
                    onClick={() => {
                      setView(key);
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
              {view === "batches" ? (
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
                    const result = await client.aesthetic.backup(projectId);
                    setNotice(
                      `评审账本备份已保存：${result.relative_path}（完整项目备份还需保留项目目录）`,
                    );
                  })
                }
              >
                备份评审账本
              </button>
            </footer>
          )}
        </main>
      </Workbench>
      {context.creating && (
        <WorkbenchDialog title="新建评审阶段" onClose={context.onCloseCreation}>
          {error !== null && <ErrorDetails error={error} />}
          <div className="evaluation-create">
            <form
              className="wb-field-list"
              onInvalidCapture={(event) => {
                if (event.target instanceof HTMLElement) {
                  const group = event.target.closest("details");
                  if (group) group.open = true;
                }
              }}
              onSubmit={(e) => {
                e.preventDefault();
                void perform(create);
              }}
            >
              <details className="wb-fold" open>
                <summary>
                  输入范围{" "}
                  <small>
                    {draft.value.collectionId ? "已选择工作集" : "待选择"}
                  </small>
                </summary>
                <div className="wb-field-list">
                  <label>
                    阶段名称
                    <input
                      value={draft.value.name}
                      disabled={!editable}
                      onChange={(e) =>
                        draft.controller.set({
                          ...draft.value,
                          name: e.target.value,
                        })
                      }
                      required
                      maxLength={120}
                    />
                  </label>
                  <label>
                    候选工作集
                    <select
                      aria-label="候选工作集"
                      value={draft.value.collectionId}
                      disabled={!editable}
                      onChange={(e) =>
                        draft.controller.set({
                          ...draft.value,
                          collectionId: e.target.value,
                        })
                      }
                      required
                    >
                      <option value="">选择已保存的工作集</option>
                      {inputs.map(
                        (item) =>
                          item.scope.target.kind === "workset" && (
                            <option
                              key={item.value}
                              value={item.scope.target.collection_id}
                            >
                              {item.label}
                            </option>
                          ),
                      )}
                    </select>
                  </label>
                </div>
              </details>
              <details className="wb-fold" open>
                <summary>
                  评审标准{" "}
                  <small>
                    {models.find((model) => model.id === draft.value.modelId)
                      ?.config.name ?? "待选择模型"}
                  </small>
                </summary>
                <div className="wb-field-list">
                  <label>
                    评审模型
                    <select
                      aria-label="评审模型"
                      value={draft.value.modelId}
                      disabled={!editable}
                      onChange={(e) =>
                        draft.controller.set({
                          ...draft.value,
                          modelId: e.target.value,
                        })
                      }
                      required
                    >
                      <option value="">选择模型</option>
                      {models.map((m) => (
                        <option key={m.id} value={m.id}>
                          {m.config.name}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label>
                    审美标准（System Prompt）
                    <select
                      aria-label="审美标准（System Prompt）"
                      value={draft.value.promptId}
                      disabled={!editable}
                      onChange={(e) =>
                        draft.controller.set({
                          ...draft.value,
                          promptId: e.target.value,
                        })
                      }
                      required
                    >
                      <option value="">选择已保存的提示词</option>
                      {prompts.map((p) => (
                        <option key={p.id} value={p.id}>
                          {p.config.name}
                        </option>
                      ))}
                    </select>
                  </label>
                  {prompts.length === 0 && (
                    <p>请先在设置中保存审美标准及顶级图片的判断标准。</p>
                  )}
                </div>
              </details>
              <details className="wb-fold" open>
                <summary>
                  执行预算{" "}
                  <small>
                    {draft.value.exposures} 次曝光 · 至多 {draft.value.maxCalls}{" "}
                    次调用
                  </small>
                </summary>
                <div className="wb-field-list">
                  {(
                    [
                      ["exposures", "每图目标有效曝光", 32],
                      ["maxCalls", "调用次数上限", 10000000],
                      ["concurrency", "请求并发上限", 32],
                    ] as const
                  ).map(([key, label, max]) => (
                    <label key={key}>
                      {label}
                      <input
                        type="number"
                        min={1}
                        max={max}
                        required
                        value={draft.value[key]}
                        disabled={!editable}
                        onChange={(e) =>
                          draft.controller.set({
                            ...draft.value,
                            [key]: Number(e.target.value),
                          })
                        }
                      />
                    </label>
                  ))}
                </div>
              </details>
              <p className="aesthetic-help">
                常规每批 16 图，Rating
                独立分组。创建时预检输入和能力，冻结后还需明确点击“开始评审”。
              </p>
              <button type="submit" disabled={!editable}>
                创建并冻结候选
              </button>
              <DraftStatus controller={draft.controller} quiet />
            </form>
          </div>
          <div className="wb-dialog-actions">
            <button
              type="button"
              onClick={() => {
                context.onCloseCreation();
                context.openSettings?.("llm");
              }}
            >
              API 与模型设置
            </button>
            <button
              type="button"
              onClick={() => {
                context.onCloseCreation();
                context.openSettings?.("system-prompts");
              }}
            >
              管理 System Prompt
            </button>
            <button type="button" onClick={context.onCloseCreation}>
              返回工作台
            </button>
          </div>
        </WorkbenchDialog>
      )}
    </>
  );
}
