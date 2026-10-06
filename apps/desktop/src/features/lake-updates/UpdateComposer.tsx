import { useState } from "react";
import type { ReactNode } from "react";
import { ImagePolicyEditor } from "./ImagePolicyEditor.js";
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
  sourceHeader,
}: {
  client: StudioClient;
  lakes: Lake[];
  capabilities: Schema["LakeUpdateCapability"][];
  onClose: () => void;
  onCreated: (id: string, kind: "job" | "schedule") => void;
  sourceHeader?: ReactNode;
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
        {sourceHeader}
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
                  <option value="tags">按标签 / 画师 / 角色采集</option>
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
              {d.kind === "tags" && (
                <>
                  <label>
                    全部包含 Tag
                    <textarea
                      aria-label="全部包含 Tag"
                      rows={2}
                      value={d.tagAll}
                      placeholder="例如 artist_name character_name"
                      onChange={(e) => set({ tagAll: e.target.value })}
                    />
                  </label>
                  <label>
                    任一包含 Tag
                    <textarea
                      aria-label="任一包含 Tag"
                      rows={2}
                      value={d.tagAny}
                      onChange={(e) => set({ tagAny: e.target.value })}
                    />
                  </label>
                  <label>
                    排除 Tag
                    <textarea
                      aria-label="排除 Tag"
                      rows={2}
                      value={d.tagNone}
                      onChange={(e) => set({ tagNone: e.target.value })}
                    />
                  </label>
                  <p className="lake-hint">
                    使用源站精确标签，以空格、逗号或换行分隔。画师与角色填写对应
                    Tag。 图片须满足全部包含、任一包含（若填写）且不含排除标签。
                    会保留候选元数据，仅下载符合条件的图片；可用下方 ID
                    限定采集范围。
                  </p>
                </>
              )}
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
            <ImagePolicyEditor client={client} value={d} onChange={set} />
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
                  type="number"
                  min={1}
                  max={100000}
                  inputMode="numeric"
                  value={d.pageBudget}
                  onChange={(e) => set({ pageBudget: e.target.value })}
                />
              </label>
              <p className="lake-hint">
                一页是一次候选元数据扫描，不是图片下载数量。
                {d.lakes.map((id) => {
                  const lake = lakes.find((l) => l.id === id);
                  const capability = capabilities.find(
                    (c) => c.site === lake?.site,
                  );
                  return lake && capability
                    ? ` ${sites[lake.site]} 每页上限 ${capability.page_size} 条。`
                    : "";
                })}
                过滤后不足一页或没有匹配记录，也可能消耗扫描页数。
              </p>
              <label>
                每轮最多处理记录数
                <input
                  type="number"
                  min={1}
                  max={10000000}
                  inputMode="numeric"
                  value={d.itemBudget}
                  onChange={(e) => set({ itemBudget: e.target.value })}
                />
              </label>
              <p className="lake-hint">
                {d.kind === "tags"
                  ? "标签采集按返回的候选元数据计数，包括未命中完整条件的记录；"
                  : "按 API 返回并匹配范围的帖子记录计数；"}
                复用、仅元数据和无图记录也计入，
                不代表成功下载的图片数。任一预算耗尽会暂停，需点“继续”从检查点开始下一轮，
                每轮预算重新计算。任务范围由上方选项决定；希望一次跑完时请为预算留出余量。
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
                    : p.scan_strategy === "single_tag_local_filter"
                      ? "按标签获取候选，本地判断组合条件"
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
