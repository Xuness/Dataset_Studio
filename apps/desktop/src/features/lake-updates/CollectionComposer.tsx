import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import type { StudioClient } from "@studio/client";
import type { Schema } from "@studio/contracts";
import { Button, DraftStatus, ErrorDetails, WorkbenchDialog } from "@studio/ui";
import { ImagePolicyEditor } from "./ImagePolicyEditor.js";
import { ImagePolicySummary } from "./ImagePolicySummary.js";
import { lakeKey, useLakePreference, useLakeRefresh } from "./queries.js";
import {
  collectionDefinition,
  collectionRange,
  decodeCollectionDraft,
  draftForCollection,
  initialCollectionDraft,
  publicAccount,
} from "./collectionModel.js";
import type {
  CollectionDefinition,
  CollectionDraft,
  WorkspaceLake,
} from "./collectionModel.js";

export function CollectionComposer({
  client,
  lakes,
  initialLake,
  preset,
  sourceHeader,
  onClose,
  onCreated,
}: {
  client: StudioClient;
  lakes: WorkspaceLake[];
  initialLake?: string | undefined;
  preset?: CollectionDefinition | undefined;
  sourceHeader: ReactNode;
  onClose: () => void;
  onCreated: (id: string, kind: "job" | "schedule") => void;
}) {
  const draft = useLakePreference(
    client,
    "studio.lake-updates.pixiv-composer",
    initialCollectionDraft,
    decodeCollectionDraft,
  );
  const d = draft.value,
    refresh = useLakeRefresh(client);
  const initialized = useRef(false);
  useEffect(() => {
    if (!draft.editable || initialized.current) return;
    initialized.current = true;
    if (preset) draft.controller.set(draftForCollection(preset));
  }, [draft.editable, draft.controller, preset]);
  const accounts = useQuery({
    queryKey: [...lakeKey(client), "collection-accounts"],
    queryFn: ({ signal }) =>
      client.sourceCollections.accounts({ limit: 200, signal }),
  });
  const [preview, setPreview] = useState<Schema["CollectionPreview"] | null>(
    null,
  );
  const [pending, setPending] = useState(false),
    [error, setError] = useState<unknown>(null),
    [notice, setNotice] = useState("");
  const lakeId = lakes.some((l) => l.id === d.lakeId)
    ? d.lakeId
    : (lakes.some((l) => l.id === initialLake) ? initialLake! : lakes[0]?.id) ||
      "";
  const frozen = d.submission !== null;
  function change(patch: Partial<CollectionDraft>) {
    draft.controller.set({ ...d, ...patch });
    setPreview(null);
    setError(null);
    setNotice("");
  }
  function choices(
    field: "workTypes" | "ratings" | "entrypoints",
    values: [string, string][],
  ) {
    return values.map(([id, label]) => (
      <label className="lake-check" key={id}>
        <input
          type="checkbox"
          checked={d[field].includes(id)}
          onChange={(e) =>
            change({
              [field]: e.target.checked
                ? [...d[field], id]
                : d[field].filter((v) => v !== id),
            })
          }
        />
        {label}
      </label>
    ));
  }
  async function inspect() {
    setPending(true);
    setError(null);
    try {
      if (!lakeId) throw new Error("先创建或登记一个 Pixiv 数据湖。");
      const account =
        d.accountId ||
        (await publicAccount(client, accounts.data?.items ?? [])).id;
      const spec = collectionDefinition(d, lakeId, account);
      if (
        d.periodic &&
        (!Number.isSafeInteger(d.intervalHours * 3600) ||
          d.intervalHours < 1 / 60 ||
          d.intervalHours > 8784)
      )
        throw new Error("周期需为 1 分钟至 366 天。");
      setPreview(await client.sourceCollections.preview(spec));
      await accounts.refetch();
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
          scheduleId: crypto.randomUUID(),
          spec: preview.definition,
          periodic: d.periodic,
          everySeconds: Math.round(d.intervalHours * 3600),
          firstRunAt: new Date().toISOString(),
        };
        draft.controller.set({ ...d, submission });
        await draft.controller.flush();
      }
      let id = submission.id;
      let coalesced = false;
      if (!id) {
        if (submission.periodic)
          id = (
            await client.sourceCollections.saveSchedule({
              id: submission.scheduleId,
              request_key: submission.key,
              expected_revision: 0,
              definition: submission.spec,
              every_seconds: submission.everySeconds,
              first_run_at: submission.firstRunAt,
              enabled: true,
            })
          ).id;
        else {
          const result = await client.sourceCollections.create(
            submission.spec,
            submission.key,
          );
          id = result.job.id;
          coalesced = Boolean(result.coalesced);
        }
        draft.controller.set({ ...d, submission: { ...submission, id } });
        await draft.controller.flush();
      }
      setNotice(
        submission.periodic
          ? "周期计划已启用；上一轮未结束时会等待，漏跑周期合并。"
          : coalesced
            ? "已有相同范围的未结束任务，已定位到该任务。"
            : "采集任务已创建，可关闭配置继续浏览。",
      );
      onCreated(id, submission.periodic ? "schedule" : "job");
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
        <fieldset disabled={!draft.editable || pending || frozen}>
          <details open>
            <summary>目标与范围</summary>
            <div className="lake-fields">
              <label>
                Pixiv 数据湖
                <select
                  aria-label="Pixiv 数据湖"
                  value={lakeId}
                  onChange={(e) => change({ lakeId: e.target.value })}
                >
                  {!lakes.length && <option value="">先登记数据湖</option>}
                  {lakes.map((l) => (
                    <option key={l.id} value={l.id}>
                      {l.media}
                    </option>
                  ))}
                </select>
              </label>
              <label>
                访问方式
                <select
                  aria-label="访问方式"
                  value={
                    accounts.data?.items.find((a) => a.id === d.accountId)
                      ?.mode === "anonymous"
                      ? ""
                      : d.accountId
                  }
                  onChange={(e) => change({ accountId: e.target.value })}
                >
                  <option value="">公开访问 · 无需登录</option>
                  {accounts.data?.items
                    .filter((a) => a.mode === "session")
                    .map((a) => (
                      <option key={a.id} value={a.id}>
                        {a.label}
                        {a.state !== "valid" ? " · 需要验证" : ""}
                      </option>
                    ))}
                </select>
              </label>
              <label>
                采集范围
                <select
                  aria-label="Pixiv 采集范围"
                  value={d.kind}
                  onChange={(e) =>
                    change({ kind: e.target.value as CollectionDraft["kind"] })
                  }
                >
                  <option value="authors">作者作品目录</option>
                  <option value="works">指定作品</option>
                </select>
              </label>
              <label>
                {d.kind === "authors" ? "作者 ID 或链接" : "作品 ID 或链接"}
                <textarea
                  aria-label="Pixiv 种子"
                  rows={4}
                  value={d.seeds}
                  placeholder="每行一个 ID 或 Pixiv 链接，也可用逗号分隔"
                  onChange={(e) => change({ seeds: e.target.value })}
                />
              </label>
              <div className="lake-actions">
                {choices("workTypes", [
                  ["illustration", "插画"],
                  ["manga", "漫画／多页"],
                  ["ugoira", "Ugoira 动画"],
                ])}
              </div>
              <div className="lake-actions">
                {choices("ratings", [
                  ["all_ages", "全年龄"],
                  ["r18", "R-18"],
                  ["r18g", "R-18G"],
                ])}
              </div>
              <label className="lake-check">
                <input
                  type="checkbox"
                  checked={d.includeAi}
                  onChange={(e) => change({ includeAi: e.target.checked })}
                />
                包含 AI 作品
              </label>
              <label className="lake-check">
                <input
                  type="checkbox"
                  checked={d.includeUnknown}
                  onChange={(e) => change({ includeUnknown: e.target.checked })}
                />
                包含分级或 AI 标记未知的作品
              </label>
              <p className="lake-hint">
                按所选访问方式实际可见的内容采集。登录凭据在“设置 → 数据湖
                API”管理。
              </p>
            </div>
          </details>
          {d.kind === "authors" && (
            <details>
              <summary>向外发现作者</summary>
              <div className="lake-fields">
                <label>
                  扩展深度
                  <input
                    aria-label="扩展深度"
                    type="number"
                    min={0}
                    max={4}
                    value={d.depth}
                    onChange={(e) => change({ depth: Number(e.target.value) })}
                  />
                </label>
                {d.depth > 0 && (
                  <>
                    {choices("entrypoints", [
                      ["bookmarks", "公开收藏"],
                      ["following", "公开关注"],
                      ["recommendations", "相关作品推荐"],
                    ])}
                    {d.entrypoints.includes("recommendations") && (
                      <label>
                        每位作者的推荐采样作品数
                        <input
                          type="number"
                          min={1}
                          max={32}
                          value={d.recommendationSeeds}
                          onChange={(e) =>
                            change({
                              recommendationSeeds: Number(e.target.value),
                            })
                          }
                        />
                      </label>
                    )}
                  </>
                )}
                <p className="lake-hint">
                  0
                  只补全种子作者；每增加一跳，沿所选关系发现作者并补全其目录。任务受本轮作者数和请求预算限制。
                </p>
              </div>
            </details>
          )}
          <details open>
            <summary>保存策略</summary>
            <ImagePolicyEditor
              collection
              client={client}
              value={d}
              onChange={(patch) =>
                change({
                  ...patch,
                  existing: "match_profile",
                  allowSample: false,
                })
              }
            />
            {d.profile !== "metadata_only" && (
              <div className="lake-fields">
                {!["original", ""].includes(d.profile) && (
                  <label className="lake-check">
                    <input
                      type="checkbox"
                      checked={d.retainOriginal}
                      onChange={(e) =>
                        change({ retainOriginal: e.target.checked })
                      }
                    />
                    同时保留原图
                  </label>
                )}
                <label>
                  Ugoira
                  <select
                    value={d.ugoira}
                    onChange={(e) => change({ ugoira: e.target.value })}
                  >
                    <option value="archive_with_poster">
                      保存动画包、帧时序和浏览封面
                    </option>
                    <option value="metadata_only">仅保存动画元数据</option>
                  </select>
                </label>
              </div>
            )}
          </details>
          <details open>
            <summary>增量复查</summary>
            <div className="lake-fields">
              <label>
                已有作品
                <select
                  value={d.refreshMode}
                  onChange={(e) => change({ refreshMode: e.target.value })}
                >
                  <option value="missing_or_stale">补缺并复查过期快照</option>
                  <option value="all">重新获取全部详情与清单</option>
                </select>
              </label>
              {d.refreshMode === "missing_or_stale" && (
                <label>
                  快照有效期（小时）
                  <input
                    type="number"
                    min={1}
                    max={8760}
                    value={d.refreshHours}
                    onChange={(e) =>
                      change({ refreshHours: Number(e.target.value) })
                    }
                  />
                </label>
              )}
              <label>
                地址相同的已有文件
                <select
                  value={d.reuseMode}
                  onChange={(e) => change({ reuseMode: e.target.value })}
                >
                  <option value="historical_if_same_locator">
                    允许使用有效期内的历史原件
                  </option>
                  <option value="revalidate">重新下载校验</option>
                </select>
              </label>
              {d.reuseMode === "historical_if_same_locator" && (
                <label>
                  历史原件有效期（小时）
                  <input
                    type="number"
                    min={1}
                    max={8760}
                    value={d.reuseHours}
                    onChange={(e) =>
                      change({ reuseHours: Number(e.target.value) })
                    }
                  />
                </label>
              )}
              <p className="lake-hint">
                每轮重新读取作者目录。近期完整快照保留原观察时间；缺文件、缺保存配方或访问条件变化时重新获取。
              </p>
            </div>
          </details>
          <details>
            <summary>本轮预算与周期</summary>
            <div className="lake-fields">
              <label>
                最多 API 请求
                <input
                  type="number"
                  min={1}
                  max={1000000}
                  value={d.apiRequests}
                  onChange={(e) =>
                    change({ apiRequests: Number(e.target.value) })
                  }
                />
              </label>
              <label>
                最多接纳作者
                <input
                  type="number"
                  min={1}
                  max={1000000}
                  value={d.authors}
                  onChange={(e) => change({ authors: Number(e.target.value) })}
                />
              </label>
              <label>
                最多下载（GiB）
                <input
                  type="number"
                  min={0.001}
                  max={102400}
                  step="any"
                  value={d.downloadGiB}
                  onChange={(e) =>
                    change({ downloadGiB: Number(e.target.value) })
                  }
                />
              </label>
              <label>
                最长运行（小时）
                <input
                  type="number"
                  min={1 / 60}
                  max={168}
                  step="any"
                  value={d.wallHours}
                  onChange={(e) =>
                    change({ wallHours: Number(e.target.value) })
                  }
                />
              </label>
              <label className="lake-check">
                <input
                  type="checkbox"
                  checked={d.periodic}
                  onChange={(e) => change({ periodic: e.target.checked })}
                />
                按固定周期复查
              </label>
              {d.periodic && (
                <label>
                  复查间隔（小时）
                  <input
                    type="number"
                    min={1 / 60}
                    max={8784}
                    step="any"
                    value={d.intervalHours}
                    onChange={(e) =>
                      change({ intervalHours: Number(e.target.value) })
                    }
                  />
                </label>
              )}
              <p className="lake-hint">
                预算用完时保留进度，点“继续下一轮”接续。周期计划依赖 Studio
                引擎运行，上一轮未完成时等待。
              </p>
            </div>
          </details>
        </fieldset>
        {(preview || d.submission) && (
          <section className="lake-summary" aria-label="Pixiv 任务摘要">
            <strong>
              {collectionRange(d.submission?.spec ?? preview!.definition)}
            </strong>
            <ImagePolicySummary
              policy={
                (d.submission?.spec ?? preview!.definition).media.image_policy
              }
            />
            {preview?.issues.map((i) => (
              <p key={i.code}>{i.message}</p>
            ))}
          </section>
        )}
        {accounts.error && <ErrorDetails error={accounts.error} />}
        {error != null && <ErrorDetails error={error} />}
        {notice && <p role="status">{notice}</p>}
        <footer className="lake-actions">
          <Button
            disabled={
              pending || frozen || !draft.editable || accounts.isPending
            }
            onClick={() => void inspect()}
          >
            检查任务摘要
          </Button>
          <Button
            className="primary"
            disabled={pending || (!preview && !frozen) || !!d.submission?.id}
            onClick={() => void submit()}
          >
            {d.periodic ? "创建并启用计划" : "开始采集"}
          </Button>
          {frozen && (
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
