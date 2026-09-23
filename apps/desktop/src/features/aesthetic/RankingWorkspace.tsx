import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ChevronLeft,
  ChevronRight,
  Image,
  LayoutGrid,
  ListOrdered,
  Shield,
  RefreshCw,
  FolderPlus,
} from "lucide-react";
import {
  Button,
  ErrorDetails,
  DraftStatus,
  Workbench,
  WorkbenchPreferences,
  useWorkbenchLayout,
  useDraft,
  WorkbenchDialog,
} from "@studio/ui";
import type { ModuleContext, WorkbenchLayout } from "@studio/ui";
import type { Schema } from "@studio/contracts";
import { AssetImage } from "../browser/AssetImage.js";
import { FitDialog, DeriveDialog, ReviewDialog } from "./AnalysisActions.js";
import {
  analysisActive,
  analysisState,
  rankingAsset,
  rankingLabel,
} from "./analysisPresentation.js";

const initial = {
  snapshotId: "",
  rating: "g",
  after: "",
  past: [] as string[],
  page: 1,
  pageSize: 48,
  ordinal: null as number | null,
  image: false,
};
function decode(value: unknown): typeof initial | null {
  if (!value || typeof value !== "object") return null;
  const v = value as typeof initial;
  if (
    typeof v.snapshotId !== "string" ||
    !["g", "s", "q", "e"].includes(v.rating) ||
    typeof v.after !== "string" ||
    v.after.length > 16384 ||
    !Array.isArray(v.past) ||
    v.past.length > 64 ||
    v.past.some((x) => typeof x !== "string" || x.length > 16384) ||
    !Number.isSafeInteger(v.page) ||
    v.page < 1 ||
    ![12, 48, 96].includes(v.pageSize) ||
    (v.ordinal !== null &&
      (!Number.isSafeInteger(v.ordinal) || v.ordinal < 0)) ||
    typeof v.image !== "boolean"
  )
    return null;
  return v;
}
const initialLayout: WorkbenchLayout = {
  panels: { snapshots: "left", inspector: "right" },
  active: {},
  leftWidth: 220,
  rightWidth: 285,
  bottomHeight: 260,
};
type Job = Schema["AestheticAnalysisJob"];
export function RankingWorkspace({
  context,
  protectedOnly,
  onEvaluation,
  toolbarStart,
}: {
  context: ModuleContext;
  protectedOnly: boolean;
  onEvaluation: () => void;
  toolbarStart: ReactNode;
}) {
  const { client, projectId } = context;
  const cache = useQueryClient();
  const draft = useDraft(
    client,
    projectId,
    "core.aesthetic",
    initial,
    decode,
    protectedOnly ? "protected-browser" : "ranking-browser",
  );
  const layout = useWorkbenchLayout(client, "aesthetic-ranking", initialLayout);
  const [jobsAfter, setJobsAfter] = useState<string>();
  const [dialog, setDialog] = useState<
    "fit" | "derive" | "review" | "prompt" | null
  >(null);
  const [activeJobId, setActiveJobId] = useState("");
  const [notice, setNotice] = useState("");
  const [actionError, setActionError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const saved = draft.value;
  const jobs = useQuery({
    queryKey: ["project", projectId, "aesthetic", "analysis-jobs", jobsAfter],
    queryFn: ({ signal }) =>
      client.aesthetic.analysis.jobs(
        projectId,
        { ...(jobsAfter ? { after: jobsAfter } : {}), limit: 32 },
        signal,
      ),
    refetchInterval: (query) =>
      query.state.data?.items.some((job) => analysisActive(job.state))
        ? 2000
        : 5000,
  });
  const snapshots =
    jobs.data?.items.filter(
      (job) => job.state === "completed" && job.result?.kind === "fit",
    ) ?? [];
  const firstId = snapshots[0]?.id;
  useEffect(() => {
    if (draft.editable && !saved.snapshotId && firstId)
      draft.controller.set((value) => ({ ...value, snapshotId: firstId }));
  }, [draft.editable, draft.controller, saved.snapshotId, firstId]);
  const snapshot = useQuery({
    queryKey: ["project", projectId, "aesthetic", "snapshot", saved.snapshotId],
    queryFn: ({ signal }) =>
      client.aesthetic.analysis.snapshot(projectId, saved.snapshotId, signal),
    enabled: draft.editable && !!saved.snapshotId,
  });
  const rows = useQuery({
    queryKey: [
      "project",
      projectId,
      "aesthetic",
      "ranking-rows",
      saved.snapshotId,
      saved.rating,
      saved.after,
      saved.pageSize,
      protectedOnly,
    ],
    queryFn: async ({ signal }) => {
      if (protectedOnly) {
        const page = await client.aesthetic.analysis.select(
          projectId,
          saved.snapshotId,
          {
            filter: { ratings: [saved.rating], protected_only: true },
            after: saved.after || null,
            limit: saved.pageSize,
          },
          signal,
        );
        return {
          items: page.items.map((item) => item.ranking),
          next_cursor: page.next_cursor,
          watermark: page.review_watermark,
        };
      }
      return {
        ...(await client.aesthetic.analysis.rows(
          projectId,
          saved.snapshotId,
          {
            ...(saved.after ? { after: saved.after } : {}),
            rating: saved.rating,
            limit: saved.pageSize,
          },
          signal,
        )),
        watermark: null,
      };
    },
    enabled: draft.editable && !!snapshot.data,
  });
  const selectedOrdinal = saved.ordinal ?? rows.data?.items[0]?.ordinal ?? null;
  const candidate = useQuery({
    queryKey: [
      "project",
      projectId,
      "aesthetic",
      "ranking-candidate",
      saved.snapshotId,
      selectedOrdinal,
    ],
    queryFn: ({ signal }) =>
      client.aesthetic.analysis.candidate(
        projectId,
        saved.snapshotId,
        selectedOrdinal!,
        signal,
      ),
    enabled: !!snapshot.data && selectedOrdinal !== null,
  });
  const selected =
    rows.data?.items.find((row) => row.ordinal === selectedOrdinal) ??
    candidate.data;
  const stageId = snapshot.data?.input.stage_id;
  const stage = useQuery({
    queryKey: ["project", projectId, "aesthetic", "snapshot-stage", stageId],
    queryFn: ({ signal }) =>
      client.aesthetic.stage(projectId, stageId!, signal),
    enabled: !!stageId,
  });
  const activeJob = useQuery({
    queryKey: [
      "project",
      projectId,
      "aesthetic",
      "active-analysis",
      activeJobId,
    ],
    queryFn: ({ signal }) =>
      client.aesthetic.analysis.job(projectId, activeJobId, signal),
    enabled: !!activeJobId,
    refetchInterval: (query) =>
      !query.state.data || analysisActive(query.state.data.state)
        ? 1500
        : false,
  });
  function selectSnapshot(id: string) {
    draft.controller.set((value) => ({
      ...value,
      snapshotId: id,
      after: "",
      past: [],
      page: 1,
      ordinal: null,
      image: false,
    }));
  }
  useEffect(() => {
    const job = activeJob.data;
    if (job?.state !== "completed") return;
    if (job.result?.kind === "fit") {
      draft.controller.set((value) => ({
        ...value,
        snapshotId: job.id,
        after: "",
        past: [],
        page: 1,
        ordinal: null,
        image: false,
      }));
      setNotice("排名快照已发布，可按 Rating 浏览。");
    }
    if (job.result?.kind === "derive") {
      void cache.invalidateQueries({
        queryKey: ["project", projectId, "collections"],
      });
      setNotice(`工作集已发布 · ${job.result.count.toLocaleString()} 项`);
    }
    void cache.invalidateQueries({
      queryKey: ["project", projectId, "aesthetic", "analysis-jobs"],
    });
  }, [activeJob.data, cache, projectId, draft.controller]);
  function created(job: Job) {
    setActiveJobId(job.id);
    void jobs.refetch();
    setNotice("离线任务已创建，关闭页面后仍可继续。");
  }
  async function control(action: "cancel" | "resume") {
    setBusy(true);
    setActionError(null);
    try {
      await client.aesthetic.analysis.control(projectId, activeJobId, action);
      await activeJob.refetch();
      await jobs.refetch();
    } catch (error) {
      setActionError(error);
    } finally {
      setBusy(false);
    }
  }
  const summary =
    snapshot.data?.result?.kind === "fit" ? snapshot.data.result : null;
  const group = summary?.groups.find((value) => value.rating === saved.rating);
  const errors = [
    draft.error,
    jobs.error,
    snapshot.error,
    rows.error,
    candidate.error,
    activeJob.error,
    actionError,
  ].filter(Boolean);
  const outline = (
    <div className="ranking-outline">
      <details className="wb-fold" open>
        <summary>
          排名快照 <small>本页 {snapshots.length}</small>
        </summary>
        <div className="ranking-snapshot-list">
          {snapshots.map((job) => (
            <button
              type="button"
              key={job.id}
              aria-pressed={job.id === saved.snapshotId}
              disabled={!draft.editable}
              onClick={() => selectSnapshot(job.id)}
            >
              <ListOrdered size={15} />
              <span>
                {job.request.name}
                <small>
                  已发布 · {job.input.candidates.toLocaleString()} 张候选
                </small>
              </span>
            </button>
          ))}
        </div>
        {!snapshots.length && (
          <p className="aesthetic-help">本页没有已发布快照。</p>
        )}
        <div className="aesthetic-actions">
          <button
            type="button"
            disabled={!jobsAfter}
            onClick={() => setJobsAfter(undefined)}
          >
            首页
          </button>
          <button
            type="button"
            disabled={!jobs.data?.next_cursor}
            onClick={() => setJobsAfter(jobs.data?.next_cursor ?? undefined)}
          >
            下一页任务
          </button>
        </div>
      </details>
      <details className="wb-fold" open>
        <summary>
          离线任务 <small>本页 {jobs.data?.items.length ?? 0}</small>
        </summary>
        <div className="ranking-job-list">
          {jobs.data?.items.map((job) => (
            <button
              type="button"
              key={job.id}
              aria-pressed={job.id === activeJobId}
              onClick={() => setActiveJobId(job.id)}
            >
              <span>
                {job.request.name}
                <small>
                  {analysisState(job.state)} · {job.progress.toLocaleString()} /{" "}
                  {job.total.toLocaleString()}
                </small>
              </span>
            </button>
          ))}
        </div>
      </details>
      <button
        type="button"
        className="ranking-outline-link"
        onClick={onEvaluation}
      >
        评审阶段与批次证据
      </button>
    </div>
  );
  const inspector = selected ? (
    <div className="ranking-inspector">
      <div className="ranking-inspector-preview">
        <AssetImage
          client={client}
          projectId={projectId}
          asset={rankingAsset(selected)}
          edge={480}
        />
      </div>
      <h3>候选 {selected.ordinal + 1}</h3>
      <details className="wb-fold" open>
        <summary>排名与依据</summary>
        <div className="wb-fold-body">
          <dl className="wb-property-list">
            <dt>名次</dt>
            <dd>{rankingLabel(selected)}</dd>
            <dt>可比范围</dt>
            <dd>
              {selected.rating_rank_min != null
                ? `${selected.rating.toUpperCase()} 类`
                : selected.component != null
                  ? `分量 ${selected.component} · ${selected.component_size} 图`
                  : "暂无可比范围"}
            </dd>
            <dt>有效曝光</dt>
            <dd>{selected.exposures} 次</dd>
            <dt>分半波动</dt>
            <dd>
              {selected.split_percentile_delta == null
                ? "未知 / 证据不足"
                : (selected.split_percentile_delta * 100).toFixed(2) +
                  " 个百分点"}
            </dd>
            <dt>快照内提名</dt>
            <dd>{selected.protected ? "已提名" : "未提名"}</dd>
            <dt>需要复核</dt>
            <dd>{selected.needs_review ? "是" : "未标记"}</dd>
          </dl>
        </div>
      </details>
      <details className="wb-fold">
        <summary>
          评审配置 <small>冻结副本</small>
        </summary>
        <div className="wb-fold-body">
          <dl className="wb-property-list">
            <dt>评审阶段</dt>
            <dd>{stage.data?.name ?? "读取中…"}</dd>
            <dt>模型</dt>
            <dd>{stage.data?.config.model.remote_model_id ?? "—"}</dd>
            <dt>标准版本</dt>
            <dd>{stage.data?.config.model.system_prompt_revision ?? "—"}</dd>
            <dt>证据水位</dt>
            <dd>{snapshot.data?.input.evidence_watermark ?? "—"}</dd>
          </dl>
          <button
            type="button"
            disabled={!stage.data}
            onClick={() => setDialog("prompt")}
          >
            查看冻结 System Prompt
          </button>
          {stage.error && <ErrorDetails error={stage.error} />}
        </div>
      </details>
      <details className="wb-fold">
        <summary>来源与诊断</summary>
        <div className="wb-fold-body">
          <dl className="wb-property-list">
            <dt>年份</dt>
            <dd>{selected.year ?? "未知"}</dd>
            <dt>不可评判</dt>
            <dd>{selected.unjudgeable} 次</dd>
            <dt>分数</dt>
            <dd>{selected.score?.toFixed(5) ?? "未知"}</dd>
            <dt>跨年曝光</dt>
            <dd>{selected.cross_year_exposures}</dd>
          </dl>
          <code className="ranking-asset-id">{selected.key.asset_id}</code>
        </div>
      </details>
      <div className="ranking-inspector-actions">
        <Button
          onClick={() =>
            draft.controller.set((value) => ({
              ...value,
              ordinal: selected.ordinal,
              image: true,
            }))
          }
        >
          查看大图
        </Button>
        <Button onClick={() => setDialog("review")}>复核保护状态</Button>
      </div>
      <p className="aesthetic-help">
        提名和复核不改变统计分数；保护候选不会自动成为最终精选。
      </p>
    </div>
  ) : (
    <div className="wb-empty">
      <Image size={25} />
      <p>选择图片，查看名次与评审依据。</p>
    </div>
  );
  const job = activeJob.data;
  function updateRating(rating: string) {
    draft.controller.set((value) => ({
      ...value,
      rating,
      after: "",
      past: [],
      page: 1,
      ordinal: null,
      image: false,
    }));
  }
  return (
    <>
      <Workbench
        title={protectedOnly ? "保护复核工作台" : "排名浏览工作台"}
        layout={layout.value}
        onLayout={layout.update}
        disabled={!layout.editable}
        panels={[
          { id: "snapshots", title: "评审成果", content: outline },
          { id: "inspector", title: "详情", content: inspector },
        ]}
        toolbar={
          <>
            {toolbarStart}
            <Button onClick={() => setDialog("fit")}>
              <ListOrdered size={19} className="wb-analysis-icon" />
              生成排名快照
            </Button>
            <Button
              disabled={!snapshot.data}
              onClick={() => setDialog("derive")}
            >
              <FolderPlus size={19} className="wb-folder-icon" />
              生成工作集
            </Button>
            <span className="grow" />
            <button
              type="button"
              className="icon-button"
              aria-label="刷新排名工作台"
              onClick={() => {
                void jobs.refetch();
                if (saved.snapshotId) void rows.refetch();
              }}
            >
              <RefreshCw size={14} />
            </button>
            <WorkbenchPreferences state={layout} />
          </>
        }
        status={
          <>
            <span>{snapshot.data?.request.name ?? "尚未选择快照"}</span>
            <span>
              {group?.fully_connected
                ? "当前 Rating 比较关系已连通"
                : group
                  ? `${group.components} 个分量 · 分量内名次`
                  : "选择一份已发布快照"}
            </span>
            <DraftStatus controller={draft.controller} quiet />
          </>
        }
      >
        {errors.length > 0 && <ErrorDetails error={errors[0]} />}
        {notice && (
          <div className="aesthetic-notice" role="status">
            {notice}
            <button
              type="button"
              aria-label="关闭排名提示"
              onClick={() => setNotice("")}
            >
              关闭
            </button>
          </div>
        )}
        {job && (
          <div className="ranking-task-state" role="status">
            <strong>{job.request.name}</strong>
            <span>
              {analysisState(job.state)} · {job.progress.toLocaleString()} /{" "}
              {job.total.toLocaleString()}
            </span>
            {job.error && <ErrorDetails error={job.error} />}
            <span className="grow" />
            {analysisActive(job.state) && (
              <button
                type="button"
                disabled={busy}
                onClick={() => void control("cancel")}
              >
                取消离线任务
              </button>
            )}
            {["interrupted", "failed", "cancelled"].includes(job.state) && (
              <button
                type="button"
                disabled={busy}
                onClick={() => void control("resume")}
              >
                恢复离线任务
              </button>
            )}
            {job.result?.kind === "fit" && (
              <button type="button" onClick={() => selectSnapshot(job.id)}>
                打开快照
              </button>
            )}
            {job.result?.kind === "derive" && (
              <button
                type="button"
                onClick={() => {
                  if (job.result?.kind === "derive")
                    context.browser.onScope({
                      kind: "collection",
                      id: job.result.collection_id,
                      name: job.request.name,
                    });
                }}
              >
                打开工作集
              </button>
            )}
            <button
              type="button"
              aria-label="收起离线任务"
              onClick={() => setActiveJobId("")}
            >
              收起
            </button>
          </div>
        )}
        <div className="ranking-view-tools">
          <label>
            Rating
            <select
              aria-label="排名 Rating"
              value={saved.rating}
              disabled={!draft.editable}
              onChange={(e) => updateRating(e.target.value)}
            >
              {["g", "s", "q", "e"].map((rating) => (
                <option key={rating} value={rating}>
                  {rating.toUpperCase()}
                </option>
              ))}
            </select>
          </label>
          <span>
            {protectedOnly ? "有效保护候选" : "排名图片"}
            {group && ` · ${group.candidates.toLocaleString()} 张候选`}
          </span>
          <span className="grow" />
          <button
            type="button"
            className="icon-button"
            aria-label="返回排名网格"
            aria-pressed={!saved.image}
            onClick={() =>
              draft.controller.set((value) => ({ ...value, image: false }))
            }
          >
            <LayoutGrid size={15} />
          </button>
          <label>
            每页
            <select
              aria-label="排名每页图片数"
              value={saved.pageSize}
              onChange={(e) =>
                draft.controller.set((value) => ({
                  ...value,
                  pageSize: Number(e.target.value),
                  after: "",
                  past: [],
                  page: 1,
                  ordinal: null,
                  image: false,
                }))
              }
            >
              {[12, 48, 96].map((size) => (
                <option key={size} value={size}>
                  {size}
                </option>
              ))}
            </select>
          </label>
        </div>
        {summary && !summary.converged && (
          <p className="ranking-validity" role="status">
            本次拟合尚未收敛，结果用于检查与实验对照。
          </p>
        )}
        {snapshot.data ? (
          rows.isPending ? (
            <div className="wb-empty">正在读取图片页…</div>
          ) : saved.image && selected ? (
            <div
              className="ranking-full-image"
              onKeyDown={(e) => {
                if (e.key === "Escape")
                  draft.controller.set((value) => ({ ...value, image: false }));
              }}
            >
              <AssetImage
                client={client}
                projectId={projectId}
                asset={rankingAsset(selected)}
                edge={1440}
              />
              <Button
                onClick={() =>
                  draft.controller.set((value) => ({ ...value, image: false }))
                }
              >
                返回排名网格
              </Button>
            </div>
          ) : (
            <div className="ranking-image-scroll">
              <div className="ranking-image-grid">
                {rows.data?.items.map((row) => (
                  <button
                    type="button"
                    className="ranking-image-tile"
                    key={row.ordinal}
                    aria-label={
                      "候选 " + (row.ordinal + 1) + "，" + rankingLabel(row)
                    }
                    aria-pressed={row.ordinal === selectedOrdinal}
                    onClick={() =>
                      draft.controller.set((value) => ({
                        ...value,
                        ordinal: row.ordinal,
                      }))
                    }
                    onDoubleClick={() =>
                      draft.controller.set((value) => ({
                        ...value,
                        ordinal: row.ordinal,
                        image: true,
                      }))
                    }
                  >
                    <AssetImage
                      client={client}
                      projectId={projectId}
                      asset={rankingAsset(row)}
                      edge={480}
                    />
                    <span className="ranking-image-caption">
                      <strong>{rankingLabel(row)}</strong>
                      {row.protected && (
                        <Shield size={13} aria-label="快照内顶级提名" />
                      )}
                      <small>
                        候选 {row.ordinal + 1} · {row.exposures} 次曝光
                      </small>
                    </span>
                  </button>
                ))}
              </div>
              {!rows.data?.items.length && !rows.error && (
                <div className="wb-empty">
                  <p>
                    {rows.data?.next_cursor
                      ? "本页扫描尚未找到匹配项，可继续下一页。"
                      : protectedOnly
                        ? "当前范围没有有效保护候选。"
                        : "当前 Rating 暂无排名图片。"}
                  </p>
                </div>
              )}
            </div>
          )
        ) : (
          <div className="wb-empty">
            <ListOrdered size={32} />
            <h3>
              {snapshot.isFetching ? "正在恢复排名快照…" : "从一份排名快照开始"}
            </h3>
            <p>
              选择已发布的快照，或使用已有评审证据生成排名。首次评审可从“新建评审”开始。
            </p>
            <div className="aesthetic-actions">
              <Button onClick={() => setDialog("fit")}>生成排名快照</Button>
              <Button onClick={onEvaluation}>查看评审阶段</Button>
            </div>
          </div>
        )}
        <div className="ranking-pagebar">
          <span>
            第 {saved.page} 页 · 本页 {rows.data?.items.length ?? 0} 项
            {rows.data?.watermark != null &&
              ` · 复核水位 ${rows.data.watermark}`}
          </span>
          <span className="grow" />
          <button
            type="button"
            aria-label="排名上一页"
            disabled={!saved.past.length || rows.isFetching}
            onClick={() =>
              draft.controller.set((value) => ({
                ...value,
                after: value.past[value.past.length - 1] ?? "",
                past: value.past.slice(0, -1),
                page: value.page - 1,
                ordinal: null,
                image: false,
              }))
            }
          >
            <ChevronLeft size={15} />
            上一页
          </button>
          <button
            type="button"
            aria-label="排名下一页"
            disabled={!rows.data?.next_cursor || rows.isFetching}
            onClick={() =>
              draft.controller.set((value) => ({
                ...value,
                after: rows.data?.next_cursor ?? "",
                past: [...value.past, value.after].slice(-64),
                page: value.page + 1,
                ordinal: null,
                image: false,
              }))
            }
          >
            下一页
            <ChevronRight size={15} />
          </button>
        </div>
      </Workbench>
      {dialog === "fit" && (
        <FitDialog
          context={context}
          onClose={() => setDialog(null)}
          onCreated={created}
        />
      )}
      {dialog === "derive" && snapshot.data && (
        <DeriveDialog
          context={context}
          snapshotId={saved.snapshotId}
          rating={saved.rating}
          onClose={() => setDialog(null)}
          onCreated={created}
        />
      )}
      {dialog === "review" && selected && (
        <ReviewDialog
          context={context}
          snapshotId={saved.snapshotId}
          ordinal={selected.ordinal}
          onClose={() => setDialog(null)}
          onSaved={() => {
            setNotice("复核决定已追加保存，历史快照名次保持不变。");
            draft.controller.set((value) => ({
              ...value,
              after: "",
              past: [],
              page: 1,
            }));
            void cache.invalidateQueries({
              queryKey: ["project", projectId, "aesthetic", "ranking-rows"],
            });
          }}
        />
      )}
      {dialog === "prompt" && (
        <WorkbenchDialog
          title="冻结的 System Prompt"
          onClose={() => setDialog(null)}
        >
          <p>
            {stage.data?.name} · 标准版本{" "}
            {stage.data?.config.model.system_prompt_revision ?? "未知"}
          </p>
          <pre className="ranking-prompt">
            {stage.data?.config.model.messages
              .filter(
                (message) =>
                  message.role === "system" || message.role === "developer",
              )
              .flatMap((message) =>
                message.content
                  .filter((block) => block.type === "text")
                  .map((block) => (block.type === "text" ? block.text : "")),
              )
              .join("\n\n") || "此阶段未提供可显示的系统消息。"}
          </pre>
        </WorkbenchDialog>
      )}
    </>
  );
}
