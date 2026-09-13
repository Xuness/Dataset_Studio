import { useEffect } from "react";
import { X, Image as ImageIcon } from "lucide-react";
import { Button } from "@studio/ui";
import type { ModuleContext } from "@studio/ui";
import type { RankingRow, RankingSummary, Asset } from "@studio/contracts";
import { AssetImage } from "../browser/AssetImage.js";
import { RankingInputEvidence } from "./RankingInputEvidence.js";
import {
  eligibilityNames,
  flagNames,
  integer,
  number,
  routeNames,
  score,
  time,
} from "./types.js";
export function RankingDetails({
  row,
  summary,
  artifactId,
  context,
  onClose,
}: {
  row: RankingRow;
  summary: RankingSummary;
  artifactId: string;
  context: ModuleContext;
  onClose: () => void;
}) {
  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("keydown", close);
    return () => document.removeEventListener("keydown", close);
  }, [onClose]);
  const { input: i, scores: s } = row;
  const p = summary.parameters;
  const asset: Asset = {
    key: { source_id: i.source_id, asset_id: i.asset_id },
    name: i.post_id ? "帖子 #" + i.post_id : i.asset_id,
    source_name:
      context.sources.find((v) => v.id === i.source_id)?.name ?? "Danbooru",
    bytes: i.stored_bytes,
    selected: false,
    extension: i.stored_extension,
    ranking: {
      artifact_id: artifactId,
      ordinal: i.ordinal,
      post_id: i.post_id ?? null,
      rating: i.rating ?? null,
      record_id: i.record_id ?? null,
      observation_id: i.observation_id ?? null,
      eligibility: s.eligibility,
      main_score: s.main_score ?? null,
      rescue_score: s.rescue_score ?? null,
      main_rank: s.main_rank ?? null,
      rescue_rank: s.rescue_rank ?? null,
      v2: s.v2 ?? null,
    },
  };
  const contributions =
    s.main_score == null
      ? []
      : s.v2
        ? [
            ["直算原分", score(s.v2.direct_raw * 100)],
            ["直榜总体百分位", score(s.v2.direct_percentile * 100) + "%"],
            ["年代原分", score(s.v2.era_raw * 100)],
            ["年代榜总体百分位", score(s.v2.era_percentile * 100) + "%"],
            ["融合分 F", score(s.v2.fused_score)],
            ["类型软降分", "−" + score(s.v2.type_penalty)],
            ["年代偏好", score(s.v2.era_bonus)],
            ["筛选优先级 J", score(s.main_score)],
            ["直算名次", number(s.v2.direct_rank)],
            ["融合名次", number(s.v2.fused_rank)],
            ["年内名次", number(s.v2.year_rank)],
          ]
        : [
            ["绝对热度 G", score((s.g ?? 0) * 100)],
            [
              "时间补救",
              "+" +
                score(
                  p.time_weight * Math.max(0, (s.c ?? 0) - (s.g ?? 0)) * 100,
                ),
            ],
            ["负票扣分", "−" + score(p.vote_weight * (s.v ?? 0) * 100)],
            ["技术损伤扣分", "−" + score((s.t ?? 0) * 100)],
            ["主分 S", score(s.main_score)],
            ["补救分 R", score(s.rescue_score)],
          ];
  const reasons: Record<string, string> = {
    disabled: "本次未启用时间补救",
    time_unknown: "时间未知，C=G",
    time_invalid: "时间无效，C=G",
    heat_unknown: "热度未知，停用时间补救",
    cohort_insufficient: "比较群体不足，C=G",
    used: "已使用时间条件化表现",
    not_eligible: "本图未参与排名",
  };
  return (
    <aside className="ranking-details" aria-label="评分依据">
      <header>
        <strong>评分依据</strong>
        <button
          className="icon-button"
          onClick={onClose}
          aria-label="关闭评分依据"
        >
          <X size={15} />
        </button>
      </header>
      <div className="ranking-detail-scroll">
        <AssetImage
          client={context.client}
          projectId={context.projectId}
          asset={asset}
          edge={360}
          className="ranking-detail-image"
        />
        <p className="ranking-detail-title">
          {asset.name} · {i.rating?.toUpperCase() ?? "分级未知"}
        </p>
        <span className={"ranking-route " + s.selected_route}>
          {routeNames[s.selected_route]}
        </span>
        <p className="ranking-hint">{eligibilityNames[s.eligibility]}</p>
        {!!contributions.length && (
          <dl className="ranking-contributions">
            {contributions.map(([name, value]) => (
              <div key={name}>
                <dt>{name}</dt>
                <dd>{value}</dd>
              </div>
            ))}
          </dl>
        )}
        {s.v2 && (
          <div className="ranking-explanation">
            <p>v2 元数据模式，未进行 Bridge 视觉校准。</p>
            <p>
              时期 {s.v2.old_period ?? "未知"} → {s.v2.new_period ?? "未知"}
              ，后者权重 {score(s.v2.new_weight * 100)}%。
            </p>
            <p>
              收缩后年代表现 {score(s.v2.year_percentile * 100)}%；有效参考数量{" "}
              {number(Math.round(s.v2.effective_count))}
              （两侧有贡献时取较小值）。
            </p>
            <p>
              {!p.time_enabled
                ? "本次关闭时间与年代参考，融合使用直算榜。"
                : s.v2.era_fallback
                  ? "部分年代比较缺少时间或样本，已使用条件化表现回退。"
                  : "本图的年代参考群体达到有效样本下限。"}
            </p>
            <p>
              {s.v2.layout_protected
                ? "命中分镜／构图保护线索，本次不作漫画类型降分。"
                : s.v2.type_penalty > 0
                  ? "命中扩展漫画标签线索，仅施加配置的软降分。"
                  : "本次没有漫画类型降分。"}
            </p>
            {s.v2.selection_reason > 0 && (
              <p>
                入选依据：
                {
                  (
                    {
                      1: "直榜独有补救",
                      2: "年代榜独有补救",
                      3: "预算内随机审计",
                    } as Record<number, string>
                  )[s.v2.selection_reason]
                }
              </p>
            )}
          </div>
        )}
        <div className="ranking-explanation">
          <p>{reasons[s.time_reason] ?? s.time_reason}</p>
          {s.local_count > 0 && (
            <p>
              比较样本 {number(s.local_count)} · 局部百分位{" "}
              {score((s.local_percentile ?? 0) * 100)}% · 支持收缩 K=
              {score(s.support_k)}。
            </p>
          )}
          <p>
            {p.artist_enabled
              ? `画师先验 A=${score(s.a)}；有效其他作品组最多 ${number(s.artist_support)} 份。`
              : "本次关闭画师先验，A=0。"}
          </p>
        </div>
        <h4>固定元数据</h4>
        <RankingInputEvidence input={i} />
        <dl className="ranking-input-fields">
          {[
            ["收藏", integer(i.fav_count)],
            ["赞票", integer(i.up_score)],
            ["原始负票", integer(i.down_score)],
            ["原始净分", integer(i.score)],
            [
              i.evidence
                ? i.evidence.policy === "sum"
                  ? "最早已知发帖时间"
                  : "热度记录创建时间"
                : "创建时间",
              time(i.created_at_us),
            ],
            [
              "观察时间",
              time(i.observed_at_us, i.time_quality === "date_only"),
            ],
            ["画师标签", i.artists.join("、") || "未知"],
            [
              "成品尺寸",
              i.stored_width && i.stored_height
                ? `${i.stored_width} × ${i.stored_height}`
                : i.dimension_basis === "not_requested"
                  ? "本次未核验"
                  : "未知",
            ],
            ["匹配来源记录", number(i.record_count)],
          ].map(([name, value]) => (
            <div key={name}>
              <dt>{name}</dt>
              <dd>{value}</dd>
            </div>
          ))}
        </dl>
        {!!s.missing_flags.length && (
          <div className="ranking-tags">
            {s.missing_flags.map((f) => (
              <span key={f}>{flagNames[f] ?? f}</span>
            ))}
          </div>
        )}
        <details className="ranking-advanced">
          <summary>完整快照与特征</summary>
          <pre>{JSON.stringify(row, null, 2)}</pre>
        </details>
        <Button
          type="button"
          onClick={() => {
            context.browser.onInspect(asset);
            context.activateView("core.browser");
          }}
        >
          <ImageIcon size={13} />
          在资料浏览中查看
        </Button>
      </div>
    </aside>
  );
}
