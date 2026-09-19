import { useEffect, useState } from "react";
import type { Schema } from "@studio/contracts";
import type { ModuleContext } from "@studio/ui";

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
  outcome_unknown: "结果不明",
};
export const stateLabel = (state: string) => labels[state] ?? state;
export function CandidateCard({
  context,
  candidate,
  label,
  historical = false,
  nominated = false,
}: {
  context: ModuleContext;
  candidate: Schema["AestheticCandidate"];
  label?: string;
  historical?: boolean;
  nominated?: boolean;
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
      </div>
      <figcaption>
        <b>{label ?? `候选 ${candidate.ordinal + 1}`}</b>
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
}: {
  context: ModuleContext;
  stage: Schema["AestheticStage"];
  batch: Schema["AestheticBatch"];
  disabled: boolean;
  perform: (action: () => Promise<unknown>) => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const [attempts, setAttempts] = useState<Schema["AestheticAttempts"] | null>(
    null,
  );
  const [confirm, setConfirm] = useState(false);
  return (
    <details
      className="aesthetic-batch"
      onToggle={(e) => setOpen(e.currentTarget.open)}
    >
      <summary>
        批次 {batch.sequence} · {batch.rating.toUpperCase()} ·{" "}
        {batch.members.length} 图 · {stateLabel(batch.state)}
      </summary>
      {batch.error && <p>{batch.error}</p>}
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
        <div className="aesthetic-images">
          {batch.members.map((m) => (
            <CandidateCard
              historical
              nominated={
                batch.observation?.elite_candidates.includes(m.label) ?? false
              }
              key={m.label}
              context={context}
              candidate={m.candidate}
              label={m.label}
            />
          ))}
        </div>
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
        {["failed", "invalid", "outcome_unknown"].includes(batch.state) && (
          <button disabled={disabled} onClick={() => setConfirm(true)}>
            重新评审此批
          </button>
        )}
      </div>
      {confirm && (
        <div className="aesthetic-notice">
          <p>
            会新增一次 API
            调用；先前结果不明的请求也可能已经计费。重试保留历史尝试，同一批只接受一份有效结果。
          </p>
          <button
            disabled={disabled}
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
      {attempts && <pre>{JSON.stringify(attempts.items, null, 2)}</pre>}
    </details>
  );
}
