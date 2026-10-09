import { useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { Schema } from "@studio/contracts";
import type { ModuleContext } from "@studio/ui";
import { Button, ErrorDetails, WorkbenchDialog } from "@studio/ui";
import { AssetImage } from "../browser/AssetImage.js";
import {
  rankingAsset,
  rankingLabel,
  analysisActive,
  fitParameters,
} from "./analysisPresentation.js";
type Job = Schema["AestheticAnalysisJob"];

function useAction() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const lock = useRef(false);
  const submission = useRef<{ signature: string; key: string } | null>(null);
  function key(value: unknown) {
    const signature = JSON.stringify(value);
    if (submission.current?.signature !== signature)
      submission.current = { signature, key: crypto.randomUUID() };
    return submission.current.key;
  }
  async function run(action: () => Promise<void>, resetKey = true) {
    if (lock.current) return;
    lock.current = true;
    setBusy(true);
    setError(null);
    try {
      await action();
      if (resetKey) submission.current = null;
    } catch (e) {
      setError(e);
    } finally {
      lock.current = false;
      setBusy(false);
    }
  }
  return { busy, error, run, key };
}

export function FitDialog({
  context,
  onClose,
  onCreated,
  initialStageId,
}: {
  context: ModuleContext;
  onClose: () => void;
  onCreated: (job: Job) => void;
  initialStageId?: string | undefined;
}) {
  const action = useAction();
  const [after, setAfter] = useState<string>();
  const stages = useQuery({
    queryKey: ["project", context.projectId, "aesthetic", "fit-stages", after],
    queryFn: ({ signal }) =>
      context.client.aesthetic.stages(context.projectId, after, signal),
  });
  const [stageId, setStageId] = useState(initialStageId ?? "");
  const pinnedStage = useQuery({
    queryKey: [
      "project",
      context.projectId,
      "aesthetic",
      "fit-source",
      stageId,
    ],
    queryFn: ({ signal }) =>
      context.client.aesthetic.stage(context.projectId, stageId, signal),
    enabled: !!stageId,
  });
  const availableStages = [
    ...new Map(
      [
        ...(pinnedStage.data ? [pinnedStage.data] : []),
        ...(stages.data?.items ?? []),
      ].map((stage) => [stage.id, stage]),
    ).values(),
  ];
  const [estimator, setEstimator] = useState("davidson_v2");
  const [iterations, setIterations] = useState(128);
  const [regularization, setRegularization] = useState(0.001);
  const [tieStrength, setTieStrength] = useState(0.1);
  const [split, setSplit] = useState(true);
  // Until edited, the name follows the source stage and settings so several
  // snapshots of one stage stay distinguishable.
  const [customName, setCustomName] = useState<string | null>(null);
  const sourceStage = availableStages.find((stage) => stage.id === stageId);
  const name =
    customName ??
    (sourceStage
      ? `${sourceStage.name} · ${fitParameters({
          stage_id: stageId,
          estimator: {
            kind: estimator,
            iterations,
            regularization,
            tie_strength: tieStrength,
          },
        })}`.slice(0, 120)
      : "排名快照");
  return (
    <WorkbenchDialog
      title="生成排名快照"
      onClose={() => {
        if (!action.busy) onClose();
      }}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void action.run(async () => {
            const value = {
              name,
              spec: {
                kind: "fit" as const,
                config: {
                  stage_id: stageId,
                  estimator: {
                    kind: estimator,
                    iterations,
                    regularization,
                    tie_strength: tieStrength,
                  },
                  stability_seed: split ? 17 : null,
                },
                experiment_id: null,
                variant: null,
              },
            };
            const job = await context.client.aesthetic.analysis.create(
              context.projectId,
              { ...value, idempotency_key: action.key(value) },
            );
            onCreated(job);
            onClose();
          });
        }}
      >
        <div className="wb-field-list">
          <label>
            快照名称
            <input
              value={name}
              required
              maxLength={120}
              disabled={action.busy}
              onChange={(e) => setCustomName(e.target.value)}
            />
          </label>
          <label>
            来源评审阶段
            <select
              aria-label="来源评审阶段"
              value={stageId}
              required
              disabled={action.busy}
              onChange={(e) => setStageId(e.target.value)}
            >
              <option value="">选择已有接受证据的阶段</option>
              {availableStages.map((stage) => (
                <option
                  key={stage.id}
                  value={stage.id}
                  disabled={!stage.accepted}
                >
                  {stage.name}
                  {stage.archived && "（已归档）"} · {stage.accepted} 批有效
                </option>
              ))}
            </select>
          </label>
        </div>
        {(after || stages.data?.next_cursor) && (
          <div className="aesthetic-actions">
            <button
              type="button"
              disabled={action.busy || !after}
              onClick={() => setAfter(undefined)}
            >
              阶段首页
            </button>
            <button
              type="button"
              disabled={action.busy || !stages.data?.next_cursor}
              onClick={() => setAfter(stages.data?.next_cursor ?? undefined)}
            >
              更多阶段
            </button>
          </div>
        )}
        <details className="wb-fold">
          <summary>
            计算选项{" "}
            <small>{estimator === "borda_v1" ? "Borda" : "Davidson"}</small>
          </summary>
          <div className="wb-fold-body wb-field-list">
            <label>
              估计器
              <select
                value={estimator}
                disabled={action.busy}
                onChange={(e) => setEstimator(e.target.value)}
              >
                <option value="davidson_v2">Davidson v2 · 加速拟合</option>
                <option value="davidson_v1">Davidson v1 · 原求解器</option>
                <option value="borda_v1">Borda · 基线对照</option>
              </select>
            </label>
            <label>
              迭代上限
              <input
                type="number"
                value={iterations}
                min={1}
                max={128}
                disabled={action.busy}
                onChange={(e) => setIterations(Number(e.target.value))}
              />
            </label>
            <label>
              正则化
              <input
                type="number"
                value={regularization}
                min={0.001}
                max={10}
                step="any"
                disabled={action.busy}
                onChange={(e) => setRegularization(Number(e.target.value))}
              />
            </label>
            <label>
              并列强度
              <input
                type="number"
                value={tieStrength}
                min={0.01}
                max={100}
                step="any"
                disabled={action.busy}
                onChange={(e) => setTieStrength(Number(e.target.value))}
              />
            </label>
            <label>
              整批分半诊断
              <input
                type="checkbox"
                checked={split}
                disabled={action.busy}
                onChange={(e) => setSplit(e.target.checked)}
              />
            </label>
          </div>
        </details>
        <p>
          只使用已保存证据进行离线计算，不调用模型。计算固定当前证据水位；完成后发布独立快照。
        </p>
        <p className="aesthetic-help">
          评审运行中也可以预览当前证据。比较关系尚未连通时，快照只提供分量内名次。
        </p>
        {pinnedStage.data?.accepted === 0 && (
          <p className="aesthetic-notice">
            当前阶段尚无有效评审，保存至少一批有效结果后即可生成快照。
          </p>
        )}
        {(action.error || stages.error) && (
          <ErrorDetails error={action.error ?? stages.error} />
        )}
        <div className="wb-dialog-actions">
          <Button type="button" disabled={action.busy} onClick={onClose}>
            取消
          </Button>
          <Button
            type="submit"
            className="primary"
            disabled={action.busy || !stageId || !pinnedStage.data?.accepted}
          >
            {action.busy ? "正在提交…" : "开始离线计算"}
          </Button>
        </div>
      </form>
    </WorkbenchDialog>
  );
}

