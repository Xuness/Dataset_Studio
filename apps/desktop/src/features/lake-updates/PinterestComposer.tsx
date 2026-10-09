import { useState, type ReactNode } from "react";
import type { StudioClient } from "@studio/client";
import type { Schema } from "@studio/contracts";
import { Button, DraftStatus, ErrorDetails, WorkbenchDialog } from "@studio/ui";
import { useLakePreference, useLakeRefresh } from "./queries.js";
import {
  decodePinterestDraft,
  initialPinterestDraft,
  pinterestDefinition,
  pinterestRange,
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
  onCreated: (id: string) => void;
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
        submission = { key: crypto.randomUUID(), spec: preview.definition };
        draft.controller.set({ ...d, submission });
        await draft.controller.flush();
      }
      const id =
        submission.id ??
        (
          await client.pinterestCollections.create(
            submission.spec,
            submission.key,
          )
        ).id;
      draft.controller.set({ ...d, submission: { ...submission, id } });
      await draft.controller.flush();
      onCreated(id);
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
                Pin ID 或链接
                <textarea
                  aria-label="Pin ID 或链接"
                  rows={5}
                  value={d.pins}
                  placeholder="每行一个 Pin ID 或 https://www.pinterest.com/pin/…/"
                  onChange={(e) => change({ pins: e.target.value })}
                />
              </label>
              <p className="lake-hint">
                匿名获取指定 Pin；最多 500
                个。保存静态原图及支持的单图故事，保留原始文件和来源响应。其他媒体保留缺口。
              </p>
            </div>
          </details>
          <details open>
            <summary>本轮预算</summary>
            <div className="lake-fields">
              <label>
                最多详情请求
                <input
                  aria-label="最多详情请求"
                  type="number"
                  min={1}
                  max={500}
                  value={d.requests}
                  onChange={(e) => change({ requests: Number(e.target.value) })}
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
                预算用完时保留进度；本阶段可取消剩余工作，再为未完成的 Pin
                创建新任务。
              </p>
            </div>
          </details>
        </fieldset>
        {(preview || d.submission) && (
          <section className="lake-summary" aria-label="Pinterest 任务摘要">
            <strong>
              {pinterestRange(d.submission?.spec ?? preview!.definition)}
            </strong>
            <p>原图 · 保留原始字节 · 匿名访问</p>
            <p>摘要检查不访问源站。</p>
          </section>
        )}
        {error != null && <ErrorDetails error={error} />}
        {d.submission?.id && (
          <p role="status">采集任务已创建，可关闭配置继续浏览。</p>
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
            {d.submission && !d.submission.id ? "重试同一次提交" : "开始采集"}
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
