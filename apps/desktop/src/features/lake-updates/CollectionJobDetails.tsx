import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { StudioClient } from "@studio/client";
import { Button, ErrorDetails } from "@studio/ui";
import {
  availableCollectionActions,
  collectionAction,
  collectionActions,
  collectionRange,
  collectionReason,
  collectionStates,
  taskKinds,
  taskStates,
} from "./collectionModel.js";
import type {
  CollectionAction,
  CollectionDefinition,
  CollectionJob,
} from "./collectionModel.js";
import { bytesLabel, dateLabel } from "./model.js";
import { ImagePolicySummary } from "./ImagePolicySummary.js";
import { lakeKey, useLakeRefresh } from "./queries.js";

export function CollectionJobDetails({
  client,
  job,
  onSettings,
  onRecheck,
}: {
  client: StudioClient;
  job: CollectionJob;
  onSettings: () => void;
  onRecheck: (spec: CollectionDefinition) => void;
}) {
  const p = job.progress,
    refresh = useLakeRefresh(client);
  const [cursors, setCursors] = useState<string[]>([""]),
    [state, setState] = useState(job.progress.task_gaps ? "gaps" : ""),
    [kind, setKind] = useState("");
  const [selected, setSelected] = useState<string[]>([]),
    [pending, setPending] = useState(false),
    [error, setError] = useState<unknown>(null);
  const tasks = useQuery({
    queryKey: [
      ...lakeKey(client),
      "collection-tasks",
      job.id,
      state,
      kind,
      cursors.at(-1),
    ],
    queryFn: ({ signal }) =>
      client.sourceCollections.tasks(job.id, {
        state: state || undefined,
        kind: kind || undefined,
        cursor: cursors.at(-1) || undefined,
        limit: 50,
        signal,
      }),
    refetchInterval: job.execution_active ? 4000 : false,
  });
  async function act(action: CollectionAction, ids?: string[]) {
    setPending(true);
    setError(null);
    try {
      await collectionAction(client, job.id, action, ids);
      setSelected([]);
      await refresh();
    } catch (e) {
      setError(e);
    } finally {
      setPending(false);
    }
  }
  const completed =
    job.state === "completed" && p.access_mode === "anonymous"
      ? "本次公开范围已完成"
      : collectionStates[job.state];
  return (
    <div className="lake-details">
      <details open>
        <summary>Pixiv · {completed}</summary>
        <p>{collectionRange(job.definition)}</p>
        <ImagePolicySummary policy={job.definition.media.image_policy} />
        <div className="lake-actions">
          {availableCollectionActions(job).map((action) => (
            <Button
              key={action}
              disabled={pending}
              onClick={() => void act(action)}
            >
              {collectionActions[action]}
            </Button>
          ))}
          <Button disabled={pending} onClick={() => onRecheck(job.definition)}>
            新建复查
          </Button>
        </div>
        {job.wait_reason && (
          <p role="status">{collectionReason(job.wait_reason)}</p>
        )}
        {job.state === "waiting_credentials" && (
          <>
            <Button onClick={onSettings}>打开 API 设置</Button>
            {p.access_mode === "anonymous" && (
              <p className="lake-hint">
                公开访问遇到登录或交互验证要求时，可以新建复查任务并选择已验证的登录会话。
              </p>
            )}
          </>
        )}
        {error != null && <ErrorDetails error={error} />}
      </details>
      <details open>
        <summary>采集进度</summary>
        <dl className="wb-property-list">
          <dt>作者</dt>
          <dd>
            {p.authors.scanned} 已扫描 / {p.authors.admitted} 已接纳 /{" "}
            {p.authors.discovered} 已发现
          </dd>
          <dt>作品详情</dt>
          <dd>
            {p.works.details} / {p.works.planned ?? "仍在发现"} · 其中{" "}
            {p.works.retained} 沿用近期快照
          </dd>
          <dt>范围排除</dt>
          <dd>{p.works.excluded ?? 0} 个作品 · 可在下方选择“不在范围内”查看</dd>
          <dt>媒体总量</dt>
          <dd>{p.media.planned ?? "仍在解析清单"}</dd>
          <dt>新下载 / 历史原件</dt>
          <dd>
            {p.media.downloaded} / {p.media.historical_reused}
          </dd>
          <dt>沿用已完整媒体</dt>
          <dd>{p.media.retained}</dd>
          <dt>本轮归档 / 发布</dt>
          <dd>
            {p.media.archived} / {p.media.published}
          </dd>
          <dt>本轮下载量</dt>
          <dd>{bytesLabel(p.download_bytes)}</dd>
          <dt>未获取或待检查</dt>
          <dd>
            {p.task_gaps} 项 · 作品 {p.works.gaps} / 媒体 {p.media.gaps}
          </dd>
          <dt>目录变化</dt>
          <dd>
            新增 {p.directory_delta.added} · 保留 {p.directory_delta.unchanged}{" "}
            · 本次未列出 {p.directory_delta.no_longer_listed}
          </dd>
        </dl>
        <p className="lake-hint">
          本次未列出的作品保留既有归档；目录变化不直接判定删除。
        </p>
      </details>
      <details>
        <summary>范围完成情况</summary>
        <dl className="wb-property-list">
          <dt>访问方式</dt>
          <dd>{p.access_mode === "anonymous" ? "公开访问" : "登录会话"}</dd>
          <dt>关系遍历</dt>
          <dd>
            {p.closure.discovery_exhausted ? "本次遍历结束" : "还有待发现项"}
          </dd>
          <dt>作者目录</dt>
          <dd>{p.closure.directories_complete ? "已处理" : "未完整处理"}</dd>
          <dt>作品与媒体清单</dt>
          <dd>{p.closure.manifests_complete ? "已处理" : "未完整处理"}</dd>
          <dt>登录显示条件</dt>
          <dd>
            {p.closure.visibility_verified
              ? "已验证"
              : "未知；独立于本次范围完成情况"}
          </dd>
          <dt>发布积压</dt>
          <dd>{p.publication.pending_batches} 批</dd>
          <dt>创建时间</dt>
          <dd>{dateLabel(job.created_at)}</dd>
        </dl>
      </details>
      <details>
        <summary>本轮预算</summary>
        <dl className="wb-property-list">
          <dt>API 请求</dt>
          <dd>
            {Math.floor(p.budget.used.api_requests ?? 0)} /{" "}
            {p.budget.limits.api_requests}
          </dd>
          <dt>接纳作者</dt>
          <dd>
            {Math.floor(p.budget.used.admitted_authors ?? 0)} /{" "}
            {p.budget.limits.admitted_authors}
          </dd>
          <dt>下载量</dt>
          <dd>
            {bytesLabel(p.budget.used.download_bytes)} /{" "}
            {bytesLabel(p.budget.limits.download_bytes)}
          </dd>
          <dt>运行时间</dt>
          <dd>
            {Math.floor((p.budget.used.wall_seconds ?? 0) / 60)} /{" "}
            {Math.floor(p.budget.limits.wall_seconds / 60)} 分钟
          </dd>
        </dl>
      </details>
      <details open>
        <summary>任务项与缺口</summary>
        <div className="lake-fields">
          <label>
            任务类型
            <select
              aria-label="Pixiv 任务类型"
              value={kind}
              onChange={(e) => {
                setKind(e.target.value);
                setCursors([""]);
                setSelected([]);
              }}
            >
              <option value="">全部类型</option>
              {Object.entries(taskKinds).map(([id, label]) => (
                <option key={id} value={id}>
                  {label}
                </option>
              ))}
            </select>
          </label>
          <label>
            处理状态
            <select
              aria-label="Pixiv 处理状态"
              value={state}
              onChange={(e) => {
                setState(e.target.value);
                setCursors([""]);
                setSelected([]);
              }}
            >
              <option value="">全部状态</option>
              <option value="gaps">未获取 / 需要检查</option>
              {Object.entries(taskStates).map(([id, label]) => (
                <option key={id} value={id}>
                  {label}
                </option>
              ))}
            </select>
          </label>
        </div>
        {tasks.error && <ErrorDetails error={tasks.error} />}
        {tasks.data?.items.map((t) => (
          <div className="collection-task" key={t.id}>
            <label className="lake-check">
              <input
                type="checkbox"
                aria-label={`选择任务 ${t.subject_key}`}
                disabled={
                  ![
                    "unavailable",
                    "needs_review",
                    "retry_wait",
                    "waiting_credentials",
                    "waiting_resources",
                  ].includes(t.state)
                }
                checked={selected.includes(t.id)}
                onChange={(e) =>
                  setSelected((v) =>
                    e.target.checked
                      ? [...v, t.id]
                      : v.filter((id) => id !== t.id),
                  )
                }
              />
              <strong>{taskKinds[t.kind] ?? t.kind}</strong> ·{" "}
              {taskStates[t.state] ?? t.state}
            </label>
            <small>
              {t.subject_key} · 尝试 {t.attempts} 次
            </small>
            {t.reason && <p>{collectionReason(t.reason)}</p>}
          </div>
        ))}
        {!tasks.isPending && !tasks.data?.items.length && (
          <p className="lake-hint">没有符合筛选的任务项。</p>
        )}
        <div className="lake-actions">
          <Button
            disabled={!selected.length || pending || job.execution_active}
            onClick={() => void act("retry_failed", selected)}
          >
            重试所选 {selected.length} 项
          </Button>
        </div>
        <footer className="lake-pagination">
          <Button
            disabled={cursors.length === 1 || tasks.isFetching}
            onClick={() => {
              setCursors((v) => v.slice(0, -1));
              setSelected([]);
            }}
          >
            上一页
          </Button>
          <span>第 {cursors.length} 页</span>
          <Button
            disabled={!tasks.data?.next_cursor || tasks.isFetching}
            onClick={() => {
              setCursors((v) => [...v, tasks.data!.next_cursor!]);
              setSelected([]);
            }}
          >
            下一页
          </Button>
        </footer>
      </details>
    </div>
  );
}
