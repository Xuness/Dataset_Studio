import { useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { Schema } from "@studio/contracts";
import {
  DraftStatus,
  ErrorDetails,
  useDraft,
  WorkbenchDialog,
} from "@studio/ui";
import type { ModuleContext } from "@studio/ui";

export const samplingReason = (value: string) =>
  (
    ({
      coverage: "补齐基础曝光",
      bridge: "补强跨组连接",
      uncertainty: "位次尚不稳定",
      opponent_diversity: "增加不同对手",
      stable: "位次暂时稳定",
      exploration: "轮换抽查",
      anchor: "提供比较对手",
      covered: "基础覆盖完成",
      exposure_limit: "达到单图上限",
      unavailable: "候选当前不可派发",
      call_budget: "达到调用上限",
      evidence_limit: "达到当前采样器的证据容量上限",
      pending_outcomes: "已有批次待恢复或明确重试",
      ranking_scope_incomplete: "冻结范围不完整，无法评估全体位次变化",
      no_available_peer: "没有可用比较对手",
      no_comparison_peer: "缺少同 Rating 对手，已转异常候选",
      no_remaining_candidates: "所有候选已明确排除",
      unresolved_candidates: "仍有候选待处理",
      empirical_stability: "达到经验稳定条件",
      coverage_connected: "基础曝光和连接已完成",
    }) as Record<string, string>
  )[value] ?? value;

type Stage = Schema["AestheticStage"];
export function SamplingPanel({
  context,
  stage,
  disabled,
  onChanged,
}: {
  context: ModuleContext;
  stage: Stage;
  disabled: boolean;
  onChanged: () => void;
}) {
  const [open, setOpen] = useState(false);
  const s = stage.sampling;
  const stopped = [
    "ready",
    "paused",
    "needs_attention",
    "failed",
    "completed",
    "completed_with_exclusions",
  ].includes(stage.state);
  return (
    <details className="wb-fold" open>
      <summary>覆盖与动态曝光</summary>
      {s ? (
        <>
          <dl className="wb-property-list">
            <dt>采样方式</dt>
            <dd>{s.policy.mode === "adaptive" ? "动态分配" : "均衡覆盖"}</dd>
            <dt>每图有效曝光</dt>
            <dd>
              最低 {s.policy.min_exposures} · 上限 {s.policy.max_exposures}
            </dd>
            <dt>最近诊断</dt>
            <dd>
              {s.round
                ? `检查点 ${s.round} · 证据水位 ${s.evidence_watermark}`
                : "等待首次诊断"}
            </dd>
            {s.round > 0 && (
              <>
                <dt>基础覆盖</dt>
                <dd>
                  {s.covered} / {s.eligible}
                </dd>
                <dt>经验稳定</dt>
                <dd>
                  {s.stable} / {s.eligible}
                </dd>
                <dt>比较分量</dt>
                <dd>{s.components}（各 Rating 独立）</dd>
                <dt>尚需评估</dt>
                <dd>{s.unresolved}</dd>
              </>
            )}
            <dt>调度状态</dt>
            <dd>
              {s.reason
                ? samplingReason(s.reason)
                : s.state === "dispatching"
                  ? stopped
                    ? "轮次已停靠，可恢复或处置"
                    : "执行已冻结的轮次"
                  : "等待开始"}
            </dd>
          </dl>
          <p className="aesthetic-help">
            诊断在整轮结束后更新。
            {s.policy.mode === "adaptive"
              ? "经验稳定依据连续两次位次变化、对手多样性与拟合诊断；不是统计置信区间，也不代表模型审美正确。"
              : "均衡模式以基础曝光和比较连接作为停止条件；排名稳定性可在离线分析中查看。"}
          </p>
        </>
      ) : (
        <p className="aesthetic-help">
          此阶段使用旧版固定曝光。可基于已有证据追加连接与动态复评。
        </p>
      )}
      <button
        disabled={
          disabled ||
          !stopped ||
          stage.frozen !== stage.total ||
          stage.total > 10000
        }
        onClick={() => setOpen(true)}
      >
        配置追加评审
      </button>
      <p className="aesthetic-help">
        追加计划沿用冻结的模型、Prompt
        和图片范围。保存计划后，点击“开始评审”才会调用模型。失败或结果不明的批次仍需单独选择重试。
      </p>
      {open && (
        <SamplingDialog
          key={stage.id}
          context={context}
          stage={stage}
          onClose={() => setOpen(false)}
          onChanged={onChanged}
        />
      )}
    </details>
  );
}

function SamplingDialog({
  context,
  stage,
  onClose,
  onChanged,
}: {
  context: ModuleContext;
  stage: Stage;
  onClose: () => void;
  onChanged: () => void;
}) {
  const initial = {
    mode: "adaptive",
    min: stage.config.request.exposures,
    max: Math.min(
      32,
      Math.max(
        8,
        stage.sampling?.policy.max_exposures ?? 0,
        stage.config.request.exposures,
      ),
    ),
    tolerance: 10,
    calls: 16,
    seed: 17,
    pending: null as Schema["AestheticSamplingRequest"] | null,
  };
  const draft = useDraft(
    context.client,
    context.projectId,
    "core.aesthetic",
    initial,
    (input: unknown) => {
      if (!input || typeof input !== "object") return null;
      const v = input as typeof initial;
      return ["adaptive", "balanced"].includes(v.mode) &&
        [v.min, v.max, v.tolerance, v.calls, v.seed].every(Number.isFinite) &&
        (!v.pending || typeof v.pending.idempotency_key === "string")
        ? v
        : null;
    },
    "sampling-plan-" + stage.id,
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const lock = useRef(false);
  const v = draft.value;
  const editable = draft.editable && !busy && !v.pending;
  return (
    <WorkbenchDialog
      title="追加连接与动态评审"
      onClose={() => {
        if (!busy) onClose();
      }}
    >
      <form
        className="wb-field-list"
        onSubmit={(event) => {
          event.preventDefault();
          if (lock.current || !draft.editable) return;
          lock.current = true;
          setBusy(true);
          setError(null);
          void (async () => {
            const request = v.pending ?? {
              idempotency_key: crypto.randomUUID(),
              additional_calls: v.calls,
              policy: {
                mode: v.mode,
                min_exposures: v.min,
                max_exposures: v.max,
                rank_tolerance: v.tolerance / 100,
                seed: v.seed,
              },
            };
            draft.controller.set((old) => ({ ...old, pending: request }));
            await context.client.edits.flush(context.projectId);
            await context.client.aesthetic.configureSampling(
              context.projectId,
              stage.id,
              request,
            );
            draft.controller.set((old) => ({ ...old, pending: null }));
            await draft.controller.flush();
            onChanged();
            onClose();
          })()
            .catch(setError)
            .finally(() => {
              lock.current = false;
              setBusy(false);
            });
        }}
      >
        <p>
          复用已接受的 {stage.accepted}{" "}
          批证据；追加预算只统计保存计划后新增的调用尝试。旧排名快照保持其原证据范围。
        </p>
        <label>
          追加采样方式
          <select
            aria-label="追加采样方式"
            disabled={!editable}
            value={v.mode}
            onChange={(e) =>
              draft.controller.set((old) => ({ ...old, mode: e.target.value }))
            }
          >
            <option value="adaptive">动态分配：覆盖、连接与经验稳定</option>
            <option value="balanced">均衡覆盖：满足最低曝光并补齐连接</option>
          </select>
        </label>
        {(
          [
            ["min", "最低有效曝光", stage.config.request.exposures, 32],
            ["max", "单图有效曝光上限", v.min, 32],
            ["tolerance", "位次变化阈值（百分点）", 1, 25],
            ["calls", "追加调用次数上限", 1, 10000000],
            ["seed", "采样种子", 0, 4294967295],
          ] as const
        )
          .filter(([key]) => key !== "tolerance" || v.mode === "adaptive")
          .map(([key, label, min, max]) => (
            <label key={key}>
              {label}
              <input
                aria-label={label}
                type="number"
                required
                min={min}
                max={max}
                step={1}
                value={v[key]}
                disabled={!editable}
                onChange={(e) =>
                  draft.controller.set((old) => ({
                    ...old,
                    [key]: Number(e.target.value),
                  }))
                }
              />
            </label>
          ))}
        <p className="aesthetic-help">
          达到单图或调用上限时会保留未满足的条件；位次变化阈值是调度参数，不是准确率承诺。更换模型或
          Prompt 请创建新阶段。
        </p>
        {error != null && <ErrorDetails error={error} />}
        <DraftStatus controller={draft.controller} />
        <button type="submit" disabled={busy || !draft.editable}>
          {v.pending ? "重试保存同一计划" : "保存追加计划"}
        </button>
      </form>
    </WorkbenchDialog>
  );
}

export function SamplingDiagnostic({
  context,
  stage,
  ordinal,
}: {
  context: ModuleContext;
  stage: Stage;
  ordinal: number;
}) {
  const { data, error } = useQuery({
    queryKey: [
      "project",
      context.projectId,
      "aesthetic",
      "sampling",
      stage.id,
      stage.sampling?.plan_id,
      stage.sampling?.round,
      ordinal,
    ],
    queryFn: ({ signal }) =>
      context.client.aesthetic.samplingDiagnostic(
        context.projectId,
        stage.id,
        ordinal,
        signal,
      ),
    enabled: !!stage.sampling,
  });
  if (error) return <ErrorDetails error={error} />;
  if (!data) return null;
  return (
    <details className="wb-fold" open>
      <summary>最近轮次诊断</summary>
      <dl className="wb-property-list">
        <dt>采样依据</dt>
        <dd>{samplingReason(data.reason)}</dd>
        <dt>不同对手</dt>
        <dd>
          {data.distinct_opponents}
          {data.distinct_opponents === 32 ? " 及以上" : ""}
        </dd>
        <dt>位次变化</dt>
        <dd>
          {data.rank_delta == null
            ? "未知 / 尚不可比较"
            : `${(data.rank_delta * 100).toFixed(1)} 个百分点`}
        </dd>
        <dt>连续稳定检查</dt>
        <dd>{data.stable_rounds}</dd>
      </dl>
    </details>
  );
}
