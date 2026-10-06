import { samplingReason } from "./SamplingPanel.js";
import { imageInputLabel } from "./ImageInputSettings.js";
import { useEffect, useState } from "react";
import type { Schema } from "@studio/contracts";
import type { ModuleContext } from "@studio/ui";
import { WorkbenchDialog } from "@studio/ui";
import { AssetViewer } from "./AssetViewer.js";
import { aestheticTime } from "./analysisPresentation.js";
import { BatchRecoveryDialog } from "./BatchRecoveryDialog.js";

const labels: Record<string, string> = {
  preparing: "准备中",
  ready: "待开始",
  running: "评审中",
  pausing: "暂停中",
  paused: "已暂停",
  cancelling: "取消中",
  cancelled: "已取消",
  completed: "本阶段已完成",
  completed_with_exclusions: "本阶段已完成（含排除项）",
  needs_attention: "需要处理",
  failed: "失败",
  queued: "等待派发",
  sent: "等待模型",
  received: "已保存返回",
  accepted: "有效评审",
  invalid: "无效结果",
  superseded: "已重组",
  outcome_unknown: "结果不明",
  retry_wait: "等待自动重试",
  deferred: "已暂缓，待后续补测",
};
export const stateLabel = (state: string) => labels[state] ?? state;
export function CandidateCard({
  context,
  candidate,
  label,
  historical = false,
  nominated = false,
  onOpen,
  tier,
}: {
  context: ModuleContext;
  candidate: Schema["AestheticCandidate"];
  label?: string;
  historical?: boolean;
  nominated?: boolean;
  onOpen?: (() => void) | undefined;
  tier?: string | undefined;
}) {
  const { client, projectId } = context;
  const { source_id, asset_id } = candidate.key;
  const [url, setUrl] = useState("");
  const [error, setError] = useState(false);
  useEffect(() => {
    const abort = new AbortController();
    let release: (() => void) | undefined;
    setUrl("");
    setError(false);
    void client
      .acquireMedia(
        projectId,
        {
          key: { source_id, asset_id },
          name: asset_id,
          bytes: String(candidate.bytes),
          extension: "",
          source_name: "",
          selected: false,
          summary: null,
        },
        240,
        { signal: abort.signal },
      )
      .then((handle) => {
        if (abort.signal.aborted) handle.release();
        else {
          release = handle.release;
          setUrl(handle.url);
        }
      })
      .catch(() => {
        if (!abort.signal.aborted) setError(true);
      });
    return () => {
      abort.abort();
      release?.();
    };
  }, [client, projectId, source_id, asset_id, candidate.bytes]);
  return (
    <figure className="aesthetic-image">
      <div>
        {url ? (
          <img src={url} alt={label ?? asset_id} />
        ) : (
          <span>{error ? "预览不可用" : "加载预览…"}</span>
        )}
        {onOpen && (
          <button
            type="button"
            className="aesthetic-image-open"
            aria-label={`查看大图 ${label ?? `候选 ${candidate.ordinal + 1}`}`}
            onClick={onOpen}
          >
            <span className="wb-sr-only">查看大图</span>
          </button>
        )}
      </div>
      <figcaption>
        <b>{label ?? `候选 ${candidate.ordinal + 1}`}</b>
        {tier && <span className="aesthetic-tier-tag">{tier}</span>}
        {candidate.blocked && (
          <span>
            {candidate.blocking_batch ? "等待批次恢复" : "待处理候选"}
          </span>
        )}
        {nominated ? (
          <em>本批顶级提名</em>
        ) : (
          candidate.protected && (
            <em>{historical ? "此前已保护" : "保护候选"}</em>
          )
        )}
        <span>
          {candidate.rating.toUpperCase()} · {candidate.year ?? "年份未知"} ·
          {historical ? "评审前曝光" : "有效曝光"} {candidate.exposures}
        </span>
        <small title={asset_id}>{asset_id.slice(0, 16)}</small>
      </figcaption>
    </figure>
  );
}
export function BatchDetail({
  context,
  stage,
  batch,
  disabled,
  perform,
  defaultOpen = false,
}: {
  context: ModuleContext;
  stage: Schema["AestheticStage"];
  batch: Schema["AestheticBatch"];
  disabled: boolean;
  perform: (action: () => Promise<unknown>) => Promise<void>;
  defaultOpen?: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  const [attempts, setAttempts] = useState<Schema["AestheticAttempts"] | null>(
    null,
  );
  const [confirm, setConfirm] = useState(false);
  const canRecover =
    !stage.archived &&
    ["ready", "paused", "needs_attention", "failed"].includes(stage.state);
  const [order, setOrder] = useState<"tiers" | "input">("tiers");
  const [imageOrdinal, setImageOrdinal] = useState<number | null>(null);
  const [deferring, setDeferring] = useState(false);
  const startedAt = Number(batch.transfer?.started_at ?? NaN);
  const elapsedSeconds = Number.isFinite(startedAt)
    ? Math.max(0, Math.floor((Date.now() - startedAt) / 1000))
    : null;
  const tiers = new Map(
    batch.observation?.tiers.flatMap((tier, index) =>
      tier.map((label) => [label, index + 1] as const),
    ) ?? [],
  );
  const ordered =
    order === "tiers" && batch.observation
      ? [
          ...batch.observation.tiers.flat(),
          ...batch.observation.unjudgeable.map((v) => v.id),
        ].flatMap((label) => batch.members.filter((m) => m.label === label))
      : batch.members;
  return (
    <details
      className="aesthetic-batch"
      data-state={batch.state}
      open={open}
      onToggle={(e) => setOpen(e.currentTarget.open)}
    >
      <summary>
        批次 {batch.stage_sequence || batch.sequence} ·{" "}
        {batch.sampling && `第 ${batch.sampling.round} 轮 · `}
        {batch.rating.toUpperCase()} · {batch.members.length} 图 ·{" "}
        {stateLabel(batch.state)}
      </summary>
      <p className="aesthetic-help">
        全局编号 #{batch.sequence} · 已调用 {batch.attempt_count ?? 0} 次
        {batch.retry_at && ` · 下次重试 ${aestheticTime(batch.retry_at)}`}
      </p>
      {batch.transfer && (
        <p role="status" className="aesthetic-transfer">
          {(
            {
              queueing: "等待本地并发名额",
              waiting_response: "等待首个响应",
              waiting_result: "上游连接活跃，等待结果",
              receiving: "接收响应",
              reasoning: "接收推理信息",
              generating: "接收结果",
              saving: "保存回执",
            } as Record<string, string>
          )[batch.transfer.phase] ?? batch.transfer.phase}
          {batch.transfer.started_at &&
            ` · 开始 ${aestheticTime(batch.transfer.started_at)}`}
          {batch.transfer.last_data_at &&
            ` · 最近数据 ${aestheticTime(batch.transfer.last_data_at)}`}
          {elapsedSeconds != null && ` · 已运行 ${elapsedSeconds} 秒`}
          {` · 已接收 ${(batch.transfer.received_bytes / 1024).toFixed(1)} KiB`}
        </p>
      )}
      {batch.error && <p>{batch.error}</p>}
      {batch.resolution_reason && (
        <p>
          暂缓原因：
          {(
            {
              recovery_time_budget: "批次恢复时限已到",
              automatic_recovery_exhausted: "自动恢复次数或预算已用尽",
            } as Record<string, string>
          )[batch.resolution_reason] ?? batch.resolution_reason}
        </p>
      )}
      {batch.last_failure && (
        <p className="aesthetic-help">
          错误类型：{batch.last_failure.code}
          {batch.last_failure.http_status != null &&
            ` · HTTP ${batch.last_failure.http_status}`}
          {batch.last_failure.outcome_unknown && " · 上游可能已执行并计费"}
        </p>
      )}
      {batch.sampling && (
        <details className="wb-fold">
          <summary>第 {batch.sampling.round} 轮 · 采样依据</summary>
          <p>
            决策证据水位 {batch.sampling.evidence_watermark}
            ；整轮计划在派发前冻结。
          </p>
          {batch.sampling.members.map((m) => (
            <p key={m.ordinal}>
              候选 {m.ordinal + 1} · {samplingReason(m.reason)}
            </p>
          ))}
        </details>
      )}
      {batch.parent_sequence != null && (
        <p>由批次 {batch.parent_sequence} 在发送前重组。</p>
      )}
      {(batch.replacement_sequences?.length ?? 0) > 0 && (
        <p>
          替代批次：{batch.replacement_sequences.join("、")}。原批次未调用模型。
        </p>
      )}
      {batch.observation && (
        <>
          <p>
            批内梯队：
            {batch.observation.tiers
              .map((tier) => tier.join(" = "))
              .join(" ＞ ")}
          </p>
          <p>
            顶级提名：{batch.observation.elite_candidates.join("、") || "无"}
          </p>
          {batch.observation.unjudgeable.map((v) => (
            <p key={v.id}>
              无法判断 {v.id}：{v.reason}
            </p>
          ))}
        </>
      )}
      {open && (
        <>
          {batch.observation && (
            <label className="aesthetic-evidence-order">
              图片顺序
              <select
                aria-label={`批次 ${batch.stage_sequence || batch.sequence} 图片顺序`}
                value={order}
                onChange={(e) => setOrder(e.target.value as "tiers" | "input")}
              >
                <option value="tiers">按梯队从高到低</option>
                <option value="input">按发送给模型的顺序</option>
              </select>
            </label>
          )}
          <div className="aesthetic-images">
            {ordered.map((m) => (
              <CandidateCard
                historical
                nominated={
                  batch.observation?.elite_candidates.includes(m.label) ?? false
                }
                key={m.label}
                context={context}
                candidate={m.candidate}
                label={m.label}
                tier={
                  tiers.has(m.label)
                    ? `第 ${tiers.get(m.label)} 梯队`
                    : batch.observation
                      ? "无法判断"
                      : undefined
                }
                onOpen={() => setImageOrdinal(m.candidate.ordinal)}
              />
            ))}
          </div>
        </>
      )}
      <div className="aesthetic-actions">
        <button
          onClick={() =>
            void perform(async () =>
              setAttempts(
                await context.client.aesthetic.attempts(
                  context.projectId,
                  stage.id,
                  batch.sequence,
                ),
              ),
            )
          }
        >
          查看调用返回
        </button>
        {batch.attempt_id &&
          ["failed", "invalid", "outcome_unknown"].includes(batch.state) && (
            <button
              disabled={disabled || !batch.has_raw_receipt}
              title={
                batch.has_raw_receipt
                  ? "使用已保存回执，不调用模型"
                  : "此调用没有原始回执，无法本地重解析"
              }
              onClick={() =>
                void perform(async () => {
                  await context.client.aesthetic.reparse(
                    context.projectId,
                    stage.id,
                    batch.sequence,
                  );
                  setAttempts(null);
                })
              }
            >
              本地重解析原始回执
            </button>
          )}
        {["failed", "invalid", "outcome_unknown"].includes(batch.state) && (
          <>
            <button
              disabled={disabled || !canRecover}
              title={
                canRecover
                  ? "保留旧尝试并明确安排新调用"
                  : "阶段已结束或归档，不能安排重试"
              }
              onClick={() => setConfirm(true)}
            >
              重新评审此批
            </button>
            <button
              disabled={disabled || !canRecover}
              onClick={() => setDeferring(true)}
            >
              暂缓此批并释放候选
            </button>
          </>
        )}
      </div>
      {confirm && (
        <div className="aesthetic-notice">
          <p>
            会新增一次 API
            调用；先前结果不明的请求也可能已经计费。重试保留历史尝试，同一批只接受一份有效结果。
          </p>
          <button
            disabled={disabled || !canRecover}
            onClick={() =>
              void perform(async () => {
                await context.client.aesthetic.retry(
                  context.projectId,
                  stage.id,
                  batch.sequence,
                  true,
                );
                setConfirm(false);
              })
            }
          >
            确认加入重试队列
          </button>{" "}
          <button onClick={() => setConfirm(false)}>返回</button>
          <p>加入后点击“开始评审”派发。</p>
        </div>
      )}
      {attempts && (
        <div className="aesthetic-attempts">
          {attempts.items.map((a, index) => (
            <article key={a.id}>
              <p>
                尝试 {index + 1} · {stateLabel(a.state)} ·{" "}
                {aestheticTime(a.created_at)}
              </p>
              {a.failure && (
                <p>
                  {a.failure.code}：{a.failure.message}
                </p>
              )}
              <p className="aesthetic-help">
                {a.raw_receipt
                  ? `已保存 ${a.raw_receipt.bytes.toLocaleString()} 字节回执${a.raw_receipt.complete ? "" : "（接收不完整）"}`
                  : "没有原始回执"}
                {a.execution_settings &&
                  ` · ${a.execution_settings.policy.stream ? "流式" : "非流式"} · ${imageInputLabel(a.execution_settings.policy.image_max_edge)} · 单次上限 ${a.execution_settings.policy.request_timeout_ms / 1000} 秒`}
              </p>
              {a.image_inputs ? (
                <details>
                  <summary>
                    实际发送图片 · {a.image_inputs.images.length} 张 · 请求体{" "}
                    {(a.image_inputs.request_bytes / 1048576).toFixed(2)} MiB
                  </summary>
                  <div className="aesthetic-image-inputs-scroll">
                    <table
                      className="aesthetic-image-inputs"
                      aria-label="实际发送图片尺寸"
                    >
                      <thead>
                        <tr>
                          <th>图片</th>
                          <th>原尺寸</th>
                          <th>发送尺寸</th>
                          <th>格式</th>
                          <th>原文件 → 发送</th>
                        </tr>
                      </thead>
                      <tbody>
                        {a.image_inputs.images.map(({ label, image }) => (
                          <tr key={label}>
                            <td>{label}</td>
                            <td>
                              {image.source_width && image.source_height
                                ? `${image.source_width} × ${image.source_height}`
                                : "未知"}
                            </td>
                            <td>
                              {image.width && image.height
                                ? `${image.width} × ${image.height}`
                                : "未知"}
                            </td>
                            <td>
                              {image.content_type
                                .replace("image/", "")
                                .toUpperCase()}
                            </td>
                            <td>
                              {(image.source_bytes / 1024).toFixed(1)} →{" "}
                              {(image.bytes / 1024).toFixed(1)} KiB
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                </details>
              ) : (
                <p className="aesthetic-help">历史调用未记录实际发送尺寸。</p>
              )}
              <details>
                <summary>完整调用记录</summary>
                <pre>{JSON.stringify(a, null, 2)}</pre>
              </details>
              {a.receipt && (
                <p className="aesthetic-help">
                  输入 {a.receipt.usage.input_tokens ?? "未知"} tokens ·{" "}
                  缓存读取 {a.receipt.usage.cached_input_tokens ?? "未知"} /
                  写入 {a.receipt.usage.cache_write_tokens ?? "未知"} tokens
                  {` · 费用 ${a.receipt.usage.cost_usd != null ? `$${a.receipt.usage.cost_usd.toFixed(6)}` : "未知"}`}
                  {` · 上游 ${a.receipt.usage.upstream_provider ?? "未知"} · 实际层级 ${a.receipt.usage.service_tier ?? "未知"}`}
                </p>
              )}
            </article>
          ))}
        </div>
      )}
      {imageOrdinal != null && (
        <EvidenceViewer
          context={context}
          candidates={ordered.map((m) => m.candidate)}
          initialOrdinal={imageOrdinal}
          onClose={() => setImageOrdinal(null)}
        />
      )}
      {deferring && (
        <BatchRecoveryDialog
          context={context}
          stage={stage}
          action="defer"
          batches={[batch.sequence]}
          onClose={() => setDeferring(false)}
          onChanged={() => {
            void perform(async () => {});
          }}
        />
      )}
    </details>
  );
}

export function EvidenceViewer({
  context,
  candidates,
  initialOrdinal,
  onClose,
}: {
  context: ModuleContext;
  candidates: Schema["AestheticCandidate"][];
  initialOrdinal: number;
  onClose: () => void;
}) {
  const [ordinal, setOrdinal] = useState(initialOrdinal);
  const index = Math.max(
    0,
    candidates.findIndex((c) => c.ordinal === ordinal),
  );
  const candidate = candidates[index];
  if (!candidate) return null;
  return (
    <WorkbenchDialog title="评审图片大图" onClose={onClose}>
      <div className="aesthetic-evidence-viewer">
        <AssetViewer
          key={candidate.ordinal}
          context={context}
          asset={{
            key: candidate.key,
            name: `候选 ${candidate.ordinal + 1}`,
            bytes: String(candidate.bytes),
            extension: "",
            source_name: "",
            selected: false,
            summary: null,
          }}
          title={`候选 ${candidate.ordinal + 1} · ${candidate.rating.toUpperCase()}`}
          backLabel="关闭评审大图"
          onGrid={onClose}
          disabled={false}
          previous={index > 0}
          next={index < candidates.length - 1}
          onNavigate={(delta) => {
            const next = candidates[index + delta];
            if (next) setOrdinal(next.ordinal);
          }}
        />
      </div>
    </WorkbenchDialog>
  );
}
