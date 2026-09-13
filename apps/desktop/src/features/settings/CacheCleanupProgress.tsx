import { useEffect, useState } from "react";
import type { Schema } from "@studio/contracts";

const phases: Record<string, string> = {
  queued: "等待后台处理",
  waiting: "等待当前写入完成后继续",
  planning: "正在确定可回收的缓存",
  indexes: "正在清理排名索引",
  deleting: "正在分批删除成员记录",
  accounting: "正在更新空间统计",
  compacting: "正在归还空闲磁盘空间",
  completed: "清理完成",
  failed: "清理失败",
};
export const cleanupPhase = (phase: string) => phases[phase] ?? "正在整理缓存";
export const cleanupActive = (phase: string) =>
  ["queued", "deleting", "accounting"].includes(phase);

export function CacheCleanupProgress({
  task,
  label,
  retry,
}: {
  task: Schema["CacheCleanup"];
  label: string;
  retry: () => void;
}) {
  const [now, setNow] = useState(Date.now());
  const active = cleanupActive(task.state);
  useEffect(() => {
    if (!active) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [active]);
  const seconds = Math.max(
    0,
    Math.floor(
      ((active ? now : Number(task.updated_millis)) -
        Number(task.started_millis)) /
        1000,
    ),
  );
  const elapsed =
    seconds < 60
      ? `${seconds} 秒`
      : `${Math.floor(seconds / 60)} 分 ${seconds % 60} 秒`;
  return (
    <div className="settings-cleanup-progress" role="status" aria-live="polite">
      <strong>
        {label} · {cleanupPhase(task.state)}
      </strong>
      <div>
        已处理 {task.processed.toLocaleString()} / {task.total.toLocaleString()}{" "}
        条，已移除 {task.removed.toLocaleString()} 条 · {elapsed}
      </div>
      {active && (
        <progress
          aria-label={`${label} 清理进度`}
          max={Math.max(1, task.total)}
          value={task.processed}
        />
      )}
      {task.state === "accounting" && (
        <small>成员处理已结束，正在汇总可用空间。</small>
      )}
      {task.state === "completed" && (
        <small>空间已可供项目复用；磁盘文件会在后台逐步缩小。</small>
      )}
      {task.error && (
        <>
          <p className="error">{task.error}</p>
          <button onClick={retry}>重试清理</button>
        </>
      )}
    </div>
  );
}
