import { useEffect, useRef, useState } from "react";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { Button, ErrorDetails } from "@studio/ui";
import type { Schema } from "@studio/contracts";
import type { SettingsPageProps } from "./types.js";
import { sizeLabel } from "./types.js";
import { describe, usedAt } from "./cacheLabels.js";
import {
  CacheCleanupProgress,
  cleanupActive,
  cleanupPhase,
} from "./CacheCleanupProgress.js";

const states: Record<string, string> = {
  open: "已打开",
  closed: "已关闭",
  background: "后台运行",
  draining: "正在关闭",
  unavailable: "不可用",
};
export function CacheProjectManager({
  client,
  project,
  activeResultId,
  data,
  busy,
  action,
}: SettingsPageProps) {
  const [chosen, setChosen] = useState(project?.id ?? "");
  const [focusCleanup, setFocusCleanup] = useState("");
  const cleanupPanel = useRef<HTMLElement>(null);
  const projects = useQuery({
    queryKey: ["settings", "cache-projects", client.connection.instance_id],
    queryFn: ({ signal }) => client.settings.projects(signal),
    refetchInterval: 3000,
  });
  const items = projects.data?.items ?? [];
  const selected =
    items.find((p) => p.project_id === chosen) ??
    items.find((p) => p.project_id === project?.id) ??
    items[0];
  const pid = selected?.project_id ?? "";
  const inventory = useInfiniteQuery({
    queryKey: [
      "settings",
      "cache-inventory",
      client.connection.instance_id,
      pid,
    ],
    queryFn: ({ pageParam, signal }) =>
      client.settings.inventory(pid, pageParam ?? undefined, signal),
    initialPageParam: null as string | null,
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    enabled: !!pid,
    refetchInterval: data.storage.cleanup_pending ? 1000 : 3000,
  });
  const first = inventory.data?.pages[0];
  const members = inventory.data?.pages.flatMap((p) => p.members) ?? [];
  const cleanups = [...(first?.cleanups ?? [])].sort(
    (a, b) => Number(b.started_millis) - Number(a.started_millis),
  );
  const visibleCleanups = [
    ...cleanups.filter((t) => cleanupActive(t.state)),
    ...cleanups.filter((t) => !cleanupActive(t.state)).slice(0, 3),
  ];
  useEffect(() => {
    if (
      focusCleanup &&
      first?.cleanups.some((t) => t.result_id === focusCleanup)
    ) {
      cleanupPanel.current?.scrollIntoView({ block: "nearest" });
      setFocusCleanup("");
    }
  }, [focusCleanup, first]);
  async function change(run: () => Promise<unknown>, notice: string) {
    await action(async () => {
      await run();
      await Promise.all([inventory.refetch(), projects.refetch()]);
    }, notice);
  }
  async function release(resultId: string) {
    setFocusCleanup(resultId);
    await change(
      () => client.settings.releaseMember(pid, resultId),
      "已提交后台清理，项目可以继续使用。",
    );
  }
  const retain = (
    entry: Schema["CacheMemberItem"],
    tier: "long_term" | "temporary",
    fixed: boolean,
  ) =>
    change(
      () => client.settings.retainMember(pid, entry.result_id, tier, fixed),
      "保留方式已更新，成员无需复制。",
    );
  return (
    <>
      <section className="settings-section">
        <h4>各项目占用</h4>
        <p className="settings-note">
          包含已关闭的项目。成员存储包含筛选结果和项目固定输入；共享的分级基础、来源索引与缩略图另计。
        </p>
        {projects.error && <ErrorDetails error={projects.error} />}
        <div className="settings-table-wrap">
          <table className="settings-table settings-project-caches">
            <thead>
              <tr>
                <th>项目</th>
                <th>成员存储</th>
                <th>排名索引</th>
                <th>合计</th>
              </tr>
            </thead>
            <tbody>
              {items.map((p) => (
                <tr key={p.project_id} aria-selected={p.project_id === pid}>
                  <td>
                    <button
                      className="settings-cache-project"
                      disabled={busy}
                      onClick={() => setChosen(p.project_id)}
                    >
                      {p.name}
                    </button>
                    <small>
                      {states[p.state] ?? p.state}
                      {p.project_id === project?.id ? " · 当前项目" : ""}
                    </small>
                  </td>
                  <td>{sizeLabel(p.member_bytes)}</td>
                  <td>{sizeLabel(p.ranked_index_bytes)}</td>
                  <td>{sizeLabel(p.total_bytes)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {projects.isPending && <p role="status">正在读取项目占用…</p>}
        {!projects.isPending && !items.length && <p>尚无项目缓存。</p>}
        <p className="settings-note">
          共享占用：来源索引 {sizeLabel(data.storage.source_index_bytes)} ·
          分级基础 {sizeLabel(data.storage.rating_basis_bytes)} · 缩略图{" "}
          {sizeLabel(data.storage.preview_bytes)}。构建中工作空间另计{" "}
          {sizeLabel(data.storage.working_temporary_bytes)}。
        </p>
      </section>
      {!!visibleCleanups.length && (
        <section ref={cleanupPanel} className="settings-cleanup-status">
          <h4>{selected?.name} · 清理进度</h4>
          {data.storage.cleanup_pending && (
            <p role="status">
              {cleanupPhase(data.maintenance.phase)} · 可复用空间{" "}
              {sizeLabel(data.storage.reusable_bytes)}
            </p>
          )}
          {visibleCleanups.map((task) => (
            <CacheCleanupProgress
              key={task.family_id}
              task={task}
              label={task.spec ? describe(task.spec) : "项目输入"}
              retry={() => void release(task.result_id)}
            />
          ))}
        </section>
      )}
      {selected && (
        <section className="settings-section">
          <h4>{selected.name} · 缓存明细</h4>
          <p className="settings-note">
            临时类 {sizeLabel(selected.temporary_bytes)} · 长期类{" "}
            {sizeLabel(selected.long_term_bytes)}
            。临时类中也包含被项目引用的输入，清理时会保留。
          </p>
          <details className="settings-cache-path">
            <summary>项目与成员存储位置</summary>
            <code>{first?.member_path ?? selected.directory}</code>
          </details>
          {selected.issue && <p role="alert">{selected.issue}</p>}
          {inventory.error && <ErrorDetails error={inventory.error} />}
          {inventory.isPending && <p role="status">正在读取缓存明细…</p>}
          <div className="settings-table-wrap">
            <table className="settings-table settings-cache-members">
              <thead>
                <tr>
                  <th>内容与用途</th>
                  <th>成员 / 占用</th>
                  <th>保留方式</th>
                  <th>操作</th>
                </tr>
              </thead>
              <tbody>
                {members.map((entry) => (
                  <tr key={entry.family_id}>
                    <td>
                      <strong>
                        {entry.cached
                          ? describe(entry.spec)
                          : entry.reference_count
                            ? "项目固定输入"
                            : "待回收成员"}
                      </strong>
                      <small>最近使用 {usedAt(entry.last_used_millis)}</small>
                      {entry.references.map((r, i) => (
                        <small key={i}>{r}</small>
                      ))}
                      {entry.reference_count > entry.references.length && (
                        <small>共 {entry.reference_count} 个引用</small>
                      )}
                      {entry.reason && <small>{entry.reason}</small>}
                    </td>
                    <td>
                      {entry.members.toLocaleString()}
                      <small>
                        {entry.estimated_bytes == null
                          ? "空间统计中…"
                          : sizeLabel(entry.estimated_bytes)}
                      </small>
                    </td>
                    <td>
                      {entry.cached ? (
                        <>
                          <select
                            aria-label={"缓存类别 " + entry.result_id}
                            disabled={busy}
                            value={entry.tier}
                            onChange={(e) =>
                              void retain(
                                entry,
                                e.target.value === "long_term"
                                  ? "long_term"
                                  : "temporary",
                                entry.fixed,
                              )
                            }
                          >
                            <option value="long_term">长期</option>
                            <option value="temporary">临时</option>
                          </select>
                          <label className="settings-cache-pin">
                            <input
                              type="checkbox"
                              aria-label={"固定缓存 " + entry.result_id}
                              checked={entry.fixed}
                              disabled={busy}
                              onChange={(e) =>
                                void retain(
                                  entry,
                                  entry.tier === "long_term"
                                    ? "long_term"
                                    : "temporary",
                                  e.target.checked,
                                )
                              }
                            />
                            固定保留
                          </label>
                          {entry.session_only && entry.tier === "temporary" && (
                            <small>仅本次会话</small>
                          )}
                        </>
                      ) : entry.reference_count ? (
                        "随项目引用保留"
                      ) : (
                        "等待回收"
                      )}
                    </td>
                    <td>
                      <Button
                        disabled={
                          busy ||
                          !entry.can_release ||
                          (pid === project?.id &&
                            entry.result_id === activeResultId)
                        }
                        onClick={() => void release(entry.result_id)}
                      >
                        {entry.in_use ? "正在使用" : "清理"}
                      </Button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {inventory.hasNextPage && (
            <Button
              disabled={inventory.isFetchingNextPage}
              onClick={() => void inventory.fetchNextPage()}
            >
              加载更多成员缓存
            </Button>
          )}
          {!inventory.isPending && !members.length && (
            <p>没有保留的成员缓存。</p>
          )}
          <p className="settings-note">
            成员占用按共享数据库中的记录数分摊估算。固定保留与项目引用都会阻止成员清理；排名索引单独遵循临时保留规则。
          </p>
          <h4>排名浏览索引</h4>
          <div className="settings-table-wrap">
            <table className="settings-table">
              <thead>
                <tr>
                  <th>对应范围</th>
                  <th>成员</th>
                  <th>占用</th>
                  <th>操作</th>
                </tr>
              </thead>
              <tbody>
                {first?.ranked_indexes.map((entry) => (
                  <tr key={entry.key}>
                    <td>
                      {entry.label}
                      <small>最近使用 {usedAt(entry.last_used_millis)}</small>
                      <details className="settings-cache-path">
                        <summary>文件位置</summary>
                        <code>{entry.path}</code>
                      </details>
                    </td>
                    <td>{entry.members.toLocaleString()}</td>
                    <td>{sizeLabel(entry.bytes)}</td>
                    <td>
                      <Button
                        disabled={busy || entry.in_use}
                        onClick={() =>
                          void change(
                            () => client.settings.releaseRanked(pid, entry.key),
                            "排名索引已清理，再次浏览此范围时会重新建立。",
                          )
                        }
                      >
                        {entry.in_use ? "正在使用" : "清理索引"}
                      </Button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {!first?.ranked_indexes.length && <p>没有已完成的排名索引。</p>}
        </section>
      )}
    </>
  );
}
