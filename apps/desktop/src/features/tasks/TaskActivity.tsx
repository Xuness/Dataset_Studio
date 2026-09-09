import { CheckCircle2, CircleAlert, Clock3, LoaderCircle } from "lucide-react";
import type { Job } from "@studio/contracts";
import { jobPresentation, isJobActive } from "@studio/ui";

export function TaskActivity({
  jobs,
  disconnected,
  onOpen,
  expanded,
}: {
  jobs: Job[];
  disconnected: boolean;
  onOpen: () => void;
  expanded: boolean;
}) {
  const active = jobs.filter(isJobActive);
  const latest = active[0] ?? jobs[0];
  const p = latest ? jobPresentation(latest) : null;
  const Icon =
    disconnected || latest?.status === "failed"
      ? CircleAlert
      : p?.active
        ? LoaderCircle
        : p?.succeeded
          ? CheckCircle2
          : Clock3;
  return (
    <button
      className="task-activity"
      data-status={disconnected ? "disconnected" : latest?.status}
      onClick={onOpen}
      aria-expanded={expanded}
      title={
        disconnected
          ? "任务状态同步中断，点击打开项目任务"
          : p
            ? `${p.title} · ${p.count}；点击打开项目任务`
            : "打开项目任务"
      }
    >
      <Icon
        size={14}
        className={p?.active && !disconnected ? "loading-icon" : ""}
      />
      <span className="task-activity-label">
        {disconnected
          ? "任务状态同步中断"
          : p
            ? `${active.length > 1 ? `${active.length} 项执行中 · ` : ""}${p.title}`
            : "项目任务"}
      </span>
      {p?.active && (
        <span className="task-activity-meter" aria-hidden="true">
          <i style={{ width: p.percent == null ? "28%" : `${p.percent}%` }} />
        </span>
      )}
      {p?.active && p.percent != null && (
        <span>当前步骤 {p.percent.toFixed(1)}%</span>
      )}
    </button>
  );
}
