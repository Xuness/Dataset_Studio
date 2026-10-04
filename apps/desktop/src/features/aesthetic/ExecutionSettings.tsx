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

export type ExecutionPolicy = Schema["AestheticExecutionPolicy"];
export const defaultExecutionPolicy: ExecutionPolicy = {
  stream: true,
  concurrency: 2,
  connect_timeout_ms: 15000,
  first_response_timeout_ms: 180000,
  idle_timeout_ms: 60000,
  request_timeout_ms: 600000,
  batch_timeout_ms: 900000,
  max_retries: 2,
  retry_unknown: false,
  exhausted: "pause",
};
export function validPolicy(value: unknown): value is ExecutionPolicy {
  if (!value || typeof value !== "object") return false;
  const p = value as ExecutionPolicy;
  return (
    typeof p.stream === "boolean" &&
    typeof p.retry_unknown === "boolean" &&
    [
      p.concurrency,
      p.connect_timeout_ms,
      p.first_response_timeout_ms,
      p.idle_timeout_ms,
      p.request_timeout_ms,
      p.batch_timeout_ms,
      p.max_retries,
    ].every(Number.isFinite) &&
    ["pause", "defer"].includes(p.exhausted)
  );
}
export function ExecutionPolicyFields({
  policy,
  disabled,
  onChange,
  showConcurrency = true,
}: {
  policy: ExecutionPolicy;
  disabled: boolean;
  onChange: (value: ExecutionPolicy) => void;
  showConcurrency?: boolean;
}) {
  return (
    <div className="wb-field-list execution-policy-fields">
      <label>
        响应方式
        <select
          aria-label="响应方式"
          disabled={disabled}
          value={policy.stream ? "stream" : "json"}
          onChange={(e) =>
            onChange({ ...policy, stream: e.target.value === "stream" })
          }
        >
          <option value="stream">流式 · 显示接收进度</option>
          <option value="json">非流式 · 等待完整响应</option>
        </select>
      </label>
      {showConcurrency && (
        <label>
          请求并发上限
          <input
            aria-label="执行并发上限"
            required
            type="number"
            min={1}
            max={32}
            value={policy.concurrency}
            disabled={disabled}
            onChange={(e) =>
              onChange({ ...policy, concurrency: Number(e.target.value) })
            }
          />
        </label>
      )}
      {(
        [
          ["connect_timeout_ms", "连接超时（秒）", 120],
          ["first_response_timeout_ms", "首包等待（秒）", 600],
          ["idle_timeout_ms", "空闲超时（秒）", 600],
          ["request_timeout_ms", "单次请求总时限（秒）", 3600],
          ["batch_timeout_ms", "批次恢复总时限（秒）", 7200],
        ] as const
      ).map(([key, label, max]) => (
        <label key={key}>
          {label}
          <input
            aria-label={label}
            required
            type="number"
            min={0.1}
            max={max}
            step={0.1}
            value={policy[key] / 1000}
            disabled={disabled}
            onChange={(e) =>
              onChange({
                ...policy,
                [key]: Math.round(Number(e.target.value) * 1000),
              })
            }
          />
        </label>
      ))}
      <p className="aesthetic-help">
        本地排队不占单次请求时限。首包可为保活心跳；每次收到数据重置空闲计时，单次总时限始终不重置。批次恢复总时限包含退避、后续排队和暂停；明确手动重试会开启新窗口。
      </p>
      <label>
        每批自动重试上限
        <select
          aria-label="每批自动重试上限"
          value={policy.max_retries}
          disabled={disabled}
          onChange={(e) =>
            onChange({ ...policy, max_retries: Number(e.target.value) })
          }
        >
          <option value={0}>不自动重试</option>
          <option value={1}>1 次</option>
          <option value={2}>2 次</option>
        </select>
      </label>
      <label>
        <input
          type="checkbox"
          checked={policy.retry_unknown}
          disabled={disabled}
          onChange={(e) =>
            onChange({ ...policy, retry_unknown: e.target.checked })
          }
        />
        允许自动重试结果不明的请求
      </label>
      <p className="aesthetic-help">
        结果不明的原请求可能已计费。允许后按上述上限自动恢复，无需逐批确认；所有重试都计入阶段调用预算。
      </p>
      <label>
        自动恢复耗尽后
        <select
          aria-label="自动恢复耗尽后"
          value={policy.exhausted}
          disabled={disabled}
          onChange={(e) => onChange({ ...policy, exhausted: e.target.value })}
        >
          <option value="pause">保留异常，完成本轮其他批次后处理</option>
          <option value="defer">暂缓旧批次，后续轮次重新编组补测</option>
        </select>
      </label>
      <p className="aesthetic-help">
        暂缓仅适用于已允许自动恢复的瞬态错误，不增加有效曝光；后续能否补齐取决于剩余预算和比较对手。
      </p>
    </div>
  );
}

