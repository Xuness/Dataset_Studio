import type { Schema } from "@studio/contracts";

export function UsageSummary({
  value,
}: {
  value: Schema["AestheticUsageSummary"];
}) {
  const percent = (n: number, d: number) =>
    d > 0 ? `${((n / d) * 100).toFixed(2)}%` : "未知";
  return (
    <div className="aesthetic-help" aria-label="缓存与费用统计">
      <p>
        请求缓存命中{" "}
        {percent(value.cache_hit_requests, value.cache_observed_requests)}
        {`（${value.cache_hit_requests}/${value.cache_observed_requests} 次有缓存明细）`}
        <br />
        输入 Token 缓存占比{" "}
        {percent(value.cached_input_tokens, value.cache_observed_input_tokens)}
        {` · 已读取 ${value.cached_input_tokens.toLocaleString()} tokens`}
        <br />
        缓存写入{" "}
        {value.cache_write_observed_requests > 0
          ? value.cache_write_tokens.toLocaleString()
          : "未知"}{" "}
        tokens
        {`（${value.cache_write_observed_requests}/${value.recorded_requests} 次已报告）`}
      </p>
      <p>
        已报告费用{" "}
        {value.cost_observed_requests > 0
          ? `$${value.cost_usd.toFixed(6)}`
          : "未知"}
        {`（${value.cost_observed_requests}/${value.recorded_requests} 次返回费用）`}
        。
        缓存比例仅统计有明细的回执；未返回费用或结果不明的调用不计入费用小计。
      </p>
    </div>
  );
}
