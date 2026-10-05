# 0060：OpenRouter 提示词缓存与用量观测

日期：2026-10-05

## 请求策略

OpenRouter 模型可配置 `openrouter.cache_strategy=implicit|system`。省略或 implicit 保留上游自动缓存；system 仅对 Gemini/Claude 的第一条 System/Developer 文本消息添加末尾 `cache_control: {type: "ephemeral"}`，保持全文及其顺序，变化的 User 内容和图片留在边界之后。不自动扩写提示词，不把 Meta 的隐式缓存当成支持显式控制。显式缓存的最低 token 长度、写入/存储费用以模型及端点为准。

`openrouter.cache_affinity=true` 启用稳定 `session_id`。新建美学阶段冻结独立会话 ID，批次及重试复用；通用调用按连接、远端模型、System 内容和工具定义生成摘要，排除请求 ID、动态 User 和图片。显式 `openrouter.session_id` 可覆盖。复制阶段移除其自动会话 ID，由新阶段重新生成。会话只用于 OpenRouter 路由及可观测分组，不引入对话历史。

`service_tier=flex` 要求配置具体 `/flex` 端点的 `provider.only`；兼容已有全 Flex `order` 时补齐同范围 `only`。缺少明确端点或混入普通层级时报错，不依赖 OpenRouter 在模型无 Flex 时的标准价回退。用户可在 only 列表内保留回退。手动 order 优先于缓存粘性路由，因此保留该设置并给出提示，不暗中重排用户偏好。

推荐 Gemini 配置：

```json
{
  "service_tier": "flex",
  "openrouter.provider": {
    "only": ["google-ai-studio/flex", "google-vertex/global/flex"],
    "allow_fallbacks": true
  },
  "openrouter.cache_affinity": true,
  "openrouter.cache_strategy": "implicit"
}
```

显式 System 缓存应先测量前缀长度。设置页的 System 输入测量只发送所选 System 与测试短句，使用隐式缓存，输出上限 64 tokens。返回的是上游输入计数，包含短句及消息封装；不将其标成纯 System 的精确分词数，也不将字符估算标成模型 token 计数。

## 回执与兼容

LLM 用量增加缓存写入 tokens、上游报告的 USD 费用、供应商及实际服务层级；JSON 和 SSE 都保留这些字段。缺失字段保持未知。费用直接取网关报告，不从静态价格表反推，也不将上游成本与网关费用混为一谈。

评审账本升级至 v9，升级前备份，仅一次性扫描已有标准化回执建立缓存及费用汇总；不重解析原始响应，不改写已接受证据。后续接收和本地重解析在同一写事务内增量替换汇总，重复接收不重复计数。界面分别显示请求命中率、输入 token 缓存比例、写入量和已报告费用覆盖度。无费用明细不等于零费用。

冻结阶段重关联时显式省略后来新增的模型默认参数，保留历史消息与参数。模型缓存设置影响新评审，不能默默改变旧评审的请求标准。

参考：[Prompt Caching](https://openrouter.ai/docs/guides/best-practices/prompt-caching)、[Service Tiers](https://openrouter.ai/docs/guides/features/service-tiers)、[Provider Routing](https://openrouter.ai/docs/guides/routing/provider-selection)。
