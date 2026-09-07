import { useQuery } from "@tanstack/react-query";
import { Download, X, CheckCircle2, LoaderCircle, Clock3 } from "lucide-react";
import { Button } from "@studio/ui";
import type { StudioClient } from "@studio/client";
import { scopeKindLabel } from "../scopes/scopes.js";
export const statusNames: Record<string, string> = {
  waiting_input: "构建输入范围",
  queued: "等待执行",
  preparing: "固定输入",
  running: "运行中",
  succeeded: "已完成",
  failed: "失败",
  cancelled: "已取消",
};
export function Tasks({
  client,
  projectId,
  onClose,
  onError,
}: {
  client: StudioClient;
  projectId: string;
  onClose: () => void;
  onError: (message: string) => void;
}) {
  const query = useQuery({
    queryKey: ["project", projectId, "jobs"],
    queryFn: () => client.jobs(projectId),
    refetchInterval: 5000,
  });
  const jobs = query.data?.items ?? [];
  return (
    <section className="tasks-panel">
      <header>
        <strong>项目任务</strong>
        <span>{jobs.length} 项</span>
        <span className="grow" />
        <button
          className="icon-button"
          aria-label="关闭任务面板"
          onClick={onClose}
        >
          <X size={15} />
        </button>
      </header>
      <div className="tasks-scroll">
        {!jobs.length ? (
          <div className="tasks-empty">
            可以为数据湖、工作集、查询结果或项目选择生成清单。任务会保存固定输入和执行进度。
          </div>
        ) : (
          jobs.map((job) => (
            <div key={job.id} className="task-row">
              <span className={"task-symbol " + job.status}>
                {job.status === "succeeded" ? (
                  <CheckCircle2 size={17} />
                ) : job.status === "running" || job.status === "preparing" ? (
                  <LoaderCircle className="loading-icon" size={17} />
                ) : (
                  <Clock3 size={17} />
                )}
              </span>
              <div className="task-title">
                <strong>数据清单</strong>
                <small>
                  {scopeKindLabel(job.input_scope)} ·{" "}
                  {!job.input_members_frozen
                    ? "成员范围尚未固定"
                    : "固定输入 " + job.total.toLocaleString() + " 项"}{" "}
                  · 第 {job.attempt} 次执行
                  {job.error ? " · " + job.error : ""}
                </small>
              </div>
              <div className="task-progress">
                <div>
                  <i
                    style={{
                      width:
                        (job.total ? (job.completed / job.total) * 100 : 0) +
                        "%",
                    }}
                  />
                </div>
                <span>
                  {!job.input_members_frozen
                    ? job.status === "waiting_input"
                      ? "固定范围中…"
                      : "数量未确定"
                    : job.completed + " / " + job.total}
                </span>
              </div>
              <span className={"task-status " + job.status}>
                {statusNames[job.status] ?? job.status}
              </span>
              {job.status === "succeeded" ? (
                <Button
                  onClick={() =>
                    void client
                      .downloadArtifact(projectId, job.id)
                      .catch((e) => onError(String(e)))
                  }
                >
                  <Download size={13} />
                  保存成果
                </Button>
              ) : ["waiting_input", "queued", "running", "preparing"].includes(
                  job.status,
                ) ? (
                <Button
                  onClick={() =>
                    void client
                      .cancelJob(projectId, job.id)
                      .then(() => query.refetch())
                      .catch((e) => onError(String(e)))
                  }
                >
                  取消
                </Button>
              ) : null}
            </div>
          ))
        )}
      </div>
    </section>
  );
}
