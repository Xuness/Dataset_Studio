import { sourceSupports } from "@studio/client";
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button, ErrorDetails, ratingLabel } from "@studio/ui";
import type { SettingsPageProps } from "./types.js";
import { sizeLabel } from "./types.js";
import { CacheProjectManager } from "./CacheProjectManager.js";
import { usedAt } from "./cacheLabels.js";
import { cleanupPhase } from "./CacheCleanupProgress.js";

export function CacheManagerPage(props: SettingsPageProps) {
  const { client, project, sources, data, busy, action } = props;
  const [chosenSource, setChosenSource] = useState("");
  const lakes = sources.filter(
    (s) => sourceSupports(s, "post_order") && s.available,
  );
  const sourceId =
    lakes.find((s) => s.id === chosenSource)?.id ?? lakes[0]?.id ?? "";
  const bases = useQuery({
    queryKey: ["settings", "rating-bases", client.connection.instance_id],
    queryFn: ({ signal }) => client.settings.ratingBases(signal),
    refetchInterval: 1500,
  });
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
        <p>按项目核对查询、固定输入和排名索引，管理共享缓存与保留方式。</p>
      </div>
      <CacheProjectManager {...props} />
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
        <h4>批量清理</h4>
        {Number(data.storage.reusable_bytes) > 0 && (
          <p className="settings-note">
            已有 {sizeLabel(data.storage.reusable_bytes)}{" "}
            空间可供项目复用，后台会逐步归还磁盘空间。
          </p>
        )}
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
            {cleanupPhase(data.maintenance?.phase ?? "queued")}
            {data.maintenance?.project_id &&
            data.maintenance.project_id !== project?.id
              ? " · 正在处理其他项目"
              : ""}
            。可以继续使用项目。
          </p>
        )}
        {data.maintenance?.error && (
          <p className="error" role="alert">
            {data.maintenance.error}
          </p>
        )}
      </section>
    </div>
  );
}
