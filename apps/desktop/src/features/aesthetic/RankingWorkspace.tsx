import { RankingValidity } from "./RankingValidity.js";
import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ChevronLeft,
  ChevronRight,
  ListOrdered,
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
  WorkbenchPanelPortal,
} from "@studio/ui";
import type { ModuleContext, WorkbenchLayout } from "@studio/ui";
import type { Schema } from "@studio/contracts";
import { FitDialog, DeriveDialog } from "./AnalysisActions.js";
import { analysisActive, analysisState } from "./analysisPresentation.js";

import {
  rankingBrowserInitial as initial,
  decodeRankingBrowser as decode,
} from "./rankingBrowserState.js";
import { RankingCanvas } from "./RankingCanvas.js";
import { RankingDetails, RankingEvidence } from "./RankingPanels.js";
import { RankingReviewPanel } from "./RankingReviewPanel.js";

const initialLayout: WorkbenchLayout = {
  panels: {
    snapshots: "left",
    inspector: "right",
    evidence: "right",
    review: "right",
  },
  active: {},
  leftWidth: 220,
  rightWidth: 380,
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
  onEvaluation: (stageId?: string) => void;
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
  const [dialog, setDialog] = useState<"fit" | "derive" | "prompt" | null>(
    null,
  );
  const [activeJobId, setActiveJobId] = useState("");
  const [notice, setNotice] = useState("");
  const [actionError, setActionError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const [reviewBusy, setReviewBusy] = useState(false);
  const pendingPage = useRef<{ after: string; pick: "first" | "last" } | null>(
    null,
  );
  const saved = draft.value;
  const effectiveLayout: WorkbenchLayout = {
    ...layout.value,
    panels: { ...initialLayout.panels, ...layout.value.panels },
  };
  function openPanel(id: string) {
    layout.update((old) => {
      const position =
        old.panels[id] && old.panels[id] !== "hidden"
          ? old.panels[id]
          : "right";
      return {
        ...old,
        panels: { ...initialLayout.panels, ...old.panels, [id]: position },
        active: { ...old.active, [position]: id },
      };
    });
  }
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
    staleTime: Infinity,
    refetchOnWindowFocus: false,
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
    enabled:
      !!snapshot.data &&
      !rows.isPending &&
      selectedOrdinal !== null &&
      !rows.data?.items.some((row) => row.ordinal === selectedOrdinal),
  });
  const selected =
    rows.data?.items.find((row) => row.ordinal === selectedOrdinal) ??
    candidate.data;
  const items = rows.data?.items ?? [];
  const selectedIndex = items.findIndex(
    (row) => row.ordinal === selectedOrdinal,
  );
  const previous = selectedIndex > 0 || saved.past.length > 0;
  const next =
    (selectedIndex >= 0 && selectedIndex < items.length - 1) ||
    !!rows.data?.next_cursor;
  useEffect(() => {
    const pending = pendingPage.current;
    if (
      !pending ||
      rows.isPending ||
      !rows.data ||
      saved.after !== pending.after
    )
      return;
    pendingPage.current = null;
    const row =
      pending.pick === "last" ? rows.data.items.at(-1) : rows.data.items[0];
    if (row) draft.controller.set((old) => ({ ...old, ordinal: row.ordinal }));
  }, [saved.after, rows.data, rows.isPending, draft.controller]);
  function turnPage(
    direction: number,
    pick: "first" | "last" = "first",
    allowReview = false,
  ) {
    if (
      !draft.editable ||
      rows.isFetching ||
      pendingPage.current ||
      (reviewBusy && !allowReview)
    )
      return;
    if (
      (direction > 0 && !rows.data?.next_cursor) ||
      (direction < 0 && !saved.past.length)
    )
      return;
    const after = direction > 0 ? rows.data!.next_cursor! : saved.past.at(-1)!;
    pendingPage.current = { after, pick };
    draft.controller.set((old) => ({
      ...old,
      after,
      past:
        direction > 0
          ? [...old.past, old.after].slice(-64)
          : old.past.slice(0, -1),
      page: old.page + (direction > 0 ? 1 : -1),
      ordinal: null,
      scrollTop: 0,
    }));
  }
  function navigate(delta: number, allowReview = false) {
    if (
      !draft.editable ||
      rows.isFetching ||
      pendingPage.current ||
      (reviewBusy && !allowReview)
    )
      return;
    const index = selectedIndex + delta;
    if (index < 0) turnPage(-1, "last", allowReview);
    else if (index >= items.length) turnPage(1, "first", allowReview);
    else if (items[index])
      draft.controller.set((old) => ({
        ...old,
        ordinal: items[index]!.ordinal,
      }));
  }
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
    if (reviewBusy) return;
    pendingPage.current = null;
    draft.controller.set((value) => ({
      ...value,
      snapshotId: id,
      after: "",
      past: [],
      page: 1,
      ordinal: null,
      image: false,
      scrollTop: 0,
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
        scrollTop: 0,
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
              disabled={!draft.editable || reviewBusy}
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
        onClick={() => onEvaluation()}
      >
        评审阶段与批次证据
      </button>
    </div>
  );
  const inspector = (
    <RankingDetails
      context={context}
      snapshotId={saved.snapshotId}
      row={selected}
      thumbnailSize={saved.thumbnailSize}
      pageSize={saved.pageSize}
      disabled={!draft.editable || reviewBusy || rows.isFetching}
      onThumbnailSize={(thumbnailSize) =>
        draft.controller.set((old) => ({ ...old, thumbnailSize }))
      }
      onPageSize={(pageSize) => {
        pendingPage.current = null;
        draft.controller.set((old) => ({
          ...old,
          pageSize,
          after: "",
          past: [],
          page: 1,
          ordinal: null,
          image: false,
          scrollTop: 0,
        }));
      }}
      onImage={() => draft.controller.set((old) => ({ ...old, image: true }))}
      onEvidence={() => openPanel("evidence")}
      onReview={() => openPanel("review")}
    />
  );
  const evidence = (
    <RankingEvidence
      row={selected}
      snapshot={snapshot.data}
      stage={stage.data}
      error={stage.error}
      onPrompt={() => setDialog("prompt")}
      onEvaluation={() => onEvaluation(stageId)}
    />
  );
  const job = activeJob.data;
  function updateRating(rating: string) {
    if (reviewBusy) return;
    pendingPage.current = null;
    draft.controller.set((value) => ({
      ...value,
      rating,
      after: "",
      past: [],
      page: 1,
      ordinal: null,
      image: false,
      scrollTop: 0,
    }));
  }
  return (
    <>
      <Workbench
        title={protectedOnly ? "保护复核工作台" : "排名浏览工作台"}
        layout={effectiveLayout}
        onLayout={layout.update}
        disabled={!layout.editable}
        panels={[
          { id: "snapshots", title: "评审成果", content: outline },
          {
            id: "inspector",
            title: "详情",
            content: inspector,
            defaultPosition: "right",
          },
          {
            id: "evidence",
            title: "评审依据",
            content: evidence,
            defaultPosition: "right",
          },
          {
            id: "review",
            title: "保护复核",
            portal: true,
            defaultPosition: "right",
          },
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
              disabled={reviewBusy}
              onClick={() => {
                void jobs.refetch();
                void cache.invalidateQueries({
                  queryKey: [
                    "project",
                    projectId,
                    "aesthetic",
                    "candidate-reviews",
                  ],
                });
                if (saved.snapshotId && !reviewBusy) {
                  pendingPage.current = null;
                  draft.controller.set((old) => ({
                    ...old,
                    after: "",
                    past: [],
                    page: 1,
                    ordinal: null,
                    image: false,
                    scrollTop: 0,
                  }));
                  void cache.invalidateQueries({
                    queryKey: [
                      "project",
                      projectId,
                      "aesthetic",
                      "ranking-rows",
                    ],
                  });
                }
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
              disabled={!draft.editable || reviewBusy}
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
          <button type="button" onClick={() => openPanel("inspector")}>
            显示与详情
          </button>
        </div>
        {summary && <RankingValidity summary={summary} />}
        {summary && !summary.converged && (
          <p className="ranking-validity" role="status">
            本次拟合尚未收敛，结果用于检查与实验对照。
          </p>
        )}
        {snapshot.data ? (
          <RankingCanvas
            context={context}
            items={items}
            selected={selected}
            image={saved.image}
            thumbnailSize={saved.thumbnailSize}
            scrollTop={saved.scrollTop}
            pageKey={`${saved.snapshotId}:${saved.rating}:${saved.after}:${saved.pageSize}`}
            loading={rows.isPending}
            disabled={!draft.editable || reviewBusy || rows.isFetching}
            previous={previous}
            next={next}
            onNavigate={navigate}
            onSelect={(ordinal) =>
              draft.controller.set((old) => ({ ...old, ordinal }))
            }
            onImage={(image) =>
              draft.controller.set((old) => ({ ...old, image }))
            }
            onScroll={(scrollTop) =>
              draft.controller.set((old) =>
                old.snapshotId === saved.snapshotId &&
                old.rating === saved.rating &&
                old.after === saved.after &&
                old.pageSize === saved.pageSize
                  ? { ...old, scrollTop }
                  : old,
              )
            }
          />
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
              <Button onClick={() => onEvaluation()}>查看评审阶段</Button>
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
            disabled={!saved.past.length || rows.isFetching || reviewBusy}
            onClick={() => turnPage(-1)}
          >
            <ChevronLeft size={15} />
            上一页
          </button>
          <button
            type="button"
            aria-label="排名下一页"
            disabled={!rows.data?.next_cursor || rows.isFetching || reviewBusy}
            onClick={() => turnPage(1)}
          >
            下一页
            <ChevronRight size={15} />
          </button>
        </div>
        <WorkbenchPanelPortal id="review">
          {selected ? (
            <RankingReviewPanel
              key={`${saved.snapshotId}:${selected.ordinal}`}
              context={context}
              snapshotId={saved.snapshotId}
              row={selected}
              next={next}
              onBusy={setReviewBusy}
              onSaved={(advance) => {
                const current = draft.controller.getSnapshot().value;
                if (
                  current.snapshotId !== saved.snapshotId ||
                  current.rating !== saved.rating ||
                  current.after !== saved.after ||
                  (current.ordinal ?? items[0]?.ordinal) !== selected.ordinal
                )
                  return;
                setNotice(`候选 ${selected.ordinal + 1} 的复核已保存。`);
                if (advance) navigate(1, true);
              }}
            />
          ) : (
            <p className="aesthetic-help">选择图片后开始复核。</p>
          )}
          {protectedOnly && (
            <p className="aesthetic-help">
              列表按进入时的复核水位保留。保存后可继续当前队列；点击刷新更新保护池。
            </p>
          )}
        </WorkbenchPanelPortal>
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
