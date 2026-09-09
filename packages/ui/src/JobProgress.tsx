import type { Job } from "@studio/contracts";
import { jobPresentation } from "./jobPresentation.js";

export function JobProgress({
  job,
  disconnected = false,
}: {
  job: Job;
  disconnected?: boolean;
}) {
  const p = jobPresentation(job);
  return (
    <div
      className="job-progress"
      data-status={job.status}
      data-disconnected={disconnected || undefined}
    >
      <div className="job-progress-label">
        <span>
          {p.progressLabel} · {p.count}
        </span>
        <strong>
          {disconnected
            ? p.percent == null
              ? "同步中断"
              : `${p.percent.toFixed(1)}% · 上次记录`
            : p.percent == null
              ? job.status === "queued"
                ? "等待中"
                : p.active
                  ? "处理中"
                  : "已停止"
              : `${p.percent.toFixed(1)}%`}
        </strong>
      </div>
      <div
        className="job-progress-track"
        role="progressbar"
        aria-label={`${p.title} · ${p.progressLabel}`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={p.percent ?? undefined}
        aria-valuetext={`${disconnected ? "状态同步中断，最后记录：" : ""}${p.title}，${p.count}`}
      >
        <span
          className={
            p.percent == null && p.active && !disconnected
              ? "indeterminate"
              : ""
          }
          style={{
            width:
              p.percent == null ? (p.active ? "28%" : "0%") : `${p.percent}%`,
          }}
        />
      </div>
    </div>
  );
}
