import type { Schema } from "@studio/contracts";
import { Button, ErrorDetails } from "@studio/ui";
import type { ModuleContext } from "@studio/ui";
import { AssetImage } from "../browser/AssetImage.js";
import { rankingAsset, rankingLabel } from "./analysisPresentation.js";
import type { RankingRow } from "./analysisPresentation.js";
import { protectionLabel, useCandidateReviews } from "./RankingReviewPanel.js";

export function RankingDetails({
  context,
  snapshotId,
  row,
  thumbnailSize,
  pageSize,
  disabled,
  onThumbnailSize,
  onPageSize,
  onImage,
  onEvidence,
  onReview,
}: {
  context: ModuleContext;
  snapshotId: string;
  row: RankingRow | undefined;
  thumbnailSize: number;
  pageSize: number;
  disabled: boolean;
  onThumbnailSize: (value: number) => void;
  onPageSize: (value: number) => void;
  onImage: () => void;
  onEvidence: () => void;
  onReview: () => void;
}) {
  const reviews = useCandidateReviews(context, snapshotId, row?.ordinal);
  return (
    <div className="ranking-inspector">
      {row ? (
        <>
          <div className="ranking-panel-identity">
            <button
              type="button"
              className="ranking-detail-preview"
              aria-label="查看大图"
              disabled={disabled}
              onClick={onImage}
            >
              <AssetImage
                client={context.client}
                projectId={context.projectId}
                asset={rankingAsset(row)}
                edge={480}
              />
            </button>
            <div>
              <strong>候选 {row.ordinal + 1}</strong>
              <span>{rankingLabel(row)}</span>
            </div>
          </div>
          <details className="wb-fold" open>
            <summary>图片状态</summary>
            <dl className="wb-property-list">
              <dt>名次</dt>
              <dd>{rankingLabel(row)}</dd>
              <dt>有效曝光</dt>
              <dd>{row.exposures} 次</dd>
              <dt>需要检查</dt>
              <dd>{row.needs_review ? "是" : "未标记"}</dd>
              <dt>快照内提名</dt>
              <dd>{row.protected ? "已提名" : "未提名"}</dd>
              <dt>最新保护状态</dt>
              <dd>
                {reviews.isPending
                  ? "读取中…"
                  : reviews.isError
                    ? "读取失败"
                    : protectionLabel(
                        reviews.data?.items[0]?.request.decision,
                        row.protected,
                      )}
              </dd>
              <dt>Rating / 年份</dt>
              <dd>
                {row.rating.toUpperCase()} / {row.year ?? "未知"}
              </dd>
            </dl>
          </details>
          {reviews.error && <ErrorDetails error={reviews.error} />}
          <div className="ranking-inspector-actions">
            <Button onClick={onEvidence}>查看评审依据</Button>
            <Button onClick={onReview}>复核保护状态</Button>
          </div>
        </>
      ) : (
        <p className="aesthetic-help">选择图片，查看名次与保护状态。</p>
      )}
      <details className="wb-fold" open>
        <summary>显示设置</summary>
        <div className="ranking-display-settings">
          <label>
            缩略图大小 <output>{thumbnailSize} px</output>
            <input
              type="range"
              aria-label="排名缩略图大小"
              min={128}
              max={320}
              step={16}
              value={thumbnailSize}
              disabled={disabled}
              onChange={(e) => onThumbnailSize(Number(e.target.value))}
            />
          </label>
          <label>
            每页图片
            <select
              aria-label="排名每页图片数"
              value={pageSize}
              disabled={disabled}
              onChange={(e) => onPageSize(Number(e.target.value))}
            >
              {[12, 48, 96].map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </label>
          <p>方向键选图 · Enter 查看大图</p>
        </div>
      </details>
      {row && (
        <details className="wb-fold">
          <summary>来源身份</summary>
          <code className="ranking-asset-id">{row.key.asset_id}</code>
        </details>
      )}
    </div>
  );
}

export function RankingEvidence({
  row,
  snapshot,
  stage,
  error,
  onPrompt,
  onEvaluation,
}: {
  row: RankingRow | undefined;
  snapshot: Schema["AestheticAnalysisJob"] | undefined;
  stage: Schema["AestheticStage"] | undefined;
  error: unknown;
  onPrompt: () => void;
  onEvaluation: () => void;
}) {
  if (!row)
    return <p className="aesthetic-help">选择图片，查看这份快照的统计依据。</p>;
  const result = snapshot?.result?.kind === "fit" ? snapshot.result : undefined;
  return (
    <div className="ranking-evidence">
      <div className="ranking-panel-identity">
        <strong>候选 {row.ordinal + 1}</strong>
        <span>{snapshot?.request.name}</span>
      </div>
      <details className="wb-fold" open>
        <summary>可比范围</summary>
        <dl className="wb-property-list">
          <dt>名次</dt>
          <dd>{rankingLabel(row)}</dd>
          <dt>比较范围</dt>
          <dd>
            {row.rating_rank_min != null
              ? `${row.rating.toUpperCase()} 类`
              : row.component != null
                ? `分量 ${row.component} · ${row.component_size} 图`
                : "暂无可比范围"}
          </dd>
          <dt>估计器</dt>
          <dd>{result?.estimator_version ?? "未知"}</dd>
          <dt>拟合状态</dt>
          <dd>{result ? (result.converged ? "已收敛" : "未收敛") : "未知"}</dd>
        </dl>
        <p className="aesthetic-help">
          并列名次保留区间；不同 Rating 或未连通分量的名次不能直接比较。
        </p>
      </details>
      <details className="wb-fold" open>
        <summary>证据与稳定性</summary>
        <dl className="wb-property-list">
          <dt>有效曝光</dt>
          <dd>{row.exposures} 次</dd>
          <dt>分半波动</dt>
          <dd>
            {row.split_percentile_delta == null
              ? "未知 / 证据不足"
              : `${(row.split_percentile_delta * 100).toFixed(2)} 个百分点`}
          </dd>
          <dt>拟合内分歧</dt>
          <dd>
            {row.disagreement == null ? "未知" : row.disagreement.toFixed(3)}
          </dd>
          <dt>不可评判</dt>
          <dd>{row.unjudgeable} 次</dd>
          <dt>跨年曝光</dt>
          <dd>{row.cross_year_exposures} 次</dd>
          <dt>对手数量估计</dt>
          <dd>
            {row.opponent_diversity_estimate == null
              ? "未知"
              : `约 ${Math.round(row.opponent_diversity_estimate)}`}
          </dd>
        </dl>
        <p className="aesthetic-help">
          分半波动来自两组完整批次的重拟合，不是置信区间。未知不代表稳定；“待检查”是诊断标记。
        </p>
      </details>
      <details className="wb-fold">
        <summary>
          评审配置 <small>冻结副本</small>
        </summary>
        <dl className="wb-property-list">
          <dt>评审阶段</dt>
          <dd>{stage?.name ?? "读取中…"}</dd>
          <dt>模型</dt>
          <dd>{stage?.config.model.remote_model_id ?? "—"}</dd>
          <dt>标准版本</dt>
          <dd>{stage?.config.model.system_prompt_revision ?? "—"}</dd>
          <dt>证据水位</dt>
          <dd>{snapshot?.input.evidence_watermark ?? "—"}</dd>
          <dt>统计分数</dt>
          <dd>{row.score?.toFixed(5) ?? "未知"}</dd>
        </dl>
        <div className="ranking-inspector-actions">
          <Button disabled={!stage} onClick={onPrompt}>
            查看冻结 System Prompt
          </Button>
          <Button disabled={!stage} onClick={onEvaluation}>
            查看阶段批次
          </Button>
        </div>
        <p className="aesthetic-help">
          这里展示快照统计；阶段页面包含后续批次，可能超出这份快照的证据水位。
        </p>
      </details>
      {error != null && <ErrorDetails error={error} />}
    </div>
  );
}