type Draft = {
  policy: ExecutionPolicy;
  expectedRevision: number;
  pending: Schema["AestheticExecutionUpdate"] | null;
};
function decode(value: unknown): Draft | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Draft;
  return validPolicy(v.policy) &&
    Number.isSafeInteger(v.expectedRevision) &&
    v.expectedRevision >= 0 &&
    (!v.pending ||
      (typeof v.pending.idempotency_key === "string" &&
        validPolicy(v.pending.policy)))
    ? v
    : null;
}
export function ExecutionSettingsDialog({
  context,
  stage,
  onClose,
  onChanged,
}: {
  context: ModuleContext;
  stage: Schema["AestheticStage"];
  onClose: () => void;
  onChanged: () => void;
}) {
  const initial: Draft = {
    policy: stage.execution_settings?.policy ?? {
      ...defaultExecutionPolicy,
      concurrency: stage.config.request.concurrency,
    },
    expectedRevision: stage.execution_settings?.revision ?? 0,
    pending: null,
  };
  const draft = useDraft(
    context.client,
    context.projectId,
    "core.aesthetic",
    initial,
    decode,
    "execution-settings-" + stage.id,
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const lock = useRef(false);
  async function save() {
    if (lock.current || !draft.editable) return;
    lock.current = true;
    setBusy(true);
    setError(null);
    try {
      const request = draft.value.pending ?? {
        idempotency_key: crypto.randomUUID(),
        expected_revision: draft.value.expectedRevision,
        policy: draft.value.policy,
      };
      draft.controller.set((old) => ({ ...old, pending: request }));
      await draft.controller.flush();
      const saved = await context.client.aesthetic.configureExecution(
        context.projectId,
        stage.id,
        request,
      );
      draft.controller.set({
        policy: saved.execution_settings!.policy,
        expectedRevision: saved.execution_settings!.revision,
        pending: null,
      });
      await draft.controller.flush();
      onChanged();
      onClose();
    } catch (e) {
      if (
        e instanceof StudioError &&
        [
          "REVISION_CONFLICT",
          "INVALID_INPUT",
          "EVALUATION_STANDARD_CHANGED",
        ].includes(e.code)
      )
        draft.controller.set((old) => ({ ...old, pending: null }));
      setError(e);
    } finally {
      lock.current = false;
      setBusy(false);
    }
  }
  return (
    <WorkbenchDialog
      title="阶段执行设置"
      onClose={() => {
        if (!busy) onClose();
      }}
    >
      <p>
        {stage.name} · {stage.config.model.remote_model_id}
      </p>
      {!stage.execution_settings && (
        <p className="aesthetic-notice">
          此阶段尚未启用独立执行设置。下方为建议值，保存后生效。
        </p>
      )}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <ExecutionPolicyFields
          policy={draft.value.policy}
          disabled={busy || !draft.editable || !!draft.value.pending}
          onChange={(policy) =>
            draft.controller.set((old) => ({ ...old, policy }))
          }
        />
        <p>
          保存时关联当前连接版本，并核对模型、端点和冻结评审标准。已保存的证据继续保留；保存后仍需点击“开始评审”；已有异常批次请先批量加入重试队列。
        </p>
        {error != null && <ErrorDetails error={error} />}
        <DraftStatus controller={draft.controller} />
        <div className="wb-dialog-actions">
          <button
            type="button"
            disabled={busy || !!draft.value.pending}
            onClick={() => {
              void context.client.aesthetic
                .stage(context.projectId, stage.id)
                .then((s) => {
                  draft.controller.set({
                    policy: s.execution_settings?.policy ?? initial.policy,
                    expectedRevision: s.execution_settings?.revision ?? 0,
                    pending: null,
                  });
                  setError(null);
                })
                .catch(setError);
            }}
          >
            重新载入阶段设置
          </button>
          <button type="submit" disabled={busy || !draft.editable}>
            {draft.value.pending ? "确认上次保存结果" : "保存并关联当前连接"}
          </button>
        </div>
      </form>
    </WorkbenchDialog>
  );
}
