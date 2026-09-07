import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Database, RotateCw } from "lucide-react";
import { Button } from "@studio/ui";
import type { ModuleContext } from "@studio/ui";
import "./resources.css";
const mib = (value: string) => (Number(value) / 1048576).toFixed(2) + " MiB";
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
      setError(e instanceof Error ? e.message : "缓存操作失败");
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
          <p role="alert">{error || status.error?.message}</p>
        )}
        {notice && <p role="status">{notice}</p>}
        {data && (
          <>
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
