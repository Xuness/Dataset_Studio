import { useQuery } from "@tanstack/react-query";
import { Button, ErrorDetails } from "@studio/ui";
import type { SettingsPageProps } from "./types.js";
import { sizeLabel } from "./types.js";
const resourceNames: Record<string, string> = {
  index: "索引读取",
  media: "图片读取",
  decode: "预览解码",
  native_query: "元数据查询",
};

export function PerformanceSettingsPage({
  client,
  data,
  busy,
  action,
  memoryDraft,
  setMemoryDraft,
}: SettingsPageProps) {
  const resources = useQuery({
    queryKey: ["resources", client.connection.instance_id],
    queryFn: ({ signal }) => client.resources.status(signal),
    refetchInterval: 2000,
  });
  const value =
    memoryDraft ??
    String(Number(data.query_limits.query_memory_bytes) / 1073741824);
  const valid =
    /^\d+$/.test(value) && Number(value) >= 1 && Number(value) <= 64;
  return (
    <div className="settings-page">
      <div className="settings-page-heading">
        <h3>性能与任务</h3>
        <p>调整查询工作预算，并查看当前资源使用情况。</p>
      </div>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (memoryDraft && valid && !busy)
            void action(async () => {
              await client.resources.configureQuery(Number(value));
              setMemoryDraft(null);
            }, "查询预算已保存，将用于下一次查询。");
        }}
      >
        <section className="settings-section">
          <h4>查询工作内存</h4>
          <div className="settings-control-row">
            <div>
              <label htmlFor="settings-query-memory">单次查询预算</label>
              <p>按需使用，不会预先占满。已运行的查询继续使用原预算。</p>
            </div>
            <div className="settings-number">
              <input
                id="settings-query-memory"
                aria-label="查询工作内存 GiB"
                disabled={busy}
                type="number"
                min={1}
                max={64}
                value={value}
                onChange={(e) => setMemoryDraft(e.target.value)}
              />
              <span>GiB</span>
            </div>
          </div>
          <p className="settings-note">
            当前分配：原生查询{" "}
            {sizeLabel(data.query_limits.native_query_memory_bytes)}，结果整理{" "}
            {sizeLabel(data.query_limits.result_work_memory_bytes)}
            。这是工作预算，整个引擎还需要界面预览和元数据等内存。
          </p>
          {data.query_limits.active_query_memory_bytes && (
            <p className="settings-note">
              正在运行的查询使用{" "}
              {sizeLabel(data.query_limits.active_query_memory_bytes)} 预算。
            </p>
          )}
          <div className="settings-page-actions">
            <Button
              type="button"
              disabled={busy || memoryDraft === null}
              onClick={() => setMemoryDraft(null)}
            >
              撤销修改
            </Button>
            <Button
              type="submit"
              className="primary"
              disabled={busy || memoryDraft === null || !valid}
            >
              保存性能设置
            </Button>
          </div>
        </section>
      </form>
      <section className="settings-section">
        <h4>执行暂存</h4>
        <p className="settings-note">
          原生查询与结果暂存分别最多使用{" "}
          {sizeLabel(data.query_limits.temporary_disk_bytes)}、
          {sizeLabel(data.query_limits.result_staging_disk_bytes)}
          。任务结束后自动清理，当前占用{" "}
          {sizeLabel(data.storage.working_temporary_bytes)}。
        </p>
      </section>
      <section className="settings-section">
        <h4>当前运行状态</h4>
        {resources.error && <ErrorDetails error={resources.error} />}
        <div className="settings-table-wrap">
          <table className="settings-table">
            <thead>
              <tr>
                <th>资源</th>
                <th>运行 / 并发</th>
                <th>排队</th>
                <th>已分配内存</th>
              </tr>
            </thead>
            <tbody>
              {resources.data?.resources.map((r) => (
                <tr key={r.class}>
                  <td>{resourceNames[r.class] ?? r.class}</td>
                  <td>
                    {r.active} / {r.concurrency}
                  </td>
                  <td>{r.queued}</td>
                  <td>{sizeLabel(r.reserved_bytes)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {resources.data?.process_memory && (
          <p className="settings-note">
            进程当前驻留{" "}
            {sizeLabel(resources.data.process_memory.resident_bytes)}，峰值{" "}
            {sizeLabel(resources.data.process_memory.peak_resident_bytes)}。
          </p>
        )}
        {resources.data && (
          <p className="settings-note">
            预览缓存命中 {resources.data.cache.hits.toLocaleString()} 次 ·
            正在等待 {resources.data.previews.queued} 项 · 已生成{" "}
            {resources.data.previews.generated.toLocaleString()} 张预览。
          </p>
        )}
      </section>
    </div>
  );
}
