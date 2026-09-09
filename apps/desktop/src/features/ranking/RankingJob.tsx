import { useEffect, useState } from "react";
import { CheckCircle2, Clock3, CircleAlert, LoaderCircle } from "lucide-react";
import type { Job } from "@studio/contracts";
import {
  Button,
  ErrorDetails,
  JobProgress,
  jobPresentation,
  jobSubmittedAt,
  jobPhaseLabel,
  jobDuration,
  rankingPhases,
} from "@studio/ui";

export function RankingJob({
  job,
  submitting,
  loading,
  syncError,
  action,
  resultReady,
  resultError,
  onCancel,
  onRetry,
  onRefresh,
  onResult,
}: {
  job: Job | undefined;
  submitting: boolean;
  loading: boolean;
  syncError: string | undefined;
  action: "cancel" | "retry" | null;
  resultReady: boolean;
  resultError: boolean;
  onCancel: () => void;
  onRetry: () => void;
  onRefresh: () => void;
  onResult: () => void;
}) {
  const p = job ? jobPresentation(job) : null;
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (!p?.active) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [p?.active]);
  const seconds = job
    ? Math.max(0, Math.floor((now - Number(job.created_at)) / 1000))
    : 0;
  const elapsed =
    seconds < 60
      ? `${seconds} 秒`
      : seconds < 3600
        ? `${Math.floor(seconds / 60)} 分 ${seconds % 60} 秒`
        : `${Math.floor(seconds / 3600)} 小时 ${Math.floor((seconds % 3600) / 60)} 分`;
  const Icon = p?.succeeded
    ? CheckCircle2
    : job?.status === "failed"
      ? CircleAlert
      : p?.active || submitting || loading
        ? LoaderCircle
        : Clock3;
  const trace = job?.stage?.telemetry;
  const runElapsed = trace
    ? Number(trace.finished_at ?? now) - Number(trace.started_at)
    : null;
  const phases = [...(trace?.phases ?? [])];
  if (
    p?.active &&
    job?.stage &&
    !phases.some(
      (phase) =>
        phase.name === job.stage?.name &&
        (phase.rating ?? null) === (job.stage?.rating ?? null),
    )
  )
    phases.push({
      name: job.stage.name,
      rating: job.stage.rating ?? null,
      elapsed_ms: 0,
    });
  return (
    <section
      className="ranking-job"
      data-status={job?.status ?? "submitting"}
      aria-label="排名任务进度"
    >
      <div className="ranking-job-heading">
        <Icon
          size={18}
          className={
            (p?.active || submitting || loading) && !syncError
              ? "loading-icon"
              : ""
          }
        />
        <strong role="status" aria-live="polite">
          {submitting
            ? "正在提交排名任务…"
            : (p?.title ??
              (loading ? "正在读取任务状态…" : "暂未找到任务记录"))}
        </strong>
        {p && <span className="ranking-job-state">{p.status}</span>}
        <span className="grow" />
        {p?.active && (
          <Button disabled={!!action || !!syncError} onClick={onCancel}>
            {action === "cancel" ? "正在取消…" : "取消任务"}
          </Button>
        )}
        {job && ["failed", "cancelled"].includes(job.status) && (
          <Button disabled={!!action || !!syncError} onClick={onRetry}>
            {action === "retry" ? "正在重试…" : "重试固定输入"}
          </Button>
        )}
        {p?.succeeded && (
          <Button
            className="primary"
            disabled={!resultReady && !resultError}
            onClick={resultReady ? onResult : onRefresh}
          >
            {resultReady
              ? "查看结果"
              : resultError
                ? "重新加载结果"
                : "正在加载结果…"}
          </Button>
        )}
      </div>
      {job && p && (
        <>
          <div className="ranking-job-meta">
            <span>
              {job.input_members_frozen
                ? `固定输入 ${job.total.toLocaleString("zh-CN")} 项`
                : "正在确定输入数量"}
            </span>
            <span>提交于 {jobSubmittedAt(job)}</span>
            {trace && runElapsed != null && Number.isFinite(runElapsed) ? (
              <span>本次执行 {jobDuration(runElapsed)}</span>
            ) : (
              p.active &&
              Number.isFinite(seconds) && <span>已提交 {elapsed}</span>
            )}
            {p.active && trace && (
              <span>
                引擎响应：{jobDuration(now - Number(trace.heartbeat_at))}前
              </span>
            )}
            {job.attempt > 1 && <span>第 {job.attempt} 次执行</span>}
          </div>
          {p.active && (
            <ol className="ranking-job-phases" aria-label="排名计算阶段">
              {rankingPhases.map((name, i) => (
                <li
                  key={name}
                  data-phase={
                    i < p.phase ? "done" : i === p.phase ? "current" : "pending"
                  }
                  aria-current={i === p.phase ? "step" : undefined}
                >
                  <span>{i < p.phase ? "✓" : i + 1}</span>
                  {name}
                </li>
              ))}
            </ol>
          )}
          <JobProgress job={job} disconnected={!!syncError} />
          <p className="ranking-job-note">
            {p.detail}
            {p.active && " 可以切换页面，任务会继续执行。"}
          </p>
          {job.error && <ErrorDetails error={job.error} compact />}
          {trace && (
            <details className="ranking-job-timings">
              <summary>阶段用时（含等待）</summary>
              <dl>
                {phases.map((phase) => (
                  <div key={`${phase.name}:${phase.rating ?? ""}`}>
                    <dt>{jobPhaseLabel(phase.name, phase.rating)}</dt>
                    <dd>
                      {jobDuration(
                        phase.elapsed_ms +
                          (p.active &&
                          phase.name === job.stage?.name &&
                          (phase.rating ?? null) === (job.stage?.rating ?? null)
                            ? Math.max(0, now - Number(trace.updated_at))
                            : 0),
                      )}
                    </dd>
                  </div>
                ))}
              </dl>
            </details>
          )}
        </>
      )}
      {!job && !submitting && !loading && (
        <p className="ranking-job-note">
          可打开项目任务查看最近的执行记录，或刷新状态。
        </p>
      )}
      {syncError && (
        <div className="ranking-job-sync" role="alert">
          <span>状态同步中断，当前显示上次记录。连接恢复后会自动更新。</span>
          <Button onClick={onRefresh}>刷新状态</Button>
          <ErrorDetails error={syncError} compact />
        </div>
      )}
    </section>
  );
}
