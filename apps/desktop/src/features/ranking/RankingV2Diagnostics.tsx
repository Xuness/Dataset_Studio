import type { RankingSummary } from "@studio/contracts";
import { number, score } from "./types.js";
export function RankingV2Diagnostics({ summary }: { summary: RankingSummary }) {
  if (!summary.parameters.v2) return null;
  return (
    <>
      <section className="ranking-card ranking-wide">
        <h3>v2 · 元数据多阶段</h3>
        <p className="ranking-hint">
          直算榜与羽化年代相对榜在同一分级内融合，再应用类型软降分和年代偏好。当前结果没有
          Bridge 视觉校准，也没有视觉质量标签。
        </p>
        <div className="ranking-table-scroll">
          <table className="ranking-summary-table">
            <thead>
              <tr>
                <th>分级</th>
                <th>年代比较回退</th>
                <th>构图保护</th>
                <th>类型软降分</th>
                <th>直榜补救</th>
                <th>年代榜补救</th>
                <th>审计</th>
                <th>保护名额回流</th>
                <th>年代名额回流</th>
              </tr>
            </thead>
            <tbody>
              {summary.ratings.map((r) => (
                <tr key={r.rating}>
                  <td>{r.rating.toUpperCase()}</td>
                  <td>{number(r.v2?.fallback_count)}</td>
                  <td>{number(r.v2?.protected_count)}</td>
                  <td>{number(r.v2?.type_penalized)}</td>
                  <td>{number(r.v2?.direct_rescued)}</td>
                  <td>{number(r.v2?.era_rescued)}</td>
                  <td>{number(r.v2?.audit_selected)}</td>
                  <td>
                    {number(
                      r.v2?.protection_shortfall.reduce((a, b) => a + b, 0),
                    )}
                  </td>
                  <td>{number(r.v2?.era_target_shortfall)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <p className="ranking-hint">
          保护名额不足会回流主榜；年代候选不足仅在允许回流时重新分配。仅排名模式不执行名额分配。
        </p>
      </section>
      {summary.ratings.map((r) => (
        <section className="ranking-card ranking-wide" key={r.rating}>
          <h3>{r.rating.toUpperCase()} · 上传年份分布</h3>
          <div className="ranking-table-scroll">
            <table className="ranking-summary-table">
              <thead>
                <tr>
                  <th>年份</th>
                  <th>候选</th>
                  <th>候选占比</th>
                  <th>前 1% 数量</th>
                  <th>前段占比</th>
                  <th>保留数量</th>
                  <th>回退数量</th>
                  <th>构图保护</th>
                  <th>软降分</th>
                </tr>
              </thead>
              <tbody>
                {r.v2?.years.map((y) => (
                  <tr key={y.year ?? "unknown"}>
                    <td>{y.year ?? "未知"}</td>
                    <td>{number(y.count)}</td>
                    <td>{score((100 * y.count) / Math.max(1, r.eligible))}%</td>
                    <td>{number(y.top_count)}</td>
                    <td>
                      {score(
                        (100 * y.top_count) /
                          Math.max(1, Math.ceil(r.eligible / 100)),
                      )}
                      %
                    </td>
                    <td>
                      {summary.parameters.mode === "select"
                        ? number(y.selected)
                        : "未分配"}
                    </td>
                    <td>{number(y.fallback)}</td>
                    <td>{number(y.protected)}</td>
                    <td>{number(y.penalized)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <p className="ranking-hint">
            每张图片按实际上传年份计一次。前段指该分级最终优先级的前
            1%；分布变化本身不证明视觉召回提升。
          </p>
        </section>
      ))}
    </>
  );
}
