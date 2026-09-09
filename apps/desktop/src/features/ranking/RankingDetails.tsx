import { useEffect } from "react";
import { X, Image as ImageIcon } from "lucide-react";
import { Button } from "@studio/ui";
import type { ModuleContext } from "@studio/ui";
import type { RankingRow, RankingSummary, Asset } from "@studio/contracts";
import { AssetImage } from "../browser/AssetImage.js";
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
  context,
  onClose,
}: {
  row: RankingRow;
  summary: RankingSummary;
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
  };
  const contributions =
    s.main_score == null
      ? []
      : [
          ["绝对热度 G", score((s.g ?? 0) * 100)],
          [
            "时间补救",
            "+" +
              score(p.time_weight * Math.max(0, (s.c ?? 0) - (s.g ?? 0)) * 100),
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
        <dl className="ranking-input-fields">
          {[
            ["收藏", integer(i.fav_count)],
            ["赞票", integer(i.up_score)],
            ["原始负票", integer(i.down_score)],
            ["原始净分", integer(i.score)],
            ["创建时间", time(i.created_at_us)],
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
