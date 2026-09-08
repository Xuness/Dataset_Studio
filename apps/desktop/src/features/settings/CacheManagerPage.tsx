import { useState } from "react";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { Button, ErrorDetails, ratingLabel } from "@studio/ui";
import type { QuerySpec } from "@studio/contracts";
import type { SettingsPageProps } from "./types.js";
import { sizeLabel } from "./types.js";

function describe(spec: QuerySpec) {
  const names: Record<string, string> = {
    rating: "分级",
    tags: "标签",
    "stored.bytes": "文件大小",
    "stored.extension": "文件格式",
    "post.id": "帖子 ID",
    score: "评分",
  };
  const operators: Record<string, string> = {
    eq: "",
    in: "",
    has_tag: "包含",
    has_all_tags: "包含全部",
    has_any_tags: "包含任一",
    has_no_tags: "排除",
    is_missing: "未记录",
    is_present: "已记录",
    gte: "≥",
    lte: "≤",
    ne: "不等于",
  };
  if (!spec.conditions.length) return "浏览排序 · 全部成员";
  return spec.conditions
    .map((c) => {
      const raw =
        c.value?.type === "text_list"
          ? c.value.value.join(" / ")
          : c.value
            ? String(c.value.value)
            : "";
      return [
        names[c.field] ?? c.field,
        operators[c.operator] ?? c.operator,
        c.field === "rating" ? raw.toUpperCase() : raw,
      ]
        .filter(Boolean)
        .join(" ");
    })
    .join(" · ");
}
const usedAt = (value: string) =>
  Number(value)
    ? new Date(Number(value)).toLocaleString("zh-CN", {
        month: "2-digit",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
      })
    : "尚未使用";

