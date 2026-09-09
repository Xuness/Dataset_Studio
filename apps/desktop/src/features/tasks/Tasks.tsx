import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Download,
  X,
  CheckCircle2,
  LoaderCircle,
  Clock3,
  CircleAlert,
} from "lucide-react";
import {
  Button,
  ErrorDetails,
  JobProgress,
  isJobActive,
  jobPresentation,
  jobSubmittedAt,
} from "@studio/ui";
import type { Job, Schema } from "@studio/contracts";
import type { StudioClient } from "@studio/client";
import { scopeKindLabel } from "../scopes/scopes.js";

type Props = {
  client: StudioClient;
  projectId: string;
  onClose: () => void;
  onError: (message: string) => void;
  onOpenRanking?: (jobId: string) => void;
};
export function Tasks({
  client,
  projectId,
  onClose,
  onError,
  onOpenRanking,
}: Props) {
  const query = useQuery({
    queryKey: ["project", projectId, "jobs"],
    queryFn: () => client.jobs(projectId),
    refetchInterval: 2000,
    refetchIntervalInBackground: true,
  });
  const jobs = [...(query.data?.items ?? [])].sort(
    (a, b) =>
      Number(isJobActive(b)) - Number(isJobActive(a)) ||
      Number(b.created_at) - Number(a.created_at),
  );
  const operators = useQuery({
    queryKey: ["operators", client.connection.instance_id],
    queryFn: ({ signal }) => client.tools.operators(signal),
  });
  const active = jobs.filter(isJobActive).length;
  return (
    <section className="tasks-panel" aria-label="项目任务">
      <header>
        <strong>项目任务</strong>
        <span>
          {active ? active + " 项执行中 · " : ""}
          {jobs.length} 项最近记录
        </span>
        <span className="grow" />
        <Button
          onClick={() => void query.refetch()}
          disabled={query.isFetching}
        >
          刷新
        </Button>
        <button
          className="icon-button"
          aria-label="关闭任务面板"
          onClick={onClose}
        >
          <X size={15} />
        </button>
      </header>
      {query.isError && (
        <div className="tasks-sync-error" role="alert">
          状态同步中断，显示上次记录。连接恢复后会自动更新。
          <ErrorDetails error={query.error.message} compact />
        </div>
      )}
      <div className="tasks-scroll">
        {query.isPending ? (
          <div className="tasks-empty" role="status">
            正在读取项目任务…
          </div>
        ) : !jobs.length && !query.isError ? (
          <div className="tasks-empty">
            尚未提交任务。使用计算工具后，可在这里查看进度、结果或重试失败任务。
          </div>
        ) : (
          jobs.map((job) => (
            <TaskRow
              key={job.id}
              job={job}
              client={client}
              onError={onError}
              onOpenRanking={onOpenRanking}
              disconnected={query.isError}
              operator={operators.data?.items.find(
                (o) => o.id === job.operator,
              )}
            />
          ))
        )}
      </div>
    </section>
  );
}

function TaskRow({
  job,
  client,
  operator,
  onError,
  onOpenRanking,
  disconnected,
}: {
  job: Job;
  client: StudioClient;
  operator: Schema["Operators"]["items"][number] | undefined;
  onError: (message: string) => void;
  onOpenRanking: Props["onOpenRanking"];
  disconnected: boolean;
}) {
  const cache = useQueryClient();
  const [pending, setPending] = useState<string | null>(null);
  const p = jobPresentation(job);
  const Icon = p.succeeded
    ? CheckCircle2
    : job.status === "failed"
      ? CircleAlert
      : p.active
        ? LoaderCircle
        : Clock3;
  async function act(kind: "cancel" | "retry" | "download") {
    if (pending) return;
    setPending(kind);
    try {
      if (kind === "download")
        await client.downloadArtifact(job.project_id, job.id);
      else {
        const updated =
          kind === "cancel"
            ? await client.cancelJob(job.project_id, job.id)
            : await client.tools.retry(job.project_id, job.id);
        cache.setQueryData<Schema["Jobs"]>(
          ["project", job.project_id, "jobs"],
          (data) => ({
            items: (data?.items ?? []).map((j) =>
              j.id === updated.id ? updated : j,
            ),
          }),
        );
        void cache.invalidateQueries({
          queryKey: ["project", job.project_id, "jobs"],
        });
      }
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(null);
    }
  }
  return (
    <article className="task-row" data-status={job.status}>
      <span className={"task-symbol " + job.status}>
        <Icon
          size={18}
          className={p.active && !disconnected ? "loading-icon" : ""}
        />
      </span>
      <div className="task-title">
        <div className="task-heading">
          <strong>
            {operator?.name ??
              (p.ranking ? "Danbooru 元数据排名" : job.operator)}
          </strong>
          <span className={"task-status " + job.status}>{p.status}</span>
        </div>
        <small>
          {scopeKindLabel(job.input_scope)} ·{" "}
          {job.input_members_frozen
            ? "固定输入 " + job.total.toLocaleString("zh-CN") + " 项"
            : "正在确定输入数量"}{" "}
          · 提交于 {jobSubmittedAt(job)}
          {job.attempt > 1 ? " · 第 " + job.attempt + " 次执行" : ""}
        </small>
        <span className="task-stage">{p.title}</span>
        <JobProgress job={job} disconnected={disconnected} />
        {job.error && <ErrorDetails error={job.error} compact />}
      </div>
      <div className="task-actions">
        {p.ranking && onOpenRanking && (
          <Button onClick={() => onOpenRanking(job.id)}>
            {p.succeeded ? "查看排名" : "查看任务"}
          </Button>
        )}
        {p.succeeded && !p.ranking ? (
          <Button
            disabled={!!pending || disconnected}
            onClick={() => void act("download")}
          >
            <Download size={13} />
            {pending === "download" ? "正在保存…" : "保存成果"}
          </Button>
        ) : p.active ? (
          <Button
            disabled={!!pending || disconnected}
            onClick={() => void act("cancel")}
          >
            {pending === "cancel" ? "正在取消…" : "取消任务"}
          </Button>
        ) : ["failed", "cancelled"].includes(job.status) &&
          job.total > 0 &&
          operator?.capabilities.retry ? (
          <Button
            disabled={!!pending || disconnected}
            onClick={() => void act("retry")}
          >
            {pending === "retry" ? "正在重试…" : "重试固定输入"}
          </Button>
        ) : null}
      </div>
    </article>
  );
}
