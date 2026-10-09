import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Download,
  FolderOpen,
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
  MoreMenu,
} from "@studio/ui";
import type { Job, Schema, ManagedObject } from "@studio/contracts";
import type { StudioClient, ObjectTarget } from "@studio/client";
import { scopeKindLabel } from "../scopes/scopes.js";

type Props = {
  client: StudioClient;
  projectId: string;
  onClose: () => void;
  onError: (message: string) => void;
  onOpenRanking?: (jobId: string) => void;
  focusJobId?: string | null;
  onManage?: (
    target: ObjectTarget,
    mode?: "details" | "rename" | "remove",
  ) => void;
};
export function Tasks({
  client,
  projectId,
  onClose,
  onError,
  onOpenRanking,
  focusJobId,
  onManage,
}: Props) {
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState("");
  const [cursor, setCursor] = useState<string | null>(null);
  const [previous, setPrevious] = useState<(string | null)[]>([]);
  useEffect(() => {
    setSearch(focusJobId ?? "");
    setFilter("");
    setCursor(null);
    setPrevious([]);
  }, [focusJobId, projectId]);
  function reset() {
    setCursor(null);
    setPrevious([]);
  }
  const includeArchived =
    filter === "archived" ||
    filter === "deleted" ||
    (!!focusJobId && search === focusJobId);
  const query = useQuery({
    queryKey: [
      "project",
      projectId,
      "job-history",
      search,
      filter,
      cursor,
      includeArchived,
    ],
    queryFn: ({ signal }) =>
      client.management.jobs(projectId, {
        search,
        state: filter,
        include_archived: includeArchived,
        cursor,
        order: "created_desc",
        limit: 32,
        signal,
      }),
    refetchInterval: (value) =>
      value.state.data?.items.some((item) => isJobActive(item.job))
        ? 2000
        : 5000,
    refetchIntervalInBackground: true,
  });
  const jobs = [...(query.data?.items ?? [])].sort(
    (a, b) =>
      Number(isJobActive(b.job)) - Number(isJobActive(a.job)) ||
      Number(b.job.created_at) - Number(a.job.created_at),
  );
  const operators = useQuery({
    queryKey: ["operators", client.connection.instance_id],
    queryFn: ({ signal }) => client.tools.operators(signal),
  });
  const active = jobs.filter((item) => isJobActive(item.job)).length;
  return (
    <section className="tasks-panel" aria-label="项目任务">
      <header>
        <strong>项目任务</strong>
        <span>
          {active ? active + " 项排队或执行中 · " : ""}
          本页 {jobs.length} 项记录
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
      <div className="object-list-tools task-list-tools">
        <input
          type="search"
          aria-label="搜索任务记录"
          placeholder="搜索任务名称、备注或身份"
          value={search}
          maxLength={120}
          onChange={(event) => {
            setSearch(event.target.value);
            reset();
          }}
        />
        <select
          aria-label="任务状态筛选"
          value={filter}
          onChange={(event) => {
            setFilter(event.target.value);
            reset();
          }}
        >
          <option value="">未归档记录</option>
          <option value="active">执行中与排队中</option>
          <option value="succeeded">已完成</option>
          <option value="failed">失败</option>
          <option value="cancelled">已取消</option>
          <option value="archived">已归档</option>
          <option value="deleted">已清理记录</option>
        </select>
        {(cursor || query.data?.next_cursor) && (
          <>
            <Button
              disabled={!cursor || query.isFetching}
              onClick={() => {
                setCursor(previous.at(-1) ?? null);
                setPrevious((old) => old.slice(0, -1));
              }}
            >
              上一页
            </Button>
            <Button
              disabled={!query.data?.next_cursor || query.isFetching}
              onClick={() => {
                setPrevious((old) => [...old, cursor].slice(-64));
                setCursor(query.data!.next_cursor!);
              }}
            >
              更多记录
            </Button>
          </>
        )}
      </div>
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
            {search || filter
              ? "没有匹配的任务记录。可调整名称或状态条件。"
              : "尚无未归档任务。使用计算工具后，可在这里查看进度、结果或重试失败任务。"}
          </div>
        ) : (
          jobs.map(({ job, object, result_available }) => (
            <TaskRow
              key={job.id}
              job={job}
              object={object}
              resultAvailable={result_available}
              client={client}
              onError={onError}
              onOpenRanking={onOpenRanking}
              onManage={onManage}
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
  object,
  resultAvailable,
  client,
  operator,
  onError,
  onOpenRanking,
  disconnected,
  onManage,
}: {
  job: Job;
  object: ManagedObject;
  resultAvailable: boolean;
  client: StudioClient;
  operator: Schema["Operators"]["items"][number] | undefined;
  onError: (message: string) => void;
  onOpenRanking: Props["onOpenRanking"];
  disconnected: boolean;
  onManage: Props["onManage"];
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
  async function act(
    kind: "cancel" | "retry" | "download" | "archive" | "unarchive",
  ) {
    if (pending) return;
    setPending(kind);
    try {
      if (kind === "archive" || kind === "unarchive") {
        await client.management.action(job.project_id, object, {
          action: kind,
          expected_revision: object.revision,
        });
        await cache.invalidateQueries({
          queryKey: ["project", job.project_id],
        });
      } else if (kind === "download")
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
      void cache.invalidateQueries({
        queryKey: ["project", job.project_id, "job-history"],
      });
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(null);
    }
  }
  return (
    <article
      className="task-row"
      data-status={object.state}
      data-job-id={job.id}
    >
      <span className={"task-symbol " + job.status}>
        <Icon
          size={18}
          className={p.active && !disconnected ? "loading-icon" : ""}
        />
      </span>
      <div className="task-title">
        <div className="task-heading">
          <strong>
            {object.name ||
              operator?.name ||
              (p.ranking ? "Danbooru 元数据排名" : job.operator)}
          </strong>
          <span className={"task-status " + job.status}>
            {object.state === "deleted" ? "已清理" : p.status}
            {object.archived ? " · 已归档" : ""}
          </span>
        </div>
        <small>
          {scopeKindLabel(job.input_scope)} ·{" "}
          {object.state === "deleted"
            ? "固定输入已清理"
            : job.input_members_frozen
              ? "固定输入 " + job.total.toLocaleString("zh-CN") + " 项"
              : "正在确定输入数量"}{" "}
          · 提交于 {jobSubmittedAt(job)}
          {job.attempt > 1 ? " · 第 " + job.attempt + " 次执行" : ""}
        </small>
        <span className="task-stage">{p.title}</span>
        {object.state !== "deleted" && (
          <JobProgress job={job} disconnected={disconnected} />
        )}
        {object.notes && <small>{object.notes}</small>}
        {job.error && <ErrorDetails error={job.error} compact />}
      </div>
      <div className="task-actions">
        {p.ranking && onOpenRanking && object.state !== "deleted" && (
          <Button
            disabled={p.succeeded && !resultAvailable}
            onClick={() => onOpenRanking(job.id)}
          >
            {p.succeeded
              ? resultAvailable
                ? "查看排名"
                : "排名已删除"
              : "查看任务"}
          </Button>
        )}
        {object.state === "deleted" ? null : job.operator ===
          "core.export_files" ? (
          <Button
            disabled={!!pending || disconnected}
            onClick={() =>
              void client
                .revealExport(job.project_id, job.id)
                .catch((e: unknown) =>
                  onError(e instanceof Error ? e.message : String(e)),
                )
            }
          >
            <FolderOpen size={13} />
            打开文件夹
          </Button>
        ) : p.succeeded && !p.ranking ? (
          <Button
            disabled={!!pending || disconnected || !resultAvailable}
            onClick={() => void act("download")}
          >
            <Download size={13} />
            {pending === "download"
              ? "正在保存…"
              : resultAvailable
                ? "保存成果"
                : "成果已删除"}
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
        {onManage && (
          <MoreMenu
            label={object.name}
            disabled={!!pending}
            items={[
              { label: "管理与来源详情", action: () => onManage(object) },
              ...(object.state !== "deleted"
                ? [
                    {
                      label: "重命名与备注…",
                      action: () => onManage(object, "rename"),
                    },
                  ]
                : []),
              ...(!p.active && object.state !== "deleted"
                ? [
                    {
                      label: object.archived ? "恢复到任务列表" : "归档此任务",
                      action: () =>
                        void act(object.archived ? "unarchive" : "archive"),
                    },
                  ]
                : []),
              {
                label:
                  object.state === "deleted"
                    ? "检查清理状态…"
                    : "清理任务记录…",
                danger: true,
                action: () => onManage(object, "remove"),
                disabled: p.active,
              },
            ]}
          />
        )}
      </div>
    </article>
  );
}