export function DeriveDialog({
  context,
  snapshotId,
  rating,
  onClose,
  onCreated,
}: {
  context: ModuleContext;
  snapshotId: string;
  rating: string;
  onClose: () => void;
  onCreated: (job: Job) => void;
}) {
  const action = useAction();
  const [name, setName] = useState(rating.toUpperCase() + " 类排名工作集");
  const [mode, setMode] = useState<"percent" | "count">("percent");
  const [percent, setPercent] = useState(25);
  const [topCount, setTopCount] = useState(1000);
  const [protect, setProtect] = useState(true);
  const [preview, setPreview] = useState<
    Schema["AestheticRankingSelection"] | null
  >(null);
  const [countJobId, setCountJobId] = useState("");
  const countJob = useQuery({
    queryKey: [
      "project",
      context.projectId,
      "aesthetic",
      "selection-count",
      countJobId,
    ],
    queryFn: ({ signal }) =>
      context.client.aesthetic.analysis.job(
        context.projectId,
        countJobId,
        signal,
      ),
    enabled: !!countJobId,
    refetchInterval: (query) =>
      !query.state.data || analysisActive(query.state.data.state)
        ? 1000
        : false,
  });
  const summary =
    countJob.data?.state === "completed" &&
    countJob.data.result?.kind === "preview"
      ? countJob.data.result
      : null;
  const filter = {
    ratings: [rating],
    ...(mode === "percent" ? { top_percent: percent } : { rank_to: topCount }),
    include_protected: protect,
  };
  const valid =
    mode === "percent"
      ? Number.isFinite(percent) && percent > 0 && percent <= 100
      : Number.isSafeInteger(topCount) && topCount > 0 && topCount <= 10000000;
  function invalidate() {
    setPreview(null);
    setCountJobId("");
    if (countJob.data && analysisActive(countJob.data.state))
      void context.client.aesthetic.analysis
        .control(context.projectId, countJob.data.id, "cancel")
        .catch(() => {});
  }
  async function previewSelection(after?: string | null) {
    await action.run(async () => {
      const page = await context.client.aesthetic.analysis.select(
        context.projectId,
        snapshotId,
        { filter, after: after ?? null, limit: 12 },
      );
      setPreview(page);
      if (!after) {
        const value = {
          name: "工作集筛选统计",
          spec: {
            kind: "preview" as const,
            snapshot_id: snapshotId,
            filter,
            review_watermark: page.review_watermark,
          },
        };
        const job = await context.client.aesthetic.analysis.create(
          context.projectId,
          { ...value, idempotency_key: action.key(value) },
        );
        setCountJobId(job.id);
      }
    }, false);
  }
  return (
    <WorkbenchDialog
      title="从排名生成工作集"
      onClose={() => {
        if (!action.busy) onClose();
      }}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (!preview || !summary?.count) return;
          void action.run(async () => {
            const value = {
              name,
              spec: {
                kind: "derive" as const,
                snapshot_id: snapshotId,
                filter,
                review_watermark: preview.review_watermark,
              },
            };
            const job = await context.client.aesthetic.analysis.create(
              context.projectId,
              { ...value, idempotency_key: action.key(value) },
            );
            onCreated(job);
            onClose();
          });
        }}
      >
        <div className="wb-field-list">
          <label>
            工作集名称
            <input
              value={name}
              required
              maxLength={120}
              disabled={action.busy}
              onChange={(e) => setName(e.target.value)}
            />
          </label>
          <label>
            Rating
            <input value={rating.toUpperCase()} readOnly />
          </label>
          <label>
            排名范围
            <select
              aria-label="排名范围"
              value={mode}
              disabled={action.busy}
              onChange={(e) => {
                setMode(e.target.value as "percent" | "count");
                invalidate();
              }}
            >
              <option value="percent">前百分比</option>
              <option value="count">前 N 名</option>
            </select>
          </label>
          {mode === "percent" ? (
            <label>
              排名前百分比
              <input
                aria-label="排名前百分比"
                type="number"
                min={0.01}
                max={100}
                step="any"
                required
                value={percent}
                disabled={action.busy}
                onChange={(e) => {
                  setPercent(Number(e.target.value));
                  invalidate();
                }}
              />
            </label>
          ) : (
            <label>
              前 N 名
              <input
                aria-label="前 N 名"
                type="number"
                min={1}
                max={10000000}
                step={1}
                required
                value={topCount}
                disabled={action.busy}
                onChange={(e) => {
                  setTopCount(Number(e.target.value));
                  invalidate();
                }}
              />
            </label>
          )}
          <label>
            额外保留保护候选
            <input
              type="checkbox"
              checked={protect}
              disabled={action.busy}
              onChange={(e) => {
                setProtect(e.target.checked);
                invalidate();
              }}
            />
          </label>
        </div>
        <p>
          边界保留整个并列组；保护候选可额外加入。预览会按固定的复核状态统计完整范围，显示实际数量。
        </p>
        <Button
          type="button"
          disabled={action.busy || !valid}
          onClick={() => void previewSelection()}
        >
          预览筛选
        </Button>
        {countJob.data && analysisActive(countJob.data.state) && (
          <p role="status">
            正在统计完整筛选范围：{countJob.data.progress.toLocaleString()} /{" "}
            {countJob.data.total.toLocaleString()}；可关闭窗口，统计在后台继续。
          </p>
        )}
        {summary && (
          <div className="aesthetic-notice" role="status">
            <strong>实际将生成 {summary.count.toLocaleString()} 张图片</strong>
            <p>
              排名筛选 {summary.ranked_count.toLocaleString()}{" "}
              张（其中边界并列组 {summary.boundary_tie_count.toLocaleString()}{" "}
              张，已包含在内）＋额外保护{" "}
              {summary.protected_added.toLocaleString()} 张。
            </p>
          </div>
        )}
        {preview && (
          <>
            <div className="aesthetic-preview-images">
              {preview.items.map((item) => (
                <figure
                  key={`${item.ranking.key.source_id}:${item.ranking.key.asset_id}`}
                >
                  <AssetImage
                    client={context.client}
                    projectId={context.projectId}
                    asset={rankingAsset(item.ranking)}
                    edge={240}
                  />
                  <figcaption>
                    {rankingLabel(item.ranking)}
                    {item.effective_protected && " · 保护候选"}
                  </figcaption>
                </figure>
              ))}
            </div>
            <p>
              本页预览 {preview.items.length} 张 · 复核水位{" "}
              {preview.review_watermark}
              {preview.next_cursor ? " · 还有后续预览" : " · 已到预览末尾"}
            </p>
            {preview.next_cursor && (
              <button
                type="button"
                disabled={action.busy}
                onClick={() => void previewSelection(preview.next_cursor)}
              >
                继续预览
              </button>
            )}
          </>
        )}
        {(action.error ?? countJob.error ?? countJob.data?.error) != null && (
          <ErrorDetails
            error={action.error ?? countJob.error ?? countJob.data?.error}
          />
        )}
        {countJob.data &&
          ["failed", "cancelled", "interrupted"].includes(
            countJob.data.state,
          ) && (
            <button
              type="button"
              disabled={action.busy}
              onClick={() => {
                void action.run(async () => {
                  await context.client.aesthetic.analysis.control(
                    context.projectId,
                    countJobId,
                    "resume",
                  );
                  await countJob.refetch();
                });
              }}
            >
              恢复筛选统计
            </button>
          )}
        <div className="wb-dialog-actions">
          <Button type="button" disabled={action.busy} onClick={onClose}>
            取消
          </Button>
          <Button
            type="submit"
            className="primary"
            disabled={action.busy || !preview || !summary?.count}
          >
            {action.busy ? "正在提交…" : "生成完整工作集"}
          </Button>
        </div>
      </form>
    </WorkbenchDialog>
  );
}
