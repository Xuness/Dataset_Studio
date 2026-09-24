import { useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { StudioError } from "@studio/client";
import type { Schema } from "@studio/contracts";
import {
  DraftStatus,
  ErrorDetails,
  useDraft,
  WorkbenchDialog,
} from "@studio/ui";
import type { ModuleContext } from "@studio/ui";

type Pending = {
  request: Schema["AestheticCreate"];
  report: Schema["AestheticPreflight"];
  attempted: boolean;
  rejected?: boolean;
};
const initial = {
  name: "美学评审",
  collectionId: "",
  modelId: "",
  promptId: "",
  exposures: 2,
  samplingMode: "adaptive",
  maxExposures: 8,
  rankTolerance: 10,
  samplingSeed: 17,
  maxCalls: 100,
  concurrency: 2,
  maxRequestMiB: 32,
  pending: null as Pending | null,
};
function decode(value: unknown): typeof initial | null {
  if (!value || typeof value !== "object") return null;
  const v = value as typeof initial;
  if (
    ![v.name, v.collectionId, v.modelId, v.promptId].every(
      (s) => typeof s === "string",
    ) ||
    ![v.exposures, v.maxCalls, v.concurrency].every(Number.isFinite)
  )
    return null;
  const pending = v.pending ?? null;
  if (
    pending &&
    (typeof pending.attempted !== "boolean" ||
      typeof pending.request?.idempotency_key !== "string" ||
      typeof pending.report?.input_version !== "string" ||
      typeof pending.report?.admitted !== "boolean" ||
      !pending.report.capabilities)
  )
    return null;
  return {
    ...initial,
    ...v,
    maxRequestMiB:
      typeof v.maxRequestMiB === "number" &&
      v.maxRequestMiB >= 8 &&
      v.maxRequestMiB <= 48
        ? v.maxRequestMiB
        : 32,
    pending,
  };
}

export function StageCreationDialog({
  context,
  onClose,
  onCreated,
}: {
  context: ModuleContext;
  onClose: () => void;
  onCreated: (stage: Schema["AestheticStage"]) => void | Promise<void>;
}) {
  const { client, projectId } = context;
  const draft = useDraft(
    client,
    projectId,
    "core.aesthetic",
    initial,
    decode,
    "configuration",
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const lock = useRef(false);
  const models = useQuery({
    queryKey: ["aesthetic", "creation-models"],
    queryFn: async ({ signal }) => {
      const providers = await client.llm.providers.list(signal);
      const lists = await Promise.all(
        providers.items
          .filter((p) => p.config.enabled)
          .map((p) => client.llm.models.list(p.id, signal)),
      );
      return lists.flatMap((v) => v.items);
    },
    staleTime: 0,
    refetchOnMount: "always",
  });
  const prompts = useQuery({
    queryKey: ["aesthetic", "creation-prompts"],
    queryFn: ({ signal }) => client.llm.systemPrompts.list(signal),
    staleTime: 0,
    refetchOnMount: "always",
  });
  const value = draft.value;
  const pending = value.pending;
  const editable = draft.editable && !busy && !pending?.attempted;
  const inputs = context.inputOptions.filter(
    (v) => v.scope.target.kind === "workset",
  );
  function edit(patch: Partial<typeof initial>) {
    if (editable)
      draft.controller.set((old) => ({ ...old, ...patch, pending: null }));
    setError(null);
  }
  async function run(action: () => Promise<void>) {
    if (lock.current || !draft.editable) return;
    lock.current = true;
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(e);
    } finally {
      lock.current = false;
      setBusy(false);
    }
  }
  async function preflight() {
    const request: Schema["AestheticCreate"] = {
      idempotency_key: crypto.randomUUID(),
      name: value.name,
      collection_id: value.collectionId,
      model_id: value.modelId,
      system_prompt_id: value.promptId,
      exposures: value.exposures,
      max_calls: value.maxCalls,
      concurrency: value.concurrency,
      max_request_mib: value.maxRequestMiB,
      sampling: {
        mode: value.samplingMode,
        min_exposures: value.exposures,
        max_exposures: value.maxExposures,
        rank_tolerance: value.rankTolerance / 100,
        seed: value.samplingSeed,
      },
      overrides: {},
    };
    // Invalidate an older report even when this check fails.
    draft.controller.set((old) => ({ ...old, pending: null }));
    const report = await client.aesthetic.preflight(projectId, request);
    draft.controller.set((old) => ({
      ...old,
      pending: {
        request: { ...request, expected_input_version: report.input_version },
        report,
        attempted: false,
      },
    }));
  }
  async function create() {
    if (!pending?.report.admitted) return;
    draft.controller.set((old) => ({
      ...old,
      pending: { ...pending, attempted: true, rejected: false },
    }));
    await client.edits.flush(projectId);
    let stage: Schema["AestheticStage"];
    try {
      stage = await client.aesthetic.create(projectId, pending.request);
    } catch (e) {
      if (e instanceof StudioError && e.code === "NOT_FOUND") {
        draft.controller.set((old) => ({
          ...old,
          pending: { ...pending, attempted: true, rejected: true },
        }));
      }
      // These authoritative failures occur before any creation intent is persisted.
      if (
        e instanceof StudioError &&
        [
          "EVALUATION_INPUT_CHANGED",
          "EVALUATION_CAPACITY_EXCEEDED",
          "EVALUATION_SAMPLING_CAPACITY",
          "EVALUATION_EMPTY",
          "INVALID_INPUT",
        ].includes(e.code)
      )
        draft.controller.set((old) => ({ ...old, pending: null }));
      throw e;
    }
    draft.controller.set((old) => ({ ...old, pending: null }));
    await draft.controller.flush();
    await onCreated(stage);
    onClose();
  }
  const report = pending?.report;
  return (
    <WorkbenchDialog
      title="新建评审阶段"
      onClose={() => {
        if (!busy) onClose();
      }}
    >
      <div className="evaluation-create">
        {error !== null && <ErrorDetails error={error} />}
        {(models.error || prompts.error) && (
          <ErrorDetails error={models.error || prompts.error} />
        )}
        <form
          className="wb-field-list"
          onSubmit={(e) => {
            e.preventDefault();
            if (editable) void run(preflight);
          }}
          onInvalidCapture={(e) => {
            if (e.target instanceof HTMLElement) {
              const group = e.target.closest("details");
              if (group) group.open = true;
            }
          }}
        >
          <details className="wb-fold" open>
            <summary>输入范围</summary>
            <div className="wb-field-list">
              <label>
                阶段名称
                <input
                  value={value.name}
                  disabled={!editable}
                  required
                  maxLength={120}
                  onChange={(e) => edit({ name: e.target.value })}
                />
              </label>
              <label>
                候选工作集
                <select
                  aria-label="候选工作集"
                  value={value.collectionId}
                  disabled={!editable}
                  required
                  onChange={(e) => edit({ collectionId: e.target.value })}
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
            <summary>评审标准</summary>
            <div className="wb-field-list">
              <label>
                评审模型
                <select
                  aria-label="评审模型"
                  value={value.modelId}
                  disabled={!editable}
                  required
                  onChange={(e) => edit({ modelId: e.target.value })}
                >
                  <option value="">选择模型</option>
                  {models.data?.map((m) => (
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
                  value={value.promptId}
                  disabled={!editable}
                  required
                  onChange={(e) => edit({ promptId: e.target.value })}
                >
                  <option value="">选择已保存的提示词</option>
                  {prompts.data?.items.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.config.name}
                    </option>
                  ))}
                </select>
              </label>
              {prompts.data?.items.length === 0 && (
                <p>请先在设置中保存审美标准及顶级图片的判断标准。</p>
              )}
            </div>
          </details>
          <details className="wb-fold" open>
            <summary>执行预算</summary>
            <div className="wb-field-list">
              <label>
                采样方式
                <select
                  aria-label="采样方式"
                  disabled={!editable}
                  value={value.samplingMode}
                  onChange={(e) => edit({ samplingMode: e.target.value })}
                >
                  <option value="adaptive">动态分配</option>
                  <option value="balanced">均衡覆盖与连接</option>
                </select>
              </label>
              <p className="aesthetic-help">
                每轮交叉组批后并发评审。动态模式对位次不稳定、对手单一的图片继续加测，稳定图片降低频率。此版支持至多
                10000 图。
              </p>
              {(
                [
                  ["exposures", "每图最低有效曝光", 32],
                  ["maxExposures", "每图有效曝光上限", 32],
                  ["rankTolerance", "位次变化阈值（百分点）", 25],
                  ["samplingSeed", "采样种子", 4294967295],
                  ["maxCalls", "调用次数上限", 10000000],
                  ["concurrency", "请求并发上限", 32],
                ] as const
              )
                .filter(
                  ([key]) =>
                    key !== "rankTolerance" ||
                    value.samplingMode === "adaptive",
                )
                .map(([key, label, max]) => (
                  <label key={key}>
                    {label}
                    <input
                      type="number"
                      min={
                        key === "samplingSeed"
                          ? 0
                          : key === "maxExposures"
                            ? value.exposures
                            : 1
                      }
                      max={max}
                      step={1}
                      required
                      value={value[key]}
                      disabled={!editable}
                      onChange={(e) => edit({ [key]: Number(e.target.value) })}
                    />
                  </label>
                ))}
              <label>
                每批请求体预算
                <select
                  aria-label="每批请求体预算"
                  disabled={!editable}
                  value={value.maxRequestMiB}
                  onChange={(e) =>
                    edit({ maxRequestMiB: Number(e.target.value) })
                  }
                >
                  {[8, 12, 16, 24, 32, 48].map((n) => (
                    <option key={n} value={n}>
                      {n} MiB{n === 32 ? "（默认）" : ""}
                    </option>
                  ))}
                </select>
              </label>
            </div>
          </details>
          <button type="submit" disabled={!editable}>
            预检输入
          </button>
        </form>
        {report && (
          <section className="evaluation-preflight" aria-label="输入预检结果">
            <h4>{report.admitted ? "基础容量预检通过" : "预检未通过"}</h4>
            <dl className="wb-property-list">
              <dt>工作集候选</dt>
              <dd>{report.total.toLocaleString()}</dd>
              {report.available_storage_bytes !== undefined && (
                <>
                  <dt>可用磁盘空间</dt>
                  <dd>
                    {(report.available_storage_bytes / 1073741824).toFixed(1)}{" "}
                    GiB
                  </dd>
                </>
              )}
              {report.minimum_calls_lower_bound !== undefined && (
                <>
                  <dt>基础曝光调用下界</dt>
                  <dd>
                    至少 {report.minimum_calls_lower_bound.toLocaleString()}{" "}
                    次；连接、动态加测、拆批和重评可能增加
                  </dd>
                </>
              )}
              <dt>阶段候选上限</dt>
              <dd>
                {report.capabilities.max_stage_candidates.toLocaleString()}
              </dd>
              <dt>每批目标</dt>
              <dd>至多 {report.capabilities.batch_size} 图，Rating 独立</dd>
              <dt>原图上限</dt>
              <dd>{report.capabilities.max_image_bytes / 1048576} MiB / 图</dd>
              <dt>请求上限</dt>
              <dd>
                {pending?.request.max_request_mib ??
                  (report.capabilities.default_request_bytes ??
                    report.capabilities.max_request_bytes) / 1048576}{" "}
                MiB / 批（本地预算）
              </dd>
              <dt>调用次数上限</dt>
              <dd>{pending.request.max_calls}</dd>
            </dl>
            {!report.admitted && (
              <p role="alert">
                {report.rejection_reason}（{report.rejection_code}）
              </p>
            )}
            <p className="aesthetic-help">
              此预检核对工作集版本及候选数量。图片可读取性、实际编码大小和模型配置在后续准备时验证；通过不代表每张图片均可发送。
            </p>
            <p className="aesthetic-help">
              创建会冻结候选与配置，并逐页检查图片可读取性、格式和内容版本。准备完成后，点击“开始评审”才会调用模型。
            </p>
            <button
              type="button"
              disabled={busy || !draft.editable || !report.admitted}
              onClick={() => void run(create)}
            >
              {pending.attempted ? "恢复此次创建" : "创建并冻结候选"}
            </button>
            {pending.attempted && (
              <div className="evaluation-pending">
                <p>创建结果尚待确认。恢复会沿用同一份请求，可在重载后继续。</p>
                <button
                  type="button"
                  disabled={busy || !draft.editable}
                  onClick={() =>
                    void run(async () => {
                      const stage = await client.aesthetic.stage(
                        projectId,
                        pending.request.idempotency_key,
                      );
                      draft.controller.set((old) => ({
                        ...old,
                        pending: null,
                      }));
                      await draft.controller.flush();
                      await onCreated(stage);
                      onClose();
                    })
                  }
                >
                  查找已创建阶段
                </button>
                <button
                  type="button"
                  disabled={busy || !draft.editable}
                  onClick={() =>
                    void run(async () => {
                      try {
                        await client.aesthetic.abandonCreation(
                          projectId,
                          pending.request.idempotency_key,
                        );
                      } catch (e) {
                        // Only a definite failed creation plus confirmed absence may be
                        // discarded here, after the user's explicit abandon action.
                        if (!(
                          pending.rejected &&
                          e instanceof StudioError &&
                          e.code === "NOT_FOUND"
                        ))
                          throw e;
                      }
                      draft.controller.set((old) => ({
                        ...old,
                        pending: null,
                      }));
                      await draft.controller.flush();
                    })
                  }
                >
                  放弃未完成创建
                </button>
                <p className="aesthetic-help">
                  已建成阶段请先找回，再使用阶段取消；放弃失败时保留原请求。
                </p>
              </div>
            )}
          </section>
        )}
        <DraftStatus controller={draft.controller} quiet />
      </div>
      <div className="wb-dialog-actions">
        <button
          type="button"
          disabled={busy}
          onClick={() => {
            onClose();
            context.openSettings?.("llm");
          }}
        >
          API 与模型设置
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={() => {
            onClose();
            context.openSettings?.("system-prompts");
          }}
        >
          System Prompt 设置
        </button>
      </div>
    </WorkbenchDialog>
  );
}
