import { useState } from "react";
import type { StudioClient } from "@studio/client";
import type { Schema } from "@studio/contracts";
import { Button, DraftStatus, ErrorDetails, WorkbenchDialog } from "@studio/ui";
import {
  decodeDraft,
  definitions,
  initialDraft,
  rangeLabel,
  policyLabel,
  sites,
} from "./model.js";
import type { FormDraft, Lake } from "./model.js";
import { useLakePreference, useLakeRefresh } from "./queries.js";

export function UpdateComposer({
  client,
  lakes,
  capabilities,
  onClose,
  onCreated,
}: {
  client: StudioClient;
  lakes: Lake[];
  capabilities: Schema["LakeUpdateCapability"][];
  onClose: () => void;
  onCreated: (id: string, kind: "job" | "schedule") => void;
}) {
  const draft = useLakePreference(
    client,
    "studio.lake-updates.composer",
    initialDraft,
    decodeDraft,
  );
  const d = draft.value,
    refresh = useLakeRefresh(client);
  const [previews, setPreviews] = useState<Schema["LakeUpdatePreview"][]>([]);
  const [error, setError] = useState<unknown>(null),
    [pending, setPending] = useState(false);
  const [notice, setNotice] = useState("");
  const frozen = d.submissions.length > 0;
  function set(values: Partial<FormDraft>) {
    draft.controller.set({ ...d, ...values });
    setPreviews([]);
    setError(null);
  }
  async function inspect() {
    setPending(true);
    setError(null);
    try {
      const specs = definitions(d);
      for (const spec of specs) {
        const lake = lakes.find((l) => l.id === spec.library_id);
        if (!lake) throw new Error("目标数据湖尚未登记");
        if (
          spec.range.kind === "changes" &&
          !capabilities.find((c) => c.site === lake.site)?.change_sequence
        )
          throw new Error(`${sites[lake.site]} 不支持变更序号范围`);
      }
      if (d.execution !== "now") {
        if (!d.firstRun || !Number.isFinite(new Date(d.firstRun).getTime()))
          throw new Error("请选择首次执行时间");
        if (
          d.execution === "interval" &&
          (!Number.isFinite(Number(d.intervalHours)) ||
            Number(d.intervalHours) < 1 / 60 ||
            Number(d.intervalHours) > 8784)
        )
          throw new Error("执行间隔需为 1 分钟至 366 天");
      }
      setPreviews(
        await Promise.all(specs.map((s) => client.lakeUpdates.preview(s))),
      );
    } catch (e) {
      setError(e);
    } finally {
      setPending(false);
    }
  }
  async function submit() {
    setPending(true);
    setError(null);
    setNotice("");
    try {
      let submissions = d.submissions;
      if (!submissions.length) {
        if (!previews.length) throw new Error("请先检查任务摘要");
        submissions = previews.map((p) => ({
          key: crypto.randomUUID(),
          spec: p.definition,
        }));
        draft.controller.set({ ...d, submissions });
        await draft.controller.flush();
      }
      for (let i = 0; i < submissions.length; i++) {
        const item = submissions[i]!;
        if (item.jobId) continue;
        let id: string;
        if (d.execution === "now")
          id = (await client.lakeUpdates.create(item.spec, item.key)).id;
        else {
          const existing = (await client.lakeUpdates.schedules()).items.find(
            (s) => s.id === item.key,
          );
          id =
            existing?.id ??
            (
              await client.lakeUpdates.saveSchedule({
                identity: item.key,
                revision: null,
                spec: item.spec,
                enabled: false,
                first_run_at: new Date(d.firstRun).toISOString(),
                every_seconds:
                  d.execution === "interval"
                    ? Math.round(Number(d.intervalHours) * 3600)
                    : null,
              })
            ).id;
        }
        submissions = submissions.map((s, n) =>
          n === i ? { ...s, jobId: id } : s,
        );
        draft.controller.set({ ...d, submissions });
        await draft.controller.flush();
        onCreated(id, d.execution === "now" ? "job" : "schedule");
      }
      setNotice(
        d.execution === "now"
          ? "各湖任务已创建，可以关闭配置继续浏览。"
          : "计划已保存为未启用，请在计划列表检查后启用。",
      );
      await refresh();
    } catch (e) {
      setError(e);
      await refresh();
    } finally {
      setPending(false);
    }
  }
  const done = frozen && d.submissions.every((s) => s.jobId);
  return (
    <WorkbenchDialog title="新建数据湖更新" onClose={onClose}>
      <div className="lake-composer">
        <DraftStatus controller={draft.controller} quiet />
        <fieldset disabled={!draft.editable || pending || frozen}>
          <details open>
            <summary>目标数据湖</summary>
            <div className="lake-fields">
              {lakes.map((l) => (
                <label className="lake-check" key={l.id}>
                  <input
                    type="checkbox"
                    checked={d.lakes.includes(l.id)}
                    onChange={(e) =>
                      set({
                        lakes: e.target.checked
                          ? [...d.lakes, l.id]
                          : d.lakes.filter((id) => id !== l.id),
                      })
                    }
                  />
                  {sites[l.site]}
                </label>
              ))}
              {!lakes.length && <p>先登记一个可更新的数据湖。</p>}
            </div>
          </details>
          <details open>
            <summary>更新范围</summary>
            <div className="lake-fields">
              <label>
                范围
                <select
                  aria-label="范围"
                  value={d.kind}
                  onChange={(e) =>
                    set({ kind: e.target.value as FormDraft["kind"] })
                  }
                >
                  <option value="new">补充新帖</option>
                  <option value="local">刷新已有记录</option>
                  <option value="missing">补齐本地缺图</option>
                  <option value="created">指定创建日期</option>
                  <option value="updated">指定最后修改日期</option>
                  <option value="ids">指定帖子 ID 列表</option>
                  <option value="id_range">指定帖子 ID 区间</option>
                  <option value="changes">变更序号（Yandere）</option>
                  {Object.keys(d.inputIds).length > 0 && (
                    <option value="input">已准备的固定成员范围</option>
                  )}
                </select>
              </label>
              {["new", "ids", "changes"].includes(d.kind) &&
                d.lakes.map((id) => {
                  const lake = lakes.find((l) => l.id === id);
                  if (!lake) return null;
                  const p = d.perLake[id] ?? {};
                  return (
                    <label key={id}>
                      {sites[lake.site]} ·{" "}
                      {d.kind === "ids"
                        ? "帖子 ID"
                        : d.kind === "changes"
                          ? "变更序号起点"
                          : "起点 ID（不含）"}
                      {d.kind === "ids" ? (
                        <textarea
                          rows={3}
                          value={p.ids ?? ""}
                          placeholder="使用空格、换行或逗号分隔"
                          onChange={(e) =>
                            set({
                              perLake: {
                                ...d.perLake,
                                [id]: { ...p, ids: e.target.value },
                              },
                            })
                          }
                        />
                      ) : (
                        <input
                          inputMode="numeric"
                          value={p.after ?? ""}
                          placeholder={
                            d.kind === "new" ? "留空接续已验证基线" : "填写序号"
                          }
                          onChange={(e) =>
                            set({
                              perLake: {
                                ...d.perLake,
                                [id]: { ...p, after: e.target.value },
                              },
                            })
                          }
                        />
                      )}
                    </label>
                  );
                })}
              {d.kind === "new" && (
                <p className="lake-hint">
                  首次没有已验证基线时必须指定起点；导入文件中的最大 ID
                  不代表完整覆盖。
                </p>
              )}
              {["created", "updated"].includes(d.kind) && (
                <>
                  <label>
                    开始日期
                    <input
                      type="date"
                      value={d.from}
                      onChange={(e) => set({ from: e.target.value })}
                    />
                  </label>
                  <label>
                    结束日期（含当天）
                    <input
                      type="date"
                      value={d.until}
                      onChange={(e) => set({ until: e.target.value })}
                    />
                  </label>
                  <label>
                    时区
                    <input
                      value={d.timezone}
                      onChange={(e) => set({ timezone: e.target.value })}
                    />
                  </label>
                  <p className="lake-hint">
                    日期条件可能需要扫描候选
                    ID。最后修改时间不等于该日期全部历史变更事件。
                  </p>
                </>
              )}
              {!["new", "ids", "input"].includes(d.kind) && (
                <>
                  <label>
                    起始 ID（含）
                    <input
                      inputMode="numeric"
                      value={d.startId}
                      onChange={(e) => set({ startId: e.target.value })}
                    />
                  </label>
                  <label>
                    结束 ID（含）
                    <input
                      inputMode="numeric"
                      value={d.endId}
                      onChange={(e) => set({ endId: e.target.value })}
                    />
                  </label>
                </>
              )}
              {["local", "missing"].includes(d.kind) && (
                <label>
                  上次获取早于（可选）
                  <input
                    type="datetime-local"
                    value={d.observedBefore}
                    onChange={(e) => set({ observedBefore: e.target.value })}
                  />
                </label>
              )}
            </div>
          </details>
          <details open>
            <summary>更新内容</summary>
            <div className="lake-fields">
              <label>
                保存策略
                <select
                  aria-label="保存策略"
                  value={d.profile}
                  onChange={(e) =>
                    set({
                      profile: e.target.value as FormDraft["profile"],
                      allowSample: false,
                    })
                  }
                >
                  <option value="">请选择策略</option>
                  <option value="metadata_only">仅元数据</option>
                  <option value="original">保存原图</option>
                  <option value="webp-2048-q95">
                    WebP · 最长边 2048 / 质量 95
                  </option>
                </select>
              </label>
              {d.profile && d.profile !== "metadata_only" && (
                <>
                  <label>
                    已有图片
                    <select
                      aria-label="已有图片"
                      value={d.existing}
                      onChange={(e) =>
                        set({
                          existing: e.target.value as FormDraft["existing"],
                        })
                      }
                    >
                      <option value="keep">保留可复用图片，只补缺图</option>
                      <option value="match_profile">
                        补入符合所选策略的版本
                      </option>
                    </select>
                  </label>
                  <p className="lake-hint">
                    保存原图不会自动替换全部已有 HF 缩图。旧版本会保留。
                  </p>
                  {d.profile !== "original" && (
                    <label className="lake-check">
                      <input
                        type="checkbox"
                        checked={d.allowSample}
                        onChange={(e) => set({ allowSample: e.target.checked })}
                      />
                      原件不可用时允许备用图片
                    </label>
                  )}
                </>
              )}
            </div>
          </details>
          <details open>
            <summary>执行方式</summary>
            <div className="lake-fields">
              <label>
                执行
                <select
                  aria-label="执行"
                  value={d.execution}
                  onChange={(e) =>
                    set({ execution: e.target.value as FormDraft["execution"] })
                  }
                >
                  <option value="now">立即创建任务</option>
                  <option value="once">保存单次预约计划</option>
                  <option value="interval">保存固定间隔计划</option>
                </select>
              </label>
              {d.execution !== "now" && (
                <>
                  <label>
                    首次执行（本机时间）
                    <input
                      type="datetime-local"
                      value={d.firstRun}
                      onChange={(e) => set({ firstRun: e.target.value })}
                    />
                  </label>
                  {d.execution === "interval" && (
                    <label>
                      间隔（小时）
                      <input
                        type="number"
                        min="0.016667"
                        step="any"
                        value={d.intervalHours}
                        onChange={(e) => set({ intervalHours: e.target.value })}
                      />
                    </label>
                  )}
                  <p className="lake-hint">
                    计划保存后手动启用。固定间隔不等于日历每天固定时刻；日期范围会原样重复，运行时不会自动改为“昨天”。
                  </p>
                </>
              )}
            </div>
          </details>
          <details>
            <summary>高级预算</summary>
            <div className="lake-fields">
              <label>
                每轮最多扫描页数
                <input
                  inputMode="numeric"
                  value={d.pageBudget}
                  onChange={(e) => set({ pageBudget: e.target.value })}
                />
              </label>
              <label>
                每轮最多处理记录数
                <input
                  inputMode="numeric"
                  value={d.itemBudget}
                  onChange={(e) => set({ itemBudget: e.target.value })}
                />
              </label>
              <p className="lake-hint">
                达到预算会暂停；继续时从原检查点开始下一轮。
              </p>
            </div>
          </details>
        </fieldset>
        {previews.length > 0 && (
          <section aria-label="任务摘要" className="lake-summary">
            {previews.map((p) => (
              <div key={p.definition.library_id}>
                <strong>
                  {
                    sites[
                      lakes.find((l) => l.id === p.definition.library_id)!.site
                    ]
                  }
                </strong>
                <p>
                  {rangeLabel(p.definition)} · {policyLabel(p.definition)}
                </p>
                <small>
                  候选数量：{p.known_candidates ?? "尚未知"} ·{" "}
                  {p.scan_strategy === "id_filtered_scan"
                    ? "按 ID 扫描并过滤日期"
                    : "按范围游标获取"}
                </small>
              </div>
            ))}
          </section>
        )}
        {frozen && (
          <div className="lake-summary">
            {d.submissions.map((s) => (
              <p key={s.key}>
                {
                  sites[
                    lakes.find((l) => l.id === s.spec.library_id)?.site ??
                      "danbooru"
                  ]
                }
                ：{s.jobId ? "已创建" : "尚未确认，重试沿用原提交"}
              </p>
            ))}
          </div>
        )}
        {error != null && <ErrorDetails error={error} />}
        {notice && <p role="status">{notice}</p>}
        <footer className="lake-actions">
          <Button onClick={onClose}>关闭</Button>
          {frozen ? (
            <>
              {!done && (
                <Button
                  disabled={pending}
                  className="primary"
                  onClick={() => void submit()}
                >
                  继续提交未确认任务
                </Button>
              )}
              <Button
                disabled={pending || !done}
                onClick={() => {
                  draft.controller.set({ ...d, submissions: [] });
                  setPreviews([]);
                  setNotice("");
                }}
              >
                配置另一批更新
              </Button>
            </>
          ) : (
            <>
              <Button
                disabled={pending || !draft.editable}
                onClick={() => void inspect()}
              >
                检查任务摘要
              </Button>
              <Button
                className="primary"
                disabled={pending || !previews.length}
                onClick={() => void submit()}
              >
                {d.execution === "now" ? "开始更新" : "保存未启用计划"}
              </Button>
            </>
          )}
        </footer>
      </div>
    </WorkbenchDialog>
  );
}
