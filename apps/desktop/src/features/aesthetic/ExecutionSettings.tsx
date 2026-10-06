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
import { ConnectionLimits } from "./ConnectionLimits.js";
import { ImageInputSettings } from "./ImageInputSettings.js";

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
  memory_budget_mib: 512,
  upload_bytes_per_second: 3500000,
};
/**
 * Mirrors the engine's per-request reservation: 6 × (body + 64 KiB) + 64 MiB while
 * preparing, then 2.5 × body + 1 MiB + 64 MiB once the request is sent.
 */
export function requestReservationMiB(maxRequestMiB: number, resizing = false) {
  const encoding = Math.ceil(6 * (maxRequestMiB + 1 / 16) + 64);
  return {
    // A batch reads at most 16 × 2 MiB, plus their base64 estimate. Transform
    // workspace and final request serialization peak at different times.
    preparing: resizing
      ? Math.max(
          encoding,
          Math.ceil((32 * 4) / 3 + maxRequestMiB * 2 + 256 + 64),
        )
      : encoding,
    inFlight: Math.ceil(2.5 * maxRequestMiB + 1 + 64),
  };
}
/** Engine default when a stage leaves the threshold unset. */
export function defaultFailureHaltThreshold(concurrency: number) {
  return Math.min(32, Math.max(4, concurrency));
}
export function validPolicy(value: unknown): value is ExecutionPolicy {
  if (!value || typeof value !== "object") return false;
  const p = value as ExecutionPolicy;
  return (
    typeof p.stream === "boolean" &&
    typeof p.retry_unknown === "boolean" &&
    (p.image_max_edge == null || Number.isSafeInteger(p.image_max_edge)) &&
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
  maxRequestMiB,
  concurrency = policy.concurrency,
  showConcurrency = true,
}: {
  policy: ExecutionPolicy;
  disabled: boolean;
  onChange: (value: ExecutionPolicy) => void;
  maxRequestMiB: number;
  concurrency?: number;
  showConcurrency?: boolean;
}) {
  const memory =
    policy.memory_budget_mib ?? defaultExecutionPolicy.memory_budget_mib!;
  const upload =
    policy.upload_bytes_per_second ??
    defaultExecutionPolicy.upload_bytes_per_second!;
  const perRequest = requestReservationMiB(
    maxRequestMiB,
    policy.image_max_edge != null,
  );
  // One request may still be preparing while the others wait for their responses.
  const capacity =
    memory < perRequest.preparing
      ? 0
      : Math.floor((memory - perRequest.preparing) / perRequest.inFlight) + 1;
  const needed =
    perRequest.preparing + Math.max(0, concurrency - 1) * perRequest.inFlight;
  return (
    <div className="wb-field-list execution-policy-fields">
      <ImageInputSettings
        maxEdge={policy.image_max_edge}
        disabled={disabled}
        onChange={(image_max_edge) => {
          const next = { ...policy };
          if (image_max_edge == null) delete next.image_max_edge;
          else next.image_max_edge = image_max_edge;
          onChange(next);
        }}
      />
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
            max={1024}
            value={policy.concurrency}
            disabled={disabled}
            onChange={(e) =>
              onChange({ ...policy, concurrency: Number(e.target.value) })
            }
          />
        </label>
      )}
      <label>
        阶段内存预算（MiB）
        <input
          aria-label="阶段内存预算（MiB）"
          required
          type="number"
          min={256}
          max={1048576}
          step={1}
          value={memory}
          disabled={disabled}
          onChange={(e) =>
            onChange({ ...policy, memory_budget_mib: Number(e.target.value) })
          }
        />
      </label>
      <p
        className={
          capacity < concurrency ? "aesthetic-notice" : "aesthetic-help"
        }
      >
        准备请求时预留编码副本及响应空间
        {policy.image_max_edge != null &&
          "，缩图还计入逐图解码和重采样的工作空间"}
        ，发出后降为 2.5 × 请求体 + 65 MiB。以每批 {maxRequestMiB} MiB
        请求体估算，准备时约 {perRequest.preparing} MiB、在途约{" "}
        {perRequest.inFlight} MiB，当前预算最坏情况约可同时保持 {capacity}{" "}
        个请求，图片较小时更多
        {capacity < concurrency &&
          `；要稳定达到并发 ${concurrency}，约需 ${needed.toLocaleString()} MiB`}
        。预算按阶段计算，同时运行的阶段各自占用。
      </p>
      <label>
        上传速率上限（MB/s，0 为不限速）
        <input
          aria-label="上传速率上限（MB/s）"
          required
          type="number"
          min={0}
          max={10000}
          step={0.1}
          value={upload / 1e6}
          disabled={disabled}
          onChange={(e) =>
            onChange({
              ...policy,
              upload_bytes_per_second: Math.round(Number(e.target.value) * 1e6),
            })
          }
        />
      </label>
      <p className="aesthetic-help">
        按请求体字节数错开派发，所有运行中的阶段共用一条上传队列。
        {upload > 0 &&
          `以每批 ${maxRequestMiB} MiB 计，每秒最多派发约 ${(upload / (maxRequestMiB * 1048576)).toFixed(2)} 个请求。`}
      </p>
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
        连续失败停止阈值
        <input
          aria-label="连续失败停止阈值"
          type="number"
          min={1}
          max={1024}
          step={1}
          placeholder={`自动：${defaultFailureHaltThreshold(concurrency)}`}
          value={policy.failure_halt_threshold ?? ""}
          disabled={disabled}
          onChange={(e) => {
            const next = { ...policy };
            if (e.target.value === "") delete next.failure_halt_threshold;
            else next.failure_halt_threshold = Number(e.target.value);
            onChange(next);
          }}
        />
      </label>
      <p className="aesthetic-help">
        每次已发送的请求失败（含自动重试）计 1
        次，收到有效结果或重新开始评审时清零；达到阈值后停止派发新请求，在途请求照常收尾。留空时取并发数，限定在
        4–32 之间（当前 {defaultFailureHaltThreshold(concurrency)}
        ）。认证失败、配置错误、余额不足等问题不计次，立即停止。
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
          maxRequestMiB={stage.config.max_request_bytes / 1048576}
          disabled={busy || !draft.editable || !!draft.value.pending}
          onChange={(policy) =>
            draft.controller.set((old) => ({ ...old, policy }))
          }
        />
        <ConnectionLimits
          context={context}
          providerId={stage.config.model.provider_id}
          concurrency={draft.value.policy.concurrency}
          disabled={busy || !draft.editable || !!draft.value.pending}
        />
        <p>
          图片分辨率可以在评审中途调整，恢复后新调用和重试使用新设置；历史调用保留各自的实际输入记录。保存时关联当前连接版本，并核对模型、端点和冻结提示词。保存后仍需点击“开始评审”；已有异常批次请先加入重试队列。
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
