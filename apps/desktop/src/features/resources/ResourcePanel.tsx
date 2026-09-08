import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Database, RotateCw } from "lucide-react";
import { Button, ErrorDetails } from "@studio/ui";
import type { ModuleContext } from "@studio/ui";
import "./resources.css";
const mib = (value: string) => (Number(value) / 1048576).toFixed(2) + " MiB";
const gib = (value: string) => (Number(value) / 1073741824).toFixed(0) + " GiB";
const labels: Record<string, string> = {
  index: "索引与项目数据",
  media: "源图片读取",
  decode: "缩略图解码",
  native_query: "元数据与原生查询",
};
export default function ResourcePanel({ client }: ModuleContext) {
  const status = useQuery({
    queryKey: ["resources", client.connection.instance_id],
    queryFn: ({ signal }) => client.resources.status(signal),
    refetchInterval: 2000,
  });
  const [quota, setQuota] = useState<string | null>(null);
  const [queryMemory, setQueryMemory] = useState<string | null>(null);
  const [queryQuota, setQueryQuota] = useState<string | null>(null);
  const [queryAge, setQueryAge] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const data = status.data;
  async function action(run: () => Promise<void>) {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await run();
      await status.refetch();
    } catch (e) {
      setError(e instanceof Error ? e.message : "资源设置操作失败");
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="resource-view">
      <div className="content-bar">
        <Database size={16} />
        <strong>读取与缓存</strong>
        <span className="grow" />
        <Button onClick={() => void status.refetch()}>
          <RotateCw size={14} />
          刷新
        </Button>
      </div>
      <main>
        {(error || status.error) && (
          <ErrorDetails error={error || status.error} />
        )}
        {notice && <p role="status">{notice}</p>}
        {data && (
          <>
            <section className="query-resource-summary">
              <h2>查询资源</h2>
              <p>
                单图元数据：{mib(data.query_limits.metadata_memory_bytes)}
                ；范围查询：{gib(data.query_limits.query_memory_bytes)}。
                其中原生查询 {mib(data.query_limits.native_query_memory_bytes)}
                ，结果暂存 {mib(data.query_limits.result_work_memory_bytes)}。
              </p>
              <p>
                原生查询临时空间上限为{" "}
                {gib(data.query_limits.temporary_disk_bytes)} ，成员暂存另有{" "}
                {gib(data.query_limits.result_staging_disk_bytes)}{" "}
                上限，结束后自动清理。共享索引与保留结果单独计量。
              </p>
              <div className="resource-controls">
                <label>
                  查询内存上限（GiB）
                  <input
                    aria-label="查询内存上限 GiB"
                    type="number"
                    min={1}
                    max={64}
                    step={1}
                    value={
                      queryMemory ??
                      String(
                        Number(data.query_limits.query_memory_bytes) /
                          1073741824,
                      )
                    }
                    onChange={(e) => setQueryMemory(e.target.value)}
                  />
                </label>
                <Button
                  disabled={
                    busy ||
                    queryMemory === null ||
                    !/^\d+$/.test(queryMemory) ||
                    Number(queryMemory) < 1 ||
                    Number(queryMemory) > 64
                  }
                  onClick={() =>
                    void action(async () => {
                      await client.resources.configureQuery(
                        Number(queryMemory),
                      );
                      setQueryMemory(null);
                      setNotice(
                        "查询预算已保存，将用于下一次查询；正在运行的查询保持原预算。",
                      );
                    })
                  }
                >
                  保存查询预算
                </Button>
              </div>
              <p className="subtle">
                上限按需使用，不会预先占满内存。它约束查询工作内存，整个引擎还需要元数据、图像解码和结果存储等内存。默认
                12 GiB，可设为 1 至 64 GiB。
              </p>
              {data.query_limits.active_query_memory_bytes && (
                <p>
                  当前查询使用{" "}
                  {gib(data.query_limits.active_query_memory_bytes)} 上限。
                </p>
              )}
            </section>
            <h2>查询结果复用</h2>
            <p>
              保留 {data.query_cache.retained_queries} 组结果 · 成员存储{" "}
              {mib(data.query_cache.result_storage_bytes)} ·{" "}
              {data.query_cache.active_views} 个浏览占用
            </p>
            <div className="resource-controls">
              <label>
                保留容量（MiB）
                <input
                  aria-label="查询缓存容量 MiB"
                  type="number"
                  min={0}
                  max={65536}
                  value={
                    queryQuota ??
                    String(Number(data.query_cache.quota_bytes) / 1048576)
                  }
                  onChange={(e) => setQueryQuota(e.target.value)}
                />
              </label>
              <label>
                最长未使用时间（天）
                <input
                  aria-label="查询缓存保留天数"
                  type="number"
                  min={1}
                  max={90}
                  value={queryAge ?? String(data.query_cache.max_age_days)}
                  onChange={(e) => setQueryAge(e.target.value)}
                />
              </label>
              <Button
                disabled={
                  busy ||
                  (queryQuota === null && queryAge === null) ||
                  (queryQuota !== null &&
                    (!/^\d+$/.test(queryQuota) ||
                      Number(queryQuota) > 65536)) ||
                  (queryAge !== null &&
                    (!/^\d+$/.test(queryAge) ||
                      Number(queryAge) < 1 ||
                      Number(queryAge) > 90))
                }
                onClick={() =>
                  void action(async () => {
                    await client.resources.configureQueryCache(
                      queryQuota === null
                        ? Number(data.query_cache.quota_bytes) / 1048576
                        : Number(queryQuota),
                      queryAge === null
                        ? data.query_cache.max_age_days
                        : Number(queryAge),
                    );
                    setQueryQuota(null);
                    setQueryAge(null);
                    setNotice("查询缓存设置已保存，后台分批回收未使用的结果。");
                  })
                }
              >
                保存复用设置
              </Button>
              <Button
                disabled={busy || data.query_cache.cleanup_pending}
                onClick={() =>
                  void action(async () => {
                    await client.resources.clearQueryCache();
                    setNotice(
                      "已开始清理未使用的查询结果，正在浏览和已被引用的成员会保留。",
                    );
                  })
                }
              >
                清理未使用结果
              </Button>
            </div>
            <p className="subtle">
              相同条件复用成员；来源更新后按需检查变化。保留容量设为 0
              可关闭后续复用。选择、工作集和任务引用的{" "}
              {data.query_cache.protected_results}{" "}
              个结果受保护，连同浏览占用可使成员存储暂时超过保留容量。
            </p>
            <p className="subtle">
              共享排序索引 {data.query_cache.source_indexes} 个 ·{" "}
              {mib(data.query_cache.source_index_bytes)}
              ，每个来源维护一份并独立计量。项目数据库有{" "}
              {mib(data.query_cache.database_free_bytes)}{" "}
              空闲页可供下次写入，并会分批归还磁盘。
            </p>
            <p className="subtle">
              已复用 {data.query_cache.reused_results} 次 · 增量刷新{" "}
              {data.query_cache.incremental_results} 次 · 本次运行回收{" "}
              {data.query_cache.reclaimed_queries} 组{" "}
              {data.query_cache.cleanup_pending ? "· 正在清理…" : ""}
            </p>
            <h2>缩略图磁盘缓存</h2>
            {data.cache.index_rebuilt && (
              <p role="status">
                本次启动已重建损坏的缓存索引，缓存设置已保留。
              </p>
            )}
            {data.cache.clear_pending && (
              <p role="status">
                正在后台清理缓存。读取占用会在结束后释放，完成前的新预览不写入磁盘缓存。
              </p>
            )}
            <p>
              已保存 {data.cache.entries} 张 · {mib(data.cache.bytes)} /{" "}
              {mib(data.cache.quota_bytes)} · {data.cache.pinned} 个读取占用
            </p>
            <div className="resource-controls">
              <label>
                配额（MiB）
                <input
                  aria-label="缓存配额 MiB"
                  type="number"
                  min={0}
                  max={1048576}
                  value={
                    quota ?? String(Number(data.cache.quota_bytes) / 1048576)
                  }
                  onChange={(e) => setQuota(e.target.value)}
                />
              </label>
              <Button
                disabled={busy || quota === null || !/^\d+$/.test(quota)}
                onClick={() =>
                  void action(async () => {
                    await client.resources.configure(Number(quota));
                    setQuota(null);
                    setNotice("缓存配额已保存，后台分批回收超过配额的材料。");
                  })
                }
              >
                保存配额
              </Button>
              <Button
                disabled={busy || data.cache.clear_pending}
                onClick={() =>
                  void action(async () => {
                    const result = await client.resources.clear();
                    setNotice(
                      result.clear_pending
                        ? `已开始后台清理，剩余 ${result.entries} 张。`
                        : "缩略图缓存已清空，浏览时会重新生成。",
                    );
                  })
                }
              >
                清理缓存
              </Button>
            </div>
            <p className="subtle">
              配额设为 0
              可停止保留新缩略图。缓存清理在后台分批完成；项目选择、任务和成果独立保存。
            </p>
            <dl>
              <dt>缓存目录</dt>
              <dd>{data.cache.directory}</dd>
              <dt>本次引擎运行</dt>
              <dd>
                命中 {data.cache.hits} · 未命中 {data.cache.misses} · 修复损坏{" "}
                {data.cache.corrupt} · 写入 {data.cache.writes} · 淘汰{" "}
                {data.cache.evicted}
              </dd>
              <dt>磁盘读取</dt>
              <dd>
                缓存 {mib(data.cache.read_bytes)} · 原图{" "}
                {mib(data.previews.source_bytes)} · 打开数据包{" "}
                {data.previews.pack_opens} 次
              </dd>
              <dt>预览工作</dt>
              <dd>
                合并 {data.previews.shared} 次 · 生成 {data.previews.generated}{" "}
                张 · 等待 {data.previews.queued} 项 · 读盘前取消{" "}
                {data.previews.cancelled_before_read} 项
              </dd>
              <dt>累计开销</dt>
              <dd>
                源读取 {data.previews.read_ms} ms · 解码{" "}
                {data.previews.decode_ms} ms · {data.previews.batches}{" "}
                个批次，最大 {data.previews.max_batch} 张
              </dd>
            </dl>
            <h2>读取资源</h2>
            <table>
              <thead>
                <tr>
                  <th>资源</th>
                  <th>使用 / 并发</th>
                  <th>排队 / 上限</th>
                  <th>预算</th>
                  <th>累计等待</th>
                  <th>排队取消</th>
                </tr>
              </thead>
              <tbody>
                {data.resources.map((r) => (
                  <tr key={r.class}>
                    <td>{labels[r.class]}</td>
                    <td>
                      {r.active} / {r.concurrency}
                    </td>
                    <td>
                      {r.queued} / {r.queue_limit}
                    </td>
                    <td>
                      {mib(r.reserved_bytes)} / {mib(r.byte_budget)}
                    </td>
                    <td>{r.wait_ms} ms</td>
                    <td>{r.cancelled_waiting}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            <p className="subtle">
              交互请求优先，后台工作定期获得资源。取消会移除等待项；已经进入系统读取或解码库的调用会在可检查的位置停止。
            </p>
            <p className="subtle">
              离线时仅使用此前校验过的缓存，并显示“离线缓存”。缓存计数与耗时是本次引擎运行的累计值。
            </p>
          </>
        )}
      </main>
    </section>
  );
}
