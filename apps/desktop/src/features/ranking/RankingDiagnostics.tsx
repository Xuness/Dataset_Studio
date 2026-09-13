import { RankingV2Diagnostics } from "./RankingV2Diagnostics.js";
import { useQuery } from "@tanstack/react-query";
import type { StudioClient } from "@studio/client";
import type { RankingSummary, RankingEligibility } from "@studio/contracts";
import { Button } from "@studio/ui";
import { eligibilityNames, flagNames, number } from "./types.js";
export function RankingDiagnostics({
  summary: s,
  client,
  projectId,
  artifactId,
  onEligibility,
}: {
  summary: RankingSummary;
  client: StudioClient;
  projectId: string;
  artifactId: string;
  onEligibility: (value: RankingEligibility) => void;
}) {
  const evidence = useQuery({
    queryKey: ["project", projectId, "ranking-evidence", artifactId],
    queryFn: ({ signal }) =>
      client.ranking.evidence(projectId, artifactId, signal),
    gcTime: 0,
  });
  return (
    <div className="ranking-diagnostics">
      <RankingV2Diagnostics summary={s} />
      <section className="ranking-card">
        <h3>输入资格</h3>
        <p className="ranking-hint">
          固定输入 {number(s.input_count)} 个图片对象，合格候选{" "}
          {number(s.eligible_count)} 个。未逐图进行解码检查。
        </p>
        <table className="ranking-summary-table">
          <thead>
            <tr>
              <th>状态</th>
              <th>数量</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {Object.entries(s.eligibility_counts).map(([key, value]) => (
              <tr key={key}>
                <td>{eligibilityNames[key as RankingEligibility] ?? key}</td>
                <td>{number(value)}</td>
                <td>
                  <Button
                    type="button"
                    onClick={() => onEligibility(key as RankingEligibility)}
                  >
                    查看
                  </Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
      <section className="ranking-card">
        <h3>字段与统计提示</h3>
        <p className="ranking-hint">提示可以重叠，不等同于图片淘汰数量。</p>
        <table className="ranking-summary-table">
          <tbody>
            {Object.entries(s.missing_counts).map(([key, value]) => (
              <tr key={key}>
                <td>{flagNames[key] ?? key}</td>
                <td>{number(value)}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {!Object.keys(s.missing_counts).length && (
          <p className="ranking-hint">本次没有已记录的字段提示。</p>
        )}
      </section>
      <section className="ranking-card ranking-wide">
        <h3>分级与通道</h3>
        <div className="ranking-table-scroll">
          <table className="ranking-summary-table">
            <thead>
              <tr>
                <th>分级</th>
                <th>合格候选</th>
                <th>有效热度</th>
                <th>时间补救</th>
                <th>主通道</th>
                <th>补救</th>
                <th>审计</th>
                <th>与收藏排序重叠</th>
              </tr>
            </thead>
            <tbody>
              {s.ratings.map((r) => (
                <tr key={r.rating}>
                  <td>{r.rating.toUpperCase()}</td>
                  <td>{number(r.eligible)}</td>
                  <td>{number(r.valid_heat)}</td>
                  <td>{number(r.time_used)}</td>
                  {r.selected.map((v, i) => (
                    <td key={i}>
                      {s.parameters.mode === "rank" ? "—" : number(v)}
                    </td>
                  ))}
                  <td>
                    {s.parameters.mode === "rank"
                      ? "未分配名额"
                      : number(r.favorite_baseline_overlap)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <p className="ranking-hint">
          收藏基线采用相同分级、候选池和总名额。重叠数量衡量选择差异，不能证明视觉质量或召回提升。
        </p>
      </section>
      <section className="ranking-card ranking-wide">
        <h3>时间补救覆盖</h3>
        <div className="ranking-table-scroll">
          <table className="ranking-summary-table">
            <thead>
              <tr>
                <th>分级</th>
                <th>±6 个月</th>
                <th>±12 个月</th>
                <th>±24 个月</th>
                <th>合并层级一</th>
                <th>合并层级二</th>
                <th>未使用</th>
              </tr>
            </thead>
            <tbody>
              {s.ratings.map((r) => (
                <tr key={r.rating}>
                  <td>{r.rating.toUpperCase()}</td>
                  {r.cohort_counts.map((v, i) => (
                    <td key={i}>{number(v)}</td>
                  ))}
                  <td>{number(r.time_fallback)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <p className="ranking-hint">
          “未使用”包含关闭补救、时间未知、热度未知及群体不足；具体原因可在单图依据中查看。
        </p>
      </section>
      <section className="ranking-card ranking-wide">
        <h3>范围与版本依据</h3>
        <p className="ranking-hint">
          本成果使用固定元数据快照，后续来源变化不会改写这里的结果。
        </p>
        {evidence.error && (
          <p className="ranking-notice">{evidence.error.message}</p>
        )}
        <details className="ranking-advanced">
          <summary>参数、来源版本与查询条件</summary>
          <pre>
            {JSON.stringify(
              evidence.data ?? {
                parameters: s.parameters,
                input_sha256: s.input_sha256,
              },
              null,
              2,
            )}
          </pre>
        </details>
      </section>
    </div>
  );
}
