import { useRef, useState } from "react";
import { StudioError } from "@studio/client";
import type { Schema } from "@studio/contracts";
import {
  DraftStatus,
  ErrorDetails,
  useDraft,
  WorkbenchDialog,
} from "@studio/ui";
import type { ModuleContext } from "@studio/ui";

const initial = {
  reason: "",
  acknowledge: false,
  pending: null as Schema["AestheticBatchAction"] | null,
};
function decode(value: unknown): typeof initial | null {
  if (!value || typeof value !== "object") return null;
  const v = value as typeof initial;
  return typeof v.reason === "string" &&
    typeof v.acknowledge === "boolean" &&
    (!v.pending || typeof v.pending.idempotency_key === "string")
    ? v
    : null;
}
export function BatchRecoveryDialog({
  context,
  stage,
  action,
  batches = [],
  onClose,
  onChanged,
}: {
  context: ModuleContext;
  stage: Schema["AestheticStage"];
  action: "retry" | "defer";
  batches?: number[];
  onClose: () => void;
  onChanged: (message: string) => void;
}) {
  const draft = useDraft(
    context.client,
    context.projectId,
    "core.aesthetic",
    initial,
    decode,
    `batch-action-${stage.id}-${action}-${batches.join("-") || "all"}`,
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [result, setResult] = useState<
    Schema["AestheticBatchActionResult"] | null
  >(null);
  const lock = useRef(false);
  const reasonBytes = new TextEncoder().encode(
    draft.value.reason.trim(),
  ).length;
  async function submit() {
    if (
      lock.current ||
      !draft.editable ||
      !draft.value.acknowledge ||
      reasonBytes > 1000
    )
      return;
    lock.current = true;
    setBusy(true);
    setError(null);
    try {
      const request = draft.value.pending ?? {
        idempotency_key: crypto.randomUUID(),
        action,
        batches,
        acknowledge_possible_charge: true,
        reason: draft.value.reason.trim(),
      };
      draft.controller.set((old) => ({ ...old, pending: request }));
      await draft.controller.flush();
      let response: Schema["AestheticBatchActionResult"];
      do {
        response = await context.client.aesthetic.batchAction(
          context.projectId,
          stage.id,
          request,
        );
        setResult(response);
      } while (!response.completed);
      draft.controller.set({ ...initial });
      await draft.controller.flush();
      onChanged(
        `${response.succeeded} 批已${action === "retry" ? "加入重试队列" : "暂缓"}，${response.failed} 批未处理。${action === "retry" ? "点击开始评审后派发。" : "已有证据保留，继续评审后按实际曝光重新规划。"}`,
      );
    } catch (e) {
      if (
        e instanceof StudioError &&
        ["INVALID_INPUT", "IDEMPOTENCY_CONFLICT"].includes(e.code)
      )
        draft.controller.set((old) => ({ ...old, pending: null }));
      setError(e);
    } finally {
      lock.current = false;
      setBusy(false);
    }
  }
  const title = action === "retry" ? "批量安排重试" : "暂缓异常批次";
  return (
    <WorkbenchDialog
      title={title}
      onClose={() => {
        if (!busy) onClose();
      }}
    >
      <p>{stage.name}</p>
      <p>
        {batches.length
          ? `处理选中的 ${batches.length} 个批次。`
          : "处理整个阶段的异常批次，不受当前页或筛选显示范围限制。"}{" "}
        操作不会立即调用模型。
      </p>
      <p>
        {action === "retry"
          ? "保留历史尝试，开启新的批次恢复窗口。每次重试消耗阶段调用预算，结果不明的原请求也可能已计费。"
          : "结束这些逻辑批次并保留全部调用记录，释放可用图片供后续重新编组。暂缓不增加有效曝光，也不保证剩余预算足以补齐。"}
      </p>
      {!result?.completed && (
        <form
          className="wb-field-list"
          onSubmit={(e) => {
            e.preventDefault();
            void submit();
          }}
        >
          {action === "defer" && (
            <label>
              暂缓原因
              <textarea
                aria-label="暂缓原因"
                required
                maxLength={1000}
                disabled={busy || !!draft.value.pending}
                value={draft.value.reason}
                onChange={(e) =>
                  draft.controller.set((old) => ({
                    ...old,
                    reason: e.target.value,
                  }))
                }
              />
            </label>
          )}
          <label>
            <input
              type="checkbox"
              checked={draft.value.acknowledge}
              disabled={busy || !!draft.value.pending}
              onChange={(e) =>
                draft.controller.set((old) => ({
                  ...old,
                  acknowledge: e.target.checked,
                }))
              }
            />
            我已了解处理范围和可能已发生的费用
          </label>
          <button
            type="submit"
            disabled={busy || !draft.editable || !draft.value.acknowledge}
          >
            {busy
              ? "正在分批处理…"
              : draft.value.pending
                ? "继续同一次批量操作"
                : action === "retry"
                  ? "确认加入重试队列"
                  : "确认暂缓"}
          </button>
        </form>
      )}
      {result && (
        <div role="status">
          <p>
            已处理 {result.processed} 批 · 成功 {result.succeeded} · 未处理{" "}
            {result.failed}
          </p>
          {result.items
            .filter((item) => item.error)
            .map((item) => (
              <p key={item.sequence}>
                阶段批次 {item.stage_sequence || item.sequence}：{item.error}
              </p>
            ))}
          {result.failed > 0 && (
            <p>未处理的批次保留原状态，可回到异常筛选查看。</p>
          )}
        </div>
      )}
      {action === "defer" && (
        <p className="aesthetic-help">暂缓原因 {reasonBytes} / 1000 字节</p>
      )}
      {error != null && <ErrorDetails error={error} />}
      <DraftStatus controller={draft.controller} quiet />
      {result?.completed && (
        <button type="button" onClick={onClose}>
          完成
        </button>
      )}
    </WorkbenchDialog>
  );
}
