import { useState, type ReactNode } from "react";
import type { StudioClient } from "@studio/client";
import type { Schema } from "@studio/contracts";
import { Button, DraftStatus, ErrorDetails, WorkbenchDialog } from "@studio/ui";
import { useLakePreference, useLakeRefresh } from "./queries.js";
import { PinterestOptions } from "./PinterestOptions.js";
import {
  decodePinterestDraft,
  initialPinterestDraft,
  pinterestDefinition,
  pinterestRange,
  pinterestSeedLabels,
  type PinterestDraft,
} from "./pinterestModel.js";

export function PinterestComposer({
  client,
  lakes,
  initialLake,
  sourceHeader,
  onClose,
  onCreated,
}: {
  client: StudioClient;
  lakes: Schema["LakeWorkspaceLake"][];
  initialLake: string;
  sourceHeader: ReactNode;
  onClose: () => void;
  onCreated: (id: string, kind: "job" | "schedule") => void;
}) {
  const draft = useLakePreference(
    client,
    "studio.lake-updates.pinterest-composer",
    initialPinterestDraft,
    decodePinterestDraft,
  );
  const d = draft.value,
    refresh = useLakeRefresh(client);
  const [preview, setPreview] = useState<Schema["PinterestPreview"] | null>(
      null,
    ),
    [pending, setPending] = useState(false),
    [error, setError] = useState<unknown>(null);
  const lakeId = lakes.some((l) => l.id === d.lakeId)
    ? d.lakeId
    : lakes.some((l) => l.id === initialLake)
      ? initialLake
      : (lakes[0]?.id ?? "");
  function change(patch: Partial<PinterestDraft>) {
    draft.controller.set({ ...d, ...patch });
    setPreview(null);
    setError(null);
  }
  async function inspect() {
    setPending(true);
    setError(null);
    try {
      if (!lakeId) throw new Error("请先创建一个 Pinterest 数据湖。");
      if (
        d.periodic &&
        (!Number.isSafeInteger(d.intervalHours * 3600) ||
          d.intervalHours < 1 / 60 ||
          d.intervalHours > 8784)
      )
        throw new Error("复查间隔需为 1 分钟至 366 天。");
      setPreview(
        await client.pinterestCollections.preview(
          pinterestDefinition(d, lakeId),
        ),
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
    try {
      let submission = d.submission;
      if (!submission) {
        if (!preview) throw new Error("请先检查任务摘要。");
        submission = {
          key: crypto.randomUUID(),
          spec: preview.definition,
          ...(d.periodic
            ? {
                schedule: {
                  id: crypto.randomUUID(),
                  everySeconds: d.intervalHours * 3600,
                  firstRunAt: new Date().toISOString(),
                  enabled: d.scheduleEnabled,
                },
              }
            : {}),
        };
        draft.controller.set({ ...d, submission });
        await draft.controller.flush();
      }
      const id =
        submission.id ??
        (submission.schedule
          ? (
              await client.pinterestCollections.saveSchedule({
                id: submission.schedule.id,
                request_key: submission.key,
                expected_revision: 0,
                definition: submission.spec,
                every_seconds: submission.schedule.everySeconds,
                first_run_at: submission.schedule.firstRunAt,
                enabled: submission.schedule.enabled,
              })
            ).id
          : (
              await client.pinterestCollections.create(
                submission.spec,
                submission.key,
              )
            ).id);
      draft.controller.set({ ...d, submission: { ...submission, id } });
      await draft.controller.flush();
      onCreated(id, submission.schedule ? "schedule" : "job");
      await refresh();
    } catch (e) {
      setError(e);
    } finally {
      setPending(false);
    }
  }
  return (
    <WorkbenchDialog title="新建数据湖更新" onClose={onClose}>
      <div className="lake-composer">
        {sourceHeader}
        <DraftStatus controller={draft.controller} quiet />
        <fieldset disabled={!draft.editable || pending || !!d.submission}>
          <details open>
            <summary>目标与范围</summary>
            <div className="lake-fields">
              <label>
                Pinterest 数据湖
                <select
                  aria-label="Pinterest 数据湖"
                  value={lakeId}
                  onChange={(e) => change({ lakeId: e.target.value })}
                >
                  {!lakes.length && <option value="">请先创建数据湖</option>}
                  {lakes.map((l) => (
                    <option key={l.id} value={l.id}>
                      {l.media}
                    </option>
                  ))}
                </select>
              </label>
              <label>
                种子类型
                <select
                  aria-label="Pinterest 种子类型"
                  value={d.seedKind}
                  onChange={(e) => change({ seedKind: e.target.value })}
                >
                  {Object.entries(pinterestSeedLabels).map(([key, label]) => (
                    <option key={key} value={key}>
                      {label}
                    </option>
                  ))}
                </select>
              </label>
              <label>
                {d.seedKind === "pin"
                  ? "Pin ID 或链接"
                  : d.seedKind.startsWith("search_")
                    ? "搜索查询"
                    : `${pinterestSeedLabels[d.seedKind]} ID 或链接`}
                <textarea
                  aria-label={
                    d.seedKind === "pin" ? "Pin ID 或链接" : "Pinterest 种子"
                  }
                  rows={5}
                  value={d.pins}
                  placeholder={
                    d.seedKind.startsWith("search_")
                      ? "每行一个查询，可包含空格"
                      : "每行一个 ID 或完整链接；Ideas 主题请使用完整链接"
                  }
                  onChange={(e) => change({ pins: e.target.value })}
                />
              </label>
              <p className="lake-hint">
                匿名读取公开范围；最多 500
                个种子。保存静态原图及支持的单图故事，保留来源观察与关系。未支持的媒体保留缺口。
              </p>
            </div>
          </details>
          <details open>
            <summary>本轮预算</summary>
            <div className="lake-fields">
              <label>
                最多来源请求
                <input
                  aria-label="最多来源请求"
                  type="number"
                  min={1}
                  value={d.requests}
                  onChange={(e) => change({ requests: Number(e.target.value) })}
                />
              </label>
              <label>
                最多详情请求
                <input
                  aria-label="最多详情请求"
                  type="number"
                  min={0}
                  value={d.detailRequests}
                  onChange={(e) =>
                    change({ detailRequests: Number(e.target.value) })
                  }
                />
              </label>
              <label>
                最多准入 Pin
                <input
                  aria-label="最多准入 Pin"
                  type="number"
                  min={1}
                  value={d.pinBudget}
                  onChange={(e) =>
                    change({ pinBudget: Number(e.target.value) })
                  }
                />
              </label>
              <label>
                最多准入图版
                <input
                  aria-label="最多准入图版"
                  type="number"
                  min={0}
                  value={d.boardBudget}
                  onChange={(e) =>
                    change({ boardBudget: Number(e.target.value) })
                  }
                />
              </label>
              <label>
                最多下载（MiB）
                <input
                  type="number"
                  min={1}
                  value={d.downloadMiB}
                  onChange={(e) =>
                    change({ downloadMiB: Number(e.target.value) })
                  }
                />
              </label>
              <label>
                最长运行（分钟）
                <input
                  type="number"
                  min={1}
                  max={10080}
                  value={d.minutes}
                  onChange={(e) => change({ minutes: Number(e.target.value) })}
                />
              </label>
              <p className="lake-hint">
                来源请求包括发现与详情，文件下载另计流量。预算用完后保留候选和游标，可追加同等预算继续。已开始的单个文件可能使下载量超过本轮预算。
              </p>
            </div>
          </details>
          <PinterestOptions d={d} change={change} />
        </fieldset>
        {(preview || d.submission) && (
          <section className="lake-summary" aria-label="Pinterest 任务摘要">
            <strong>
              {pinterestRange(d.submission?.spec ?? preview!.definition)}
            </strong>
            <p>原图 · 保留原始字节 · 匿名访问</p>
            <p>
              {d.periodic
                ? `周期复查 · 每 ${d.intervalHours} 小时 · ${d.scheduleEnabled ? "创建后启用" : "创建为未启用"}`
                : "执行单轮采集"}
            </p>
            <p>摘要检查不访问源站。</p>
          </section>
        )}
        {error != null && <ErrorDetails error={error} />}
        {d.submission?.id && (
          <p role="status">
            {d.submission.schedule ? "周期计划已创建" : "采集任务已创建"}
            ，可关闭配置继续浏览。
          </p>
        )}
        <footer className="lake-actions">
          <Button
            disabled={!draft.editable || pending || !!d.submission}
            onClick={() => void inspect()}
          >
            检查任务摘要
          </Button>
          <Button
            className="primary"
            disabled={
              !draft.editable ||
              pending ||
              (!preview && !d.submission) ||
              !!d.submission?.id
            }
            onClick={() => void submit()}
          >
            {d.submission && !d.submission.id
              ? "重试同一次提交"
              : d.periodic
                ? "创建周期计划"
                : "开始采集"}
          </Button>
          {d.submission && (
            <Button
              disabled={pending}
              onClick={() => change({ submission: null })}
            >
              编辑并新建另一轮
            </Button>
          )}
          <Button onClick={onClose}>关闭</Button>
        </footer>
      </div>
    </WorkbenchDialog>
  );
}
