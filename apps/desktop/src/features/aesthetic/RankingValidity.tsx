import type { Schema } from "@studio/contracts";

const stability: Record<string, string> = {
  not_requested: "未请求分半诊断",
  main_not_converged: "主拟合未收敛",
  split_not_converged: "分半拟合未收敛",
  available: "分半诊断可用",
  partial_connected_overlap: "部分图片缺少完整分半连接",
  insufficient_connected_overlap: "分半后缺少完整比较连接",
};
export function RankingValidity({
  summary,
}: {
  summary: Schema["AestheticFitSummary"];
}) {
  if (!summary.validity)
    return (
      <p className="aesthetic-help">
        此历史快照未记录版本化有效性摘要；可在原证据上新建离线分析。
      </p>
    );
  return (
    <details
      className="wb-fold ranking-validity"
      open={summary.validity.groups.some((g) => g.ranking_scope !== "rating")}
    >
      <summary>排名范围与证据覆盖</summary>
      {summary.validity.groups.map((g) => (
        <p key={g.rating}>
          <strong>
            {g.rating.toUpperCase()} ·{" "}
            {g.ranking_scope === "rating"
              ? "Rating 内统一排名"
              : g.ranking_scope === "component"
                ? "仅支持各分量内部排名"
                : "尚无可比名次"}
          </strong>
          {" · "}比较覆盖 {g.compared} / {g.candidates}；分半诊断覆盖{" "}
          {g.stability_covered} / {g.candidates}（
          {stability[g.stability] ?? g.stability}）。
        </p>
      ))}
      {summary.validity.groups.some((g) => g.ranking_scope === "component") && (
        <p>
          不同分量在列表中的先后是展示顺序，不代表质量高低。追加跨组比较后需发布新快照。
        </p>
      )}
      <p className="aesthetic-help">
        计算完成、数值收敛、比较连接和审美正确性分别判断。缺失的稳定性不等于零波动。
      </p>
    </details>
  );
}
