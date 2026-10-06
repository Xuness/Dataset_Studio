import { Button } from "@studio/ui";
import type { Schema } from "@studio/contracts";
import { sizeLabel } from "./types.js";
import type { SettingsPageProps } from "./types.js";

const capacities = [
  ["long_term_mib", "长期缓存", "包含分级基础、排序索引和长期查询结果。"],
  ["temporary_mib", "临时缓存", "用于 Tag 和其他组合查询的短期复用。"],
  [
    "preview_mib",
    "图片缓存",
    "用于预览图片及按需生成的 API 缩图，可随时重建。",
  ],
] as const;
function validate(value: Schema["CacheSettings"]) {
  const sizes = [
    value.total_mib,
    value.long_term_mib,
    value.temporary_mib,
    value.preview_mib,
  ];
  if (sizes.some((n) => !Number.isInteger(n) || n < 0 || n > 1048576))
    return "容量须为 0–1024 GiB，并以 MiB 为最小单位。";
  if (sizes.slice(1).reduce((sum, n) => sum + n, 0) > value.total_mib)
    return "分类容量之和超过总预算，请调整其中一项。";
  if (
    !Number.isInteger(value.temporary_idle_hours) ||
    value.temporary_idle_hours < 1 ||
    value.temporary_idle_hours > 2160
  )
    return "临时缓存的闲置期限须为 1–2160 小时。";
  if (
    value.long_term_idle_days != null &&
    (!Number.isInteger(value.long_term_idle_days) ||
      value.long_term_idle_days < 1 ||
      value.long_term_idle_days > 3650)
  )
    return "长期缓存的闲置期限须为 1–3650 天。";
  return "";
}
export function CacheSettingsPage({
  data,
  client,
  busy,
  action,
  cacheDraft,
  setCacheDraft,
}: SettingsPageProps) {
  const value = cacheDraft ?? data.cache;
  const update = (patch: Partial<Schema["CacheSettings"]>) =>
    setCacheDraft({ ...value, ...patch });
  const issue = validate(value);
  const allocated =
    value.long_term_mib + value.temporary_mib + value.preview_mib;
  const storage = data.storage;
  return (
    <form
      className="settings-page"
      onSubmit={(event) => {
        event.preventDefault();
        if (!cacheDraft || issue || busy) return;
        void action(async () => {
          await client.settings.configureCache(value);
          setCacheDraft(null);
        }, "缓存设置已保存，后台会按新的规则分批回收。");
      }}
    >
      <div className="settings-page-heading">
        <h3>缓存与存储</h3>
        <p>统一安排缓存空间，并为不同用途设置保留方式。</p>
      </div>
      <div className="settings-usage-grid">
        <div>
          <span>当前合计</span>
          <strong>{sizeLabel(storage.total_bytes)}</strong>
          <small>预算 {sizeLabel(storage.total_quota_bytes)}</small>
        </div>
        <div>
          <span>长期与基础索引</span>
          <strong>{sizeLabel(storage.long_term_bytes)}</strong>
          <small>{storage.long_term_results} 组长期查询</small>
        </div>
        <div>
          <span>临时类占用</span>
          <strong>{sizeLabel(storage.temporary_bytes)}</strong>
          <small>
            {storage.temporary_results} 组临时查询 · 排名浏览索引{" "}
            {sizeLabel(storage.ranked_index_bytes ?? "0")}
          </small>
        </div>
        <div>
          <span>图片缓存</span>
          <strong>{sizeLabel(storage.preview_bytes)}</strong>
          <small>预览、API 缩图与索引文件</small>
        </div>
      </div>
      <p className="settings-note">
        固定保留与必需索引 {sizeLabel(storage.fixed_member_bytes)} · 被项目引用{" "}
        {storage.protected_results} 份结果 · 正在读取 {storage.active_views}{" "}
        份结果。临时类也包含项目固定输入；在缓存管理中可按项目查看归属、引用和排名索引。成员空间为分摊估算。
      </p>
      <fieldset disabled={busy} className="settings-fields">
        <section className="settings-section">
          <h4>磁盘预算</h4>
          <div className="settings-control-row">
            <div>
              <label htmlFor="settings-total">总缓存预算</label>
              <p>各类缓存合计使用的预算，支持 50、64、100 GiB 等容量。</p>
            </div>
            <div className="settings-number">
              <input
                id="settings-total"
                aria-label="总缓存预算 GiB"
                type="number"
                min={0}
                max={1024}
                step="any"
                value={value.total_mib / 1024}
                onChange={(e) =>
                  update({
                    total_mib: Math.round(Number(e.target.value) * 1024),
                  })
                }
              />
              <span>GiB</span>
            </div>
          </div>
          {capacities.map(([key, title, description]) => (
            <div className="settings-control-row" key={key}>
              <div>
                <label htmlFor={"settings-" + key}>{title}</label>
                <p>{description}</p>
              </div>
              <div className="settings-number">
                <input
                  id={"settings-" + key}
                  aria-label={title + "预算 GiB"}
                  type="number"
                  min={0}
                  max={1024}
                  step="any"
                  value={value[key] / 1024}
                  onChange={(e) =>
                    update({ [key]: Math.round(Number(e.target.value) * 1024) })
                  }
                />
                <span>GiB</span>
              </div>
            </div>
          ))}
          <p
            className={
              allocated > value.total_mib
                ? "settings-validation"
                : "settings-note"
            }
          >
            已分配 {(allocated / 1024).toFixed(2)} /{" "}
            {(value.total_mib / 1024).toFixed(2)} GiB。类别容量设为 0
            后，不继续保留该类的可回收缓存。
          </p>
        </section>
        <section className="settings-section">
          <h4>长期缓存</h4>
          <div className="settings-control-row">
            <div>
              <label htmlFor="settings-long-expiry">未使用后清理</label>
              <p>纯分级查询默认长期保留，仍会按来源变化增量更新。</p>
            </div>
            <select
              id="settings-long-expiry"
              aria-label="长期缓存过期方式"
              value={value.long_term_idle_days == null ? "never" : "days"}
              onChange={(e) =>
                update({
                  long_term_idle_days: e.target.value === "never" ? null : 90,
                })
              }
            >
              <option value="never">不自动过期</option>
              <option value="days">按闲置天数清理</option>
            </select>
          </div>
          {value.long_term_idle_days != null && (
            <div className="settings-control-row">
              <label htmlFor="settings-long-days">最长闲置时间</label>
              <div className="settings-number">
                <input
                  id="settings-long-days"
                  aria-label="长期缓存闲置天数"
                  type="number"
                  min={1}
                  max={3650}
                  value={value.long_term_idle_days}
                  onChange={(e) =>
                    update({ long_term_idle_days: Number(e.target.value) })
                  }
                />
                <span>天</span>
              </div>
            </div>
          )}
        </section>
        <section className="settings-section">
          <h4>临时缓存</h4>
          <div className="settings-control-row">
            <div>
              <label htmlFor="settings-temporary-mode">保留方式</label>
              <p>普通 Tag 搜索、组合条件和排名浏览索引使用临时缓存。</p>
            </div>
            <select
              id="settings-temporary-mode"
              aria-label="临时缓存保留方式"
              value={value.temporary_session_only ? "session" : "idle"}
              onChange={(e) =>
                update({ temporary_session_only: e.target.value === "session" })
              }
            >
              <option value="idle">按闲置时间清理</option>
              <option value="session">仅本次会话</option>
            </select>
          </div>
          {!value.temporary_session_only ? (
            <div className="settings-control-row">
              <div>
                <label htmlFor="settings-temporary-hours">最长闲置时间</label>
                <p>从最后一次使用起计时，默认 24 小时（一天）。</p>
              </div>
              <div className="settings-number">
                <input
                  id="settings-temporary-hours"
                  aria-label="临时缓存闲置小时"
                  type="number"
                  min={1}
                  max={2160}
                  value={value.temporary_idle_hours}
                  onChange={(e) =>
                    update({ temporary_idle_hours: Number(e.target.value) })
                  }
                />
                <span>小时</span>
              </div>
            </div>
          ) : (
            <p className="settings-note">
              项目会话结束后清理；仍被其他会话使用的结果，等最后一个使用者退出后再回收。
            </p>
          )}
        </section>
      </fieldset>
      <p className="settings-note">
        容量紧张时可提前淘汰未固定的结果。正在查看、固定保留和被工作集等引用的成员不会被普通清理删除，因此占用可能暂时超过预算。项目成果另行保存；运行暂存当前为{" "}
        {sizeLabel(storage.working_temporary_bytes)}。
      </p>
      {issue && (
        <p className="settings-validation" role="alert">
          {issue}
        </p>
      )}
      <div className="settings-page-actions">
        <Button
          type="button"
          disabled={busy || !cacheDraft}
          onClick={() => setCacheDraft(null)}
        >
          撤销修改
        </Button>
        <Button
          type="submit"
          className="primary"
          disabled={busy || !cacheDraft || !!issue}
        >
          保存缓存设置
        </Button>
      </div>
    </form>
  );
}
