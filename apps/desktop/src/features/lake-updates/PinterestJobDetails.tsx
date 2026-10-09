import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { StudioClient } from "@studio/client";
import { Button, ErrorDetails } from "@studio/ui";
import { lakeKey, useLakeRefresh } from "./queries.js";
import { bytesLabel, dateLabel } from "./model.js";
import {
  pinterestAction,
  pinterestActions,
  pinterestActive,
  pinterestCount,
  pinterestRange,
  pinterestReason,
  pinterestStates,
  type PinterestJob,
} from "./pinterestModel.js";

export function PinterestJobDetails({
  client,
  job,
}: {
  client: StudioClient;
  job: PinterestJob;
}) {
  const refresh = useLakeRefresh(client);
  const [cursors, setCursors] = useState<string[]>([""]),
    [filter, setFilter] = useState("");
  const [pending, setPending] = useState(false),
    [error, setError] = useState<unknown>(null);
  const items = useQuery({
    queryKey: [
      ...lakeKey(client),
      "pinterest-items",
      job.id,
      filter,
      cursors.at(-1),
    ],
    queryFn: ({ signal }) =>
      client.pinterestCollections.items(job.id, {
        state: filter || undefined,
        cursor: cursors.at(-1) || undefined,
        limit: 30,
        signal,
      }),
    refetchInterval: pinterestActive(job) ? 4000 : false,
  });
  async function act(action: string) {
    setPending(true);
    setError(null);
    try {
      await pinterestAction(client, job.id, action);
    } catch (e) {
      setError(e);
    } finally {
      await refresh();
      setPending(false);
    }
  }
  return (
    <div className="lake-details">
      <details open>
        <summary>Pinterest · {pinterestStates[job.state] ?? job.state}</summary>
        <p>{pinterestRange(job.definition)}</p>
        <p className="lake-hint">静态原图 · 匿名访问</p>
        <div className="lake-actions">
          {job.actions.map((action) => (
            <Button
              key={action}
              disabled={pending}
              onClick={() => void act(action)}
            >
              {pinterestActions[action] ?? action}
            </Button>
          ))}
        </div>
        {error != null && <ErrorDetails error={error} />}
        {job.error_code && job.state !== "waiting_budget" && (
          <p role="status">
            {pinterestReason(job.error_message ?? job.error_code)}
          </p>
        )}
        {job.state === "waiting_budget" && (
          <p className="lake-hint">
            本轮预算已用完，现有原图和响应已保留。可取消剩余工作，为未完成 Pin
            新建任务。
          </p>
        )}
      </details>
      <details open>
        <summary>采集进度</summary>
        <dl className="wb-property-list">
          <dt>Pin 详情</dt>
          <dd>
            {pinterestCount(job, ["done"], "pin_detail")} /{" "}
            {job.definition.seeds.length}
          </dd>
          <dt>已归档原图记录</dt>
          <dd>{pinterestCount(job, ["done"], "media_download")}</dd>
          <dt>未获取或待检查</dt>
          <dd>{pinterestCount(job, ["needs_review", "unavailable"])}</dd>
          <dt>本轮下载</dt>
          <dd>{bytesLabel(job.download_bytes)}</dd>
          <dt>数据湖发布</dt>
          <dd>
            {job.archive_seq === job.served_seq
              ? "已同步，可在项目中浏览"
              : `还有 ${job.archive_seq - job.served_seq} 批待发布`}
          </dd>
          <dt>详情请求</dt>
          <dd>
            {job.api_requests} / {job.definition.run_budget.api_requests}
          </dd>
          <dt>下载预算</dt>
          <dd>{bytesLabel(job.definition.run_budget.download_bytes)}</dd>
          <dt>累计运行时间</dt>
          <dd>
            {Math.round(job.elapsed_seconds)} /{" "}
            {job.definition.run_budget.wall_seconds} 秒
          </dd>
          <dt>创建时间</dt>
          <dd>{dateLabel(job.created_at)}</dd>
        </dl>
        <p className="lake-hint">
          多个 Pin 可关联同一文件。原图记录数保留每个 Pin
          的关系；未支持媒体和获取失败会保留缺口。
        </p>
      </details>
      <details open>
        <summary>Pin 与媒体明细</summary>
        <div className="lake-fields">
          <label>
            任务状态
            <select
              aria-label="Pinterest 任务状态"
              value={filter}
              onChange={(e) => {
                setFilter(e.target.value);
                setCursors([""]);
              }}
            >
              <option value="">全部</option>
              {[
                "queued",
                "running",
                "done",
                "needs_review",
                "unavailable",
                "waiting_retry",
                "cancelled",
              ].map((s) => (
                <option key={s} value={s}>
                  {pinterestStates[s] ?? s}
                </option>
              ))}
            </select>
          </label>
        </div>
        {items.error && <ErrorDetails error={items.error} />}
        {items.data?.items.map((item) => (
          <div className="collection-task" key={item.task_id}>
            <strong>
              {item.kind === "pin_detail" ? "Pin 详情" : "原图获取"} ·{" "}
              {pinterestStates[item.state] ?? item.state}
            </strong>
            <small>
              Pin {item.pin_id} · 尝试 {item.attempts} 次
            </small>
            {item.reason && <p>{pinterestReason(item.reason)}</p>}
          </div>
        ))}
        {!items.isPending && !items.data?.items.length && (
          <p className="lake-hint">没有符合筛选的任务项。</p>
        )}
        <footer className="lake-pagination">
          <Button
            disabled={cursors.length === 1 || items.isFetching}
            onClick={() => setCursors((v) => v.slice(0, -1))}
          >
            上一页
          </Button>
          <span>第 {cursors.length} 页</span>
          <Button
            disabled={!items.data?.next_cursor || items.isFetching}
            onClick={() => setCursors((v) => [...v, items.data!.next_cursor!])}
          >
            下一页
          </Button>
        </footer>
      </details>
    </div>
  );
}
