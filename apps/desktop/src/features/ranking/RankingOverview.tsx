import {
  Layers,
  SlidersHorizontal,
  LockKeyhole,
  CircleHelp,
} from "lucide-react";
import type { RankingParameters, RankingSummary } from "@studio/contracts";
import { number } from "./types.js";

export function RankingOverview({
  parameters,
  summary,
  scopeName,
  count,
  configuration,
}: {
  parameters: RankingParameters;
  summary: RankingSummary | undefined;
  scopeName: string;
  count: number | null;
  configuration: boolean;
}) {
  const p = configuration ? parameters : (summary?.parameters ?? parameters);
  const quotas: [number, number, number] =
    p.mode === "select"
      ? [p.quotas[0] ?? 0, p.quotas[1] ?? 0, p.quotas[2] ?? 0]
      : [0, 0, 0];
  const selected =
    summary?.ratings.reduce(
      (n, r) => n + r.selected.reduce((a, b) => a + b, 0),
      0,
    ) ?? 0;
  return (
    <aside className="ranking-overview" aria-label="排名属性">
      <header className="ranking-dock-heading">
        <span>
          <SlidersHorizontal size={13} />
          {configuration ? "计算属性" : "成果属性"}
        </span>
        <span className="ranking-dock-caption">MetaRecall</span>
      </header>
      <div className="ranking-overview-scroll">
        <section className="ranking-property-section">
          <h3>
            <Layers size={13} />
            {configuration ? "当前范围" : "固定范围"}
          </h3>
          <p className="ranking-scope-name">
            {configuration
              ? scopeName
              : `${summary?.ratings.map((r) => r.rating.toUpperCase()).join(" / ") ?? "—"} · 元数据排名成果`}
          </p>
          <dl className="ranking-property-values">
            <div>
              <dt>{configuration ? "图片对象" : "固定输入"}</dt>
              <dd>{number(configuration ? count : summary?.input_count)}</dd>
            </div>
            {!configuration && summary && (
              <>
                <div>
                  <dt>合格候选</dt>
                  <dd>{number(summary.eligible_count)}</dd>
                </div>
                <div>
                  <dt>已入选</dt>
                  <dd>{p.mode === "select" ? number(selected) : "仅排名"}</dd>
                </div>
              </>
            )}
            <div>
              <dt>分级</dt>
              <dd>
                {["g", "s", "q", "e"]
                  .filter((r) => p.ratings.includes(r))
                  .map((r) => r.toUpperCase())
                  .join(" · ")}
              </dd>
            </div>
            <div>
              <dt>成品最短边</dt>
              <dd>
                {p.minimum_stored_side ? `${p.minimum_stored_side} px` : "不限"}
              </dd>
            </div>
          </dl>
          <p className="ranking-hint">
            {configuration
              ? "资格与准确数量在元数据准备后确定。"
              : "本成果的范围与参数已固定。"}
          </p>
        </section>
        <section className="ranking-property-section">
          <h3>通道分配</h3>
          {p.mode === "select" ? (
            <>
              <div
                className="ranking-budget-bar"
                role="img"
                aria-label={`主通道 ${quotas[0] / 10}%，补救 ${quotas[1] / 10}%，审计 ${quotas[2] / 10}%`}
              >
                {quotas.map((q, i) => (
                  <span
                    key={i}
                    className={["main", "rescue", "audit"][i]}
                    style={{ width: `${Math.min(100, q / 10)}%` }}
                  />
                ))}
              </div>
              <dl className="ranking-property-values ranking-budget-legend">
                {["主通道", "补救通道", "随机审计"].map((name, i) => (
                  <div key={name}>
                    <dt>
                      <i className={["main", "rescue", "audit"][i]} />
                      {name}
                    </dt>
                    <dd>{((quotas[i] ?? 0) / 10).toFixed(1)}%</dd>
                  </div>
                ))}
                <div>
                  <dt>合计</dt>
                  <dd>
                    {(quotas.reduce((a, b) => a + b, 0) / 10).toFixed(1)}%
                  </dd>
                </div>
              </dl>
            </>
          ) : (
            <p className="ranking-hint">为全部合格候选保存主排名和补救排名。</p>
          )}
        </section>
        <section className="ranking-property-section">
          <h3>评分组成</h3>
          <dl className="ranking-property-values ranking-feature-values">
            {[
              ["G", "分级热度", true],
              ["C", "时间补救", p.time_enabled],
              ["A", "画师先验", p.artist_enabled],
              ["V", "负票扣分", p.votes_enabled],
              ["T", "技术损伤", p.damage_enabled],
            ].map(([key, label, enabled]) => (
              <div key={String(key)}>
                <dt>
                  <b>{key}</b>
                  {label}
                </dt>
                <dd className={enabled ? "" : "subtle"}>
                  {enabled ? "启用" : "关闭"}
                </dd>
              </div>
            ))}
          </dl>
        </section>
        {configuration && summary && (
          <section className="ranking-property-section">
            <h3>
              <LockKeyhole size={12} />
              最近查看的成果
            </h3>
            <p className="ranking-hint">
              {number(summary.eligible_count)} 个合格候选 ·{" "}
              {summary.parameters.mode === "select"
                ? "已分配名额"
                : "仅计算排名"}
            </p>
          </section>
        )}
        <div className="ranking-panel-note">
          <CircleHelp size={13} />
          <p>
            {configuration
              ? "仅计算元数据。结果可保存为工作集，继续浏览与处理。"
              : "选择榜单条目查看评分依据。元数据排名表示筛选优先级。"}
          </p>
        </div>
      </div>
    </aside>
  );
}
