import type { RankingInput } from "@studio/contracts";
import "./RankingInputEvidence.css";

export function RankingInputEvidence({ input: i }: { input: RankingInput }) {
  const e = i.evidence;
  return (
    <section className="ranking-input-evidence" aria-label="评分使用的冻结数据">
      <h4>评分使用的冻结数据</h4>
      <p>
        分级 {i.rating?.toUpperCase() ?? "未知"} · 元数据帖子 #
        {i.post_id ?? "未知"}
      </p>
      <p>
        净分 {i.score ?? "未知"} · 收藏 {i.fav_count ?? "未知"} · 赞票{" "}
        {i.up_score ?? "未知"} · 负票 {i.down_score ?? "未知"}
      </p>
      {e ? (
        <>
          <p>
            同图关联 {e.post_count} 个帖子；分级取最新元数据。热度
            {e.policy === "highest"
              ? `取较高记录 #${e.heat[0]?.post_id ?? "未知"}`
              : "按不同帖子求和"}
            。
          </p>
          {e.policy === "sum" && e.post_count > 1 && (
            <p>跨帖合计无法对应单一帖龄，本次时间比较按未知时间回退。</p>
          )}
          <details>
            <summary>查看热度来源</summary>
            <table>
              <thead>
                <tr>
                  <th>帖子</th>
                  <th>原分级</th>
                  <th>净分</th>
                  <th>收藏</th>
                  <th>赞票</th>
                  <th>负票</th>
                </tr>
              </thead>
              <tbody>
                {e.heat.map((p) => (
                  <tr key={p.observation_id}>
                    <td>#{p.post_id ?? "未知"}</td>
                    <td>{p.rating?.toUpperCase() ?? "未知"}</td>
                    <td>{p.score ?? "未知"}</td>
                    <td>{p.fav_count ?? "未知"}</td>
                    <td>{p.up_score ?? "未知"}</td>
                    <td>{p.down_score ?? "未知"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {e.omitted_posts > 0 && (
              <p>
                其余 {e.omitted_posts} 个帖子的数值已计入，明细只展示前 64 条。
              </p>
            )}
          </details>
          {e.partial_counts && <p>部分帖子的计数缺失，合计仅包含已知数值。</p>}
          {e.counts_clamped && <p>合计超出整数范围，已按上限保存。</p>}
        </>
      ) : (
        <p>以上数值来自评分时固定的代表记录。</p>
      )}
    </section>
  );
}
