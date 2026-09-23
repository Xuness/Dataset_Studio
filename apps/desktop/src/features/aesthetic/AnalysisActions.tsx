import { useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { Schema } from "@studio/contracts";
import type { ModuleContext } from "@studio/ui";
import { Button, ErrorDetails, WorkbenchDialog } from "@studio/ui";
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
}: {
  context: ModuleContext;
  onClose: () => void;
  onCreated: (job: Job) => void;
}) {
  const action = useAction();
  const [after, setAfter] = useState<string>();
  const stages = useQuery({
    queryKey: ["project", context.projectId, "aesthetic", "fit-stages", after],
    queryFn: ({ signal }) =>
      context.client.aesthetic.stages(context.projectId, after, signal),
  });
  const [stageId, setStageId] = useState("");
  const [name, setName] = useState("排名快照");
  const [estimator, setEstimator] = useState("davidson_v1");
  const [iterations, setIterations] = useState(128);
  const [regularization, setRegularization] = useState(0.1);
  const [split, setSplit] = useState(true);
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
                    tie_strength: 1,
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
              onChange={(e) => setName(e.target.value)}
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
              {stages.data?.items.map((stage) => (
                <option
                  key={stage.id}
                  value={stage.id}
                  disabled={!stage.accepted}
                >
                  {stage.name} · {stage.accepted} 批有效
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
            <small>{estimator === "davidson_v1" ? "Davidson" : "Borda"}</small>
          </summary>
          <div className="wb-fold-body wb-field-list">
            <label>
              估计器
              <select
                value={estimator}
                disabled={action.busy}
                onChange={(e) => setEstimator(e.target.value)}
              >
                <option value="davidson_v1">Davidson · 待质量校准</option>
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
            disabled={action.busy || !stageId}
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
  const [percent, setPercent] = useState(25);
  const [protect, setProtect] = useState(true);
  const [preview, setPreview] = useState<
    Schema["AestheticRankingSelection"] | null
  >(null);
  const filter = {
    ratings: [rating],
    top_percent: percent,
    include_protected: protect,
  };
  async function previewSelection(after?: string | null) {
    await action.run(
      async () =>
        setPreview(
          await context.client.aesthetic.analysis.select(
            context.projectId,
            snapshotId,
            { filter, after: after ?? null, limit: 12 },
          ),
        ),
      false,
    );
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
          if (!preview) return;
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
            排名前百分比
            <input
              aria-label="排名前百分比"
              type="number"
              min={0.01}
              max={100}
              step="any"
              value={percent}
              required
              disabled={action.busy}
              onChange={(e) => {
                setPercent(Number(e.target.value));
                setPreview(null);
              }}
            />
          </label>
          <label>
            额外保留保护候选
            <input
              type="checkbox"
              checked={protect}
              disabled={action.busy}
              onChange={(e) => {
                setProtect(e.target.checked);
                setPreview(null);
              }}
            />
          </label>
        </div>
        <p>
          后端按完整筛选范围生成，截断边界保留整个并列组；保护状态固定在预览的复核水位。
        </p>
        <Button
          type="button"
          disabled={
            action.busy ||
            !Number.isFinite(percent) ||
            percent <= 0 ||
            percent > 100
          }
          onClick={() => void previewSelection()}
        >
          预览筛选
        </Button>
        {preview && (
          <div className="aesthetic-notice" role="status">
            本页预览 {preview.items.length} 项 · 复核水位{" "}
            {preview.review_watermark}。
            {preview.next_cursor
              ? "还有待扫描的候选，这不是总数。"
              : "本次筛选已扫描到末尾。"}
            {preview.next_cursor && (
              <button
                type="button"
                disabled={action.busy}
                onClick={() => void previewSelection(preview.next_cursor)}
              >
                继续预览
              </button>
            )}
          </div>
        )}
        {action.error !== null && <ErrorDetails error={action.error} />}
        <div className="wb-dialog-actions">
          <Button type="button" disabled={action.busy} onClick={onClose}>
            取消
          </Button>
          <Button
            type="submit"
            className="primary"
            disabled={action.busy || !preview}
          >
            {action.busy ? "正在提交…" : "生成完整工作集"}
          </Button>
        </div>
      </form>
    </WorkbenchDialog>
  );
}