export function CacheManagerPage({
  client,
  project,
  sources,
  data,
  activeResultId,
  busy,
  action,
}: SettingsPageProps) {
  const [chosenSource, setChosenSource] = useState("");
  const lakes = sources.filter((s) => s.kind === "danbooru" && s.available);
  const sourceId =
    lakes.find((s) => s.id === chosenSource)?.id ?? lakes[0]?.id ?? "";
  const bases = useQuery({
    queryKey: ["settings", "rating-bases", client.connection.instance_id],
    queryFn: ({ signal }) => client.settings.ratingBases(signal),
    refetchInterval: 1500,
  });
  const entries = useInfiniteQuery({
    staleTime: 0,
    refetchInterval: 3000,
    queryKey: [
      "settings",
      "cache-entries",
      client.connection.instance_id,
      project?.id,
    ],
    queryFn: ({ pageParam, signal }) =>
      client.settings.entries(project!.id, pageParam ?? undefined, signal),
    initialPageParam: null as string | null,
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    enabled: !!project,
  });
  const rows = entries.data?.pages.flatMap((p) => p.items) ?? [];
  const builds = bases.data?.builds ?? [];
  const building = builds.find(
    (b) => b.source_id === sourceId && ["queued", "running"].includes(b.state),
  );
  const sourceName = (id: string) =>
    sources.find((s) => s.id === id)?.name ?? "数据湖 " + id.slice(0, 8);
  return (
    <div className="settings-page">
      <div className="settings-page-heading">
        <h3>缓存管理</h3>
        <p>查看基础分级和查询结果，调整保留类别或清理不再需要的内容。</p>
      </div>
      <section className="settings-section">
        <h4>分级基础缓存</h4>
        <p className="settings-note">
          G、S、Q、E 按需建立，多个项目共用。Tag
          组合查询利用相应基础集合缩小候选范围；来源更新后按需刷新。
        </p>
        <div className="settings-toolbar">
          <select
            aria-label="预建分级缓存的数据湖"
            disabled={busy || !lakes.length}
            value={sourceId}
            onChange={(e) => setChosenSource(e.target.value)}
          >
            {!lakes.length && (
              <option value="">打开项目并挂载数据湖后可预建</option>
            )}
            {lakes.map((s) => (
              <option key={s.id} value={s.id}>
                {s.name}
              </option>
            ))}
          </select>
          <Button
            disabled={
              busy ||
              !project ||
              !sourceId ||
              !!building ||
              data.cache.long_term_mib === 0
            }
            onClick={() => {
              if (project)
                void action(
                  () => client.settings.prebuild(project.id, sourceId),
                  "已安排 G、S、Q、E 基础缓存构建。",
                );
            }}
          >
            建立或更新全部分级
          </Button>
        </div>
        {builds
          .filter((b) =>
            ["queued", "running", "failed", "cancelled"].includes(b.state),
          )
          .map((b) => (
            <div className="settings-build" key={b.source_id}>
              <span>
                {sourceName(b.source_id)} ·{" "}
                {b.state === "failed"
                  ? "构建失败"
                  : b.state === "cancelled"
                    ? "已取消"
                    : b.current_rating
                      ? "正在处理 " + b.current_rating.toUpperCase()
                      : "等待构建"}{" "}
                · 已完成 {b.completed.length} / 4
                {b.error ? " · " + b.error : ""}
              </span>
              {["queued", "running"].includes(b.state) && (
                <Button
                  disabled={busy}
                  onClick={() =>
                    void action(
                      () => client.settings.cancelBuild(b.source_id),
                      "已请求取消基础缓存构建。",
                    )
                  }
                >
                  取消
                </Button>
              )}
            </div>
          ))}
        {bases.error && <ErrorDetails error={bases.error} />}
        {bases.data?.items.length ? (
          <div className="settings-table-wrap">
            <table className="settings-table">
              <thead>
                <tr>
                  <th>数据湖 / 分级</th>
                  <th>占用</th>
                  <th>最后使用</th>
                  <th>保留</th>
                  <th>操作</th>
                </tr>
              </thead>
              <tbody>
                {bases.data.items.map((item) => (
                  <tr key={item.source_id + item.rating}>
                    <td>
                      <strong>{ratingLabel(item.rating)}</strong>
                      <small>
                        {sourceName(item.source_id)} ·{" "}
                        {item.records.toLocaleString()} 条候选记录
                      </small>
                    </td>
                    <td>{sizeLabel(item.bytes)}</td>
                    <td>{usedAt(item.last_used_millis)}</td>
                    <td>
                      <label className="settings-check">
                        <input
                          type="checkbox"
                          checked={item.fixed}
                          disabled={busy || item.active}
                          onChange={(e) =>
                            void action(
                              () =>
                                client.settings.fixBasis(
                                  item.source_id,
                                  item.rating,
                                  e.target.checked,
                                ),
                              "基础缓存保留方式已更新。",
                            )
                          }
                        />
                        固定
                      </label>
                      {item.active && <small>正在使用</small>}
                    </td>
                    <td>
                      <Button
                        disabled={busy || item.fixed || item.active}
                        onClick={() =>
                          void action(
                            () =>
                              client.settings.releaseBasis(
                                item.source_id,
                                item.rating,
                              ),
                            "基础缓存已清理，下次需要时可重新建立。",
                          )
                        }
                      >
                        清理
                      </Button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <p className="settings-empty">
            尚未建立分级基础缓存。首次相关查询会按需生成，也可以在这里预先建立。
          </p>
        )}
      </section>
      <section className="settings-section">
        <h4>{project ? project.name + " · 查询结果" : "项目查询结果"}</h4>
        <p className="settings-note">
          调整类别共用原有成员；固定保留会覆盖闲置清理期限。表中空间按共享成员存储分摊估算，合计占用以“缓存与存储”页面为准。
        </p>
        {entries.error && <ErrorDetails error={entries.error} />}
        {!project ? (
          <p className="settings-empty">
            打开项目后，可以管理它的查询结果。全局容量与分级基础缓存仍可在这里查看。
          </p>
        ) : rows.length ? (
          <>
            <div className="settings-table-wrap">
              <table className="settings-table">
                <thead>
                  <tr>
                    <th>筛选条件</th>
                    <th>估算占用</th>
                    <th>保留类别</th>
                    <th>固定</th>
                    <th>操作</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((entry) => (
                    <tr key={entry.family_id}>
                      <td className="settings-query-name">
                        <strong title={describe(entry.spec)}>
                          {describe(entry.spec)}
                        </strong>
                        <small>
                          {entry.members.toLocaleString()} 条成员 ·{" "}
                          {usedAt(entry.last_used_millis)}
                          {entry.protected_results
                            ? " · 项目引用 " + entry.protected_results + " 项"
                            : ""}
                        </small>
                      </td>
                      <td>{sizeLabel(entry.estimated_bytes)}</td>
                      <td>
                        <select
                          aria-label={"缓存类别 " + entry.result_id}
                          value={entry.tier}
                          disabled={busy}
                          onChange={(e) =>
                            void action(
                              () =>
                                client.settings.retention(
                                  project.id,
                                  entry.result_id,
                                  e.target.value === "long_term"
                                    ? "long_term"
                                    : "temporary",
                                  entry.fixed,
                                ),
                              "缓存类别已更新，成员无需复制。",
                            )
                          }
                        >
                          <option value="long_term">长期</option>
                          <option value="temporary">临时</option>
                        </select>
                        {entry.session_only && entry.tier === "temporary" && (
                          <small>仅本次会话</small>
                        )}
                      </td>
                      <td>
                        <input
                          type="checkbox"
                          aria-label={"固定缓存 " + entry.result_id}
                          checked={entry.fixed}
                          disabled={busy}
                          onChange={(e) =>
                            void action(
                              () =>
                                client.settings.retention(
                                  project.id,
                                  entry.result_id,
                                  entry.tier === "long_term"
                                    ? "long_term"
                                    : "temporary",
                                  e.target.checked,
                                ),
                              "固定保留状态已更新。",
                            )
                          }
                        />
                      </td>
                      <td>
                        <Button
                          disabled={
                            busy ||
                            entry.fixed ||
                            !!entry.protected_results ||
                            entry.result_id === activeResultId
                          }
                          onClick={() =>
                            void action(
                              () =>
                                client.settings.release(
                                  project.id,
                                  entry.result_id,
                                ),
                              "查询缓存已清理，保存的查询条件仍然可用。",
                            )
                          }
                        >
                          清理
                        </Button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            {entries.hasNextPage && (
              <Button
                disabled={entries.isFetchingNextPage}
                onClick={() => void entries.fetchNextPage()}
              >
                加载更多结果
              </Button>
            )}
          </>
        ) : (
          <p className="settings-empty">当前项目尚无保留的查询结果。</p>
        )}
      </section>
      <section className="settings-section">
        <h4>批量清理</h4>
        <p className="settings-note">
          后台分批处理所有项目的可回收缓存，包括已关闭的项目。正在使用、已固定或被项目引用的成员会保留。
        </p>
        <div className="settings-toolbar">
          <Button
            disabled={busy || data.storage.cleanup_pending}
            onClick={() =>
              void action(
                () => client.settings.clear("temporary"),
                "已开始清理未使用的临时缓存。",
              )
            }
          >
            清理临时缓存
          </Button>
          <Button
            disabled={busy || data.storage.cleanup_pending}
            onClick={() =>
              void action(
                () => client.settings.clear("long_term"),
                "已开始清理未使用且未固定的长期缓存。",
              )
            }
          >
            清理未固定的长期缓存
          </Button>
          <Button
            disabled={busy}
            onClick={() =>
              void action(
                () => client.resources.clear(),
                "缩略图缓存正在后台清理。",
              )
            }
          >
            清理缩略图缓存
          </Button>
        </div>
        {data.storage.cleanup_pending && (
          <p className="settings-note" role="status">
            正在后台回收…
          </p>
        )}
      </section>
    </div>
  );
}
