import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { StudioClient } from "@studio/client";
import { Button, ErrorDetails } from "@studio/ui";
import { lakeKey, useLakeRefresh } from "./queries.js";
import { bytesLabel, dateLabel } from "./model.js";
import { PinterestStreams } from "./PinterestStreams.js";
import {
  pinterestAction,
  pinterestActions,
  pinterestActive,
  pinterestCount,
  pinterestRange,
  pinterestReason,
  pinterestStates,
  pinterestTaskLabels,
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
        {(job.metrics.manifest_discrepancies ?? 0) > 0 && (
          <p role="status">
            列表与详情出现 {job.metrics.manifest_discrepancies}{" "}
            处媒体差异。后续已转为详情确认，先前保存的列表资产仍需结合新观察复核。
          </p>
        )}
        {job.state === "waiting_budget" && (
          <p className="lake-hint">
            本轮预算已用完，现有原图、候选和游标已保留。追加一轮预算后从原进度继续；恢复暂停不会自动重置预算。
          </p>
        )}
      </details>
      <details open>
        <summary>采集进度</summary>
        <dl className="wb-property-list">
          <dt>累计准入 Pin</dt>
          <dd>{job.totals.admitted_pins ?? 0}</dd>
          <dt>已归档原图记录</dt>
          <dd>{pinterestCount(job, ["done"], "media_download")}</dd>
          <dt>去重后新增文件</dt>
          <dd>{job.metrics.new_byte_objects ?? 0}</dd>
          <dt>HTTP 核验复用</dt>
          <dd>{job.metrics.http_validated ?? 0}</dd>
          <dt>直接历史复用</dt>
          <dd>{job.metrics.historical_reuse ?? 0}</dd>
          <dt>未获取或待检查</dt>
          <dd>{pinterestCount(job, ["needs_review", "unavailable"])}</dd>
          <dt>累计下载</dt>
          <dd>{bytesLabel(job.download_bytes)}</dd>
          <dt>元数据补取</dt>
          <dd>
            {pinterestCount(job, ["done"], "pin_enrichment")} 已完成 ·{" "}
            {job.enrichment_pending} 待办或未取得
          </dd>
          <dt>采集范围</dt>
          <dd>
            {job.media_complete ? "媒体范围已处理" : "查看剩余候选与缺口"}
          </dd>
          <dt>数据湖发布</dt>
          <dd>
            {job.archive_seq === job.served_seq
              ? "已同步，可在项目中浏览"
              : `还有 ${job.archive_seq - job.served_seq} 批待发布`}
          </dd>
          <dt>第 {job.budget_round} 轮来源请求</dt>
          <dd>
            {job.budget_usage.api_requests ?? 0} /{" "}
            {job.definition.run_budget.api_requests}
          </dd>
          <dt>本轮详情请求</dt>
          <dd>
            {job.budget_usage.detail_requests ?? 0} /{" "}
            {job.definition.run_budget.detail_requests}
          </dd>
          <dt>本轮下载</dt>
          <dd>
            {bytesLabel(job.budget_usage.download_bytes ?? 0)} /{" "}
            {bytesLabel(job.definition.run_budget.download_bytes)}
          </dd>
          <dt>本轮运行时间</dt>
          <dd>
            {Math.round(job.budget_usage.elapsed_seconds ?? 0)} /{" "}
            {job.definition.run_budget.wall_seconds} 秒
          </dd>
          <dt>创建时间</dt>
          <dd>{dateLabel(job.created_at)}</dd>
        </dl>
        <p className="lake-hint">
          多个 Pin 可关联同一文件。原图记录数保留每个 Pin
          的关系；未支持媒体和获取失败会保留缺口。
        </p>
        {job.definition.metadata.detail_enrichment === "none" && (
          <p className="lake-hint">此任务未抽样校验由列表形成的清单。</p>
        )}
      </details>
      <PinterestStreams client={client} job={job} />
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
                "waiting_budget",
                "superseded",
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
              {pinterestTaskLabels[item.kind] ?? item.kind} ·{" "}
              {pinterestStates[item.state] ?? item.state}
            </strong>
            <small>
              {item.pin_id} · 尝试 {item.attempts} 次
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
