import type { Schema } from "@studio/contracts";
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { StudioClient } from "@studio/client";
import { Button, ErrorDetails, CopyButton } from "@studio/ui";
import {
  active,
  actions,
  actionLabels,
  bytesLabel,
  dateLabel,
  itemStates,
  phases,
  policyLabel,
  rangeLabel,
  states,
} from "./model.js";
import type { UpdateJob } from "./model.js";
import { lakeKey, useLakeRefresh } from "./queries.js";
import { ImagePolicySummary } from "./ImagePolicySummary.js";
const timingLabels: Record<string, string> = {
  connect: "图片连接",
  download: "下载与暂存",
  encode: "校验与编码",
  publish: "图片发布",
  metadata: "元数据获取与归档",
};
export function JobDetails({
  client,
  job,
  onSettings,
}: {
  client: StudioClient;
  job: UpdateJob;
  onSettings: () => void;
}) {
  const refresh = useLakeRefresh(client);
  const [pending, setPending] = useState(false),
    [error, setError] = useState<unknown>(null);
  const [filter, setFilter] = useState("problems"),
    [after, setAfter] = useState<number[]>([0]);
  const [reason, setReason] = useState(""),
    [draftReason, setDraftReason] = useState("");
  const issues = useQuery({
    queryKey: [
      ...lakeKey(client),
      "items",
      job.id,
      filter,
      reason,
      after.at(-1),
    ],
    queryFn: ({ signal }) =>
      client.lakeUpdates.items(job.id, {
        signal,
        status: filter || undefined,
        reason: reason || undefined,
        after: after.at(-1),
        limit: 50,
      }),
    refetchInterval: active(job) ? 5000 : false,
  });
  const coverage = useQuery({
    queryKey: [
      ...lakeKey(client),
      "coverage",
      job.id,
      job.state,
      job.cursor.metadata_complete,
    ],
    queryFn: ({ signal }) => client.lakeUpdates.coverage(job.id, signal),
  });
  const t: Partial<Schema["LakeUpdateTelemetry"]> = job.telemetry ?? {};
  const fresh = t.sampled_at && Date.now() - Date.parse(t.sampled_at) < 15000;
  const source = coverage.data?.coverage as
    Record<string, unknown> | null | undefined;
  const ratio =
    fresh && t.current_total_bytes && t.current_bytes != null
      ? Math.min(100, (100 * t.current_bytes) / t.current_total_bytes)
      : null;
  return (
    <div className="lake-details">
      <details open>
        <summary>执行状态</summary>
        <dl className="wb-property-list">
          <dt>状态</dt>
          <dd>
            {states[job.state]}
            {job.execution_active && ["paused", "cancelled"].includes(job.state)
              ? " · 等待当前批次退出"
              : ""}
          </dd>
          <dt>阶段</dt>
          <dd>{phases[t.phase ?? ""] ?? t.phase ?? "—"}</dd>
          <dt>元数据范围</dt>
          <dd>{job.cursor.metadata_complete ? "获取完成" : "尚未完成"}</dd>
          <dt>已扫描页数</dt>
          <dd>{job.cursor.pages}</dd>
          <dt>正在处理帖子</dt>
          <dd>{t.current_post_id ?? "—"}</dd>
          <dt>累计下载</dt>
          <dd>{bytesLabel(t.downloaded_bytes)}</dd>
          <dt>整体下载速度</dt>
          <dd>
            {fresh && active(job)
              ? bytesLabel(t.download_rate_bps) + "/s"
              : "—"}
          </dd>
          {t.publish_rate_images_per_second != null && (
            <>
              <dt>入湖速度</dt>
              <dd>
                {fresh && active(job)
                  ? `${t.publish_rate_images_per_second.toFixed(2)} 图/秒`
                  : "—"}
              </dd>
              <dt>统计窗口</dt>
              <dd>近 {Math.round(t.rate_window_seconds ?? 0)} 秒</dd>
            </>
          )}
          <dt>遥测时间</dt>
          <dd>
            {dateLabel(t.sampled_at)}
            {t.sampled_at && !fresh && active(job) ? "（非实时）" : ""}
          </dd>
          {Object.entries(job.counts).map(([k, v]) => (
            <div className="lake-property-pair" key={k}>
              <dt>{itemStates[k] ?? k}</dt>
              <dd>{v}</dd>
            </div>
          ))}
        </dl>
        {t.active_downloads != null && (
          <section aria-label="图片处理流水线" className="lake-summary">
            <dl className="wb-property-list">
              <dt>等待下载</dt>
              <dd>{t.waiting_download ?? 0}</dd>
              <dt>下载中</dt>
              <dd>{t.active_downloads}</dd>
              <dt>等待编码</dt>
              <dd>{t.waiting_encode ?? 0}</dd>
              <dt>编码中</dt>
              <dd>{t.active_encodes ?? 0}</dd>
              <dt>等待发布</dt>
              <dd>{t.ready_images ?? 0}</dd>
              <dt>API 请求累计</dt>
              <dd>{t.api_requests ?? 0}</dd>
              <dt>图片请求累计</dt>
              <dd>{t.image_requests ?? 0}</dd>
              <dt>远端限流响应</dt>
              <dd>{t.throttled_requests ?? 0}</dd>
            </dl>
            <p className="lake-hint">
              下载中包含连接与限流等待；入湖速度按成功发布的图片记录统计，复用和异常单独计数。
              旧任务升级前缺失的请求计数与阶段耗时不回填。
            </p>
            {(t.files ?? []).length > 0 && (
              <details open>
                <summary>在途图片</summary>
                <div className="lake-item-list">
                  {t.files!.map((f) => (
                    <div key={f.post_id}>
                      <span>#{f.post_id}</span>
                      <span>{phases[f.phase] ?? f.phase}</span>
                      {f.phase === "downloading" && (
                        <small>
                          {bytesLabel(f.current_bytes)} /{" "}
                          {bytesLabel(f.current_total_bytes)}
                        </small>
                      )}
                    </div>
                  ))}
                </div>
              </details>
            )}
            {Object.keys(t.timings_seconds ?? {}).length > 0 && (
              <details>
                <summary>阶段耗时累计</summary>
                <dl className="wb-property-list">
                  {Object.entries(t.timings_seconds!).map(([k, seconds]) => (
                    <div className="lake-property-pair" key={k}>
                      <dt>{timingLabels[k] ?? k}</dt>
                      <dd>{seconds.toFixed(1)} 秒</dd>
                    </div>
                  ))}
                </dl>
                <p className="lake-hint">
                  各工作项耗时之和；并行执行时可超过任务实际经过时间。
                </p>
              </details>
            )}
          </section>
        )}
        {ratio != null && (
          <label className="lake-file-progress">
            当前文件 {ratio.toFixed(0)}%<progress max={100} value={ratio} />
          </label>
        )}
        {job.error_message && (
          <p role="status">
            {job.error_code === "UPDATE_BUDGET"
              ? "本轮预算已用完，继续即可接着处理原范围。"
              : job.error_message}
          </p>
        )}
        {job.state === "waiting_credentials" && (
          <Button onClick={onSettings}>打开数据湖 API 设置</Button>
        )}
        <div className="lake-actions">
          {actions(job)
            .filter((a) => a !== "replay")
            .map((a) => (
              <Button
                key={a}
                disabled={pending}
                onClick={() => {
                  if (
                    a === "cancel" &&
                    !window.confirm("停止后续工作？已经发布的数据会继续保留。")
                  )
                    return;
                  setPending(true);
                  setError(null);
                  void client.lakeUpdates
                    .action(job.id, a)
                    .then(refresh)
                    .catch(setError)
                    .finally(() => setPending(false));
                }}
              >
                {actionLabels[a]}
              </Button>
            ))}
        </div>
        <details>
          <summary>诊断与恢复</summary>
          <p>任务 {job.id}</p>
          <CopyButton text={job.id} />
          {job.error_code && <p>错误代码：{job.error_code}</p>}
          {actions(job).includes("replay") && (
            <Button
              disabled={pending}
              onClick={() => {
                setPending(true);
                setError(null);
                void client.lakeUpdates
                  .action(job.id, "replay")
                  .then(refresh)
                  .catch(setError)
                  .finally(() => setPending(false));
              }}
            >
              使用已保存响应重新解析
            </Button>
          )}
        </details>
        {error != null && <ErrorDetails error={error} />}
      </details>
      <details>
        <summary>范围与图片策略</summary>
        <dl className="wb-property-list">
          <dt>范围</dt>
          <dd>{rangeLabel(job.definition)}</dd>
          <dt>保存策略</dt>
          <dd>{policyLabel(job.definition)}</dd>
          <dt>已有图片</dt>
          <dd>
            {job.definition.media?.existing === "match_profile"
              ? "补入所选版本"
              : "保留可复用版本"}
          </dd>
          <dt>创建时间</dt>
          <dd>{dateLabel(job.created_at)}</dd>
          <dt>最后更新</dt>
          <dd>{dateLabel(job.updated_at)}</dd>
          <dt>重试时间</dt>
          <dd>
            {job.retry_at
              ? dateLabel(new Date(job.retry_at * 1000).toISOString())
              : "—"}
          </dd>
        </dl>
        <ImagePolicySummary policy={job.definition.media} />
      </details>
      <details>
        <summary>已验证范围</summary>
        {coverage.error ? (
          <ErrorDetails error={coverage.error} />
        ) : source ? (
          <dl className="wb-property-list">
            <dt>元数据覆盖</dt>
            <dd>{source.metadata_complete ? "已确认" : "尚未完成"}</dd>
            <dt>图片覆盖</dt>
            <dd>{source.media_complete ? "已确认" : "仍有缺口或未完成"}</dd>
            <dt>未获取项</dt>
            <dd>{String(source.exceptions ?? "—")}</dd>
            <dt>检查时间</dt>
            <dd>{dateLabel(String(source.checked_at ?? ""))}</dd>
          </dl>
        ) : (
          <p>尚无已验证的覆盖记录。</p>
        )}
        <p className="lake-hint">
          仅代表此任务的范围；不代表整个数据湖的完整度。
        </p>
      </details>
      <details open>
        <summary>帖子与问题</summary>
        <div className="lake-fields">
          <label>
            记录范围
            <select
              value={filter}
              onChange={(e) => {
                setFilter(e.target.value);
                setAfter([0]);
              }}
            >
              <option value="problems">未获取或需检查</option>
              <option value="">全部记录</option>
              <option value="pending">待下载</option>
              <option value="stored">已保存</option>
              <option value="failed">失败</option>
              <option value="unavailable">未获取</option>
            </select>
          </label>
          <label>
            问题代码
            <input
              value={draftReason}
              onChange={(e) => setDraftReason(e.target.value)}
              placeholder="可选，精确匹配"
            />
          </label>
          <Button
            onClick={() => {
              setReason(draftReason.trim());
              setAfter([0]);
            }}
          >
            筛选问题
          </Button>
        </div>
        {issues.error && <ErrorDetails error={issues.error} />}
        <div className="lake-item-list">
          {issues.data?.items.map((i) => (
            <div key={i.post_id}>
              <strong>#{i.post_id}</strong>
              <span>{itemStates[i.state] ?? i.state}</span>
              <small>{i.reason}</small>
            </div>
          ))}
        </div>
        {!issues.isPending && !issues.error && !issues.data?.items.length && (
          <p>当前筛选没有记录。</p>
        )}
        <div className="lake-actions">
          <Button
            disabled={after.length === 1 || issues.isFetching}
            onClick={() => setAfter((v) => v.slice(0, -1))}
          >
            上一页记录
          </Button>
          <Button
            disabled={!issues.data?.next_cursor || issues.isFetching}
            onClick={() => setAfter((v) => [...v, issues.data!.next_cursor!])}
          >
            下一页记录
          </Button>
        </div>
      </details>
    </div>
  );
}
