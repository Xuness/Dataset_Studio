# System Prompt 预设与按次传输

日期：2026-09-17。承接 [0023](0023-llm-foundation.md)，扩展其此前未实现的 System Prompt 保存与管理范围。

## 职责

- `domain/llm/system_prompts`：预设 ID、修订和名称/备注/原文，无供应商、模型或项目绑定。
- `application/llm/system_prompts`：保存校验、调用引用解析及消息冲突校验；服务入口仍为 `LlmService`。
- `storage/llm`：独立 `llm_system_prompts` 表，应用级 SQLite 注册表 v3 → v4；备份后事务升级，也支持 v0/v1/v2。项目格式保持不变。
- `protocol/llm/system_prompts`、`engine/api/llm/system_prompts`、SDK `llm/system-prompts`：独立 DTO、管理路由与公开客户端，不把逻辑写入前端网络请求或协议编码器。
- 设置页 `system-prompts`：新建、重命名、备注、正文编辑、复制、搜索、删除。草稿由设置对话框持有，切换分类仍保留。关闭设置会丢弃未保存草稿，页脚提示未保存状态。

仅保存 System Prompt 原文。不保存 User Prompt，不实现变量替换、条件模板、多预设拼接、业务默认预设或历史版本库。后续业务模块自行选择预设，并为每次任务提供消息。

最多 128 个预设；名称沿用 240 字节限制，备注最多 2000 字节，正文非空且最多 64 KiB（UTF-8）。允许换行、Unicode、前后空白；仅用 trim 判断空白正文，不改写文本。复制创建新 ID，之后独立编辑。

## 调用约定

请求新增可选 `system_prompt_id` / `expected_system_prompt_revision`。提供版本却没有 ID 时拒绝；不提供版本时解析当前保存版本。准备与发送之间希望防止变化的调用方应携带版本。

选择预设时，原始 messages 中的 system 或 developer 消息会被拒绝，避免悄悄叠加或覆盖指令。引擎加载预设、校验版本，将一条 system 消息插入最前，并重新校验总消息数和输入字节上限。不选预设保持原有标准消息行为。

快照 schema_version 升为 2，新增可空的 `system_prompt_id` / `system_prompt_revision`，实际原文固定在 `messages` 中。新增字段反序列化允许缺失，旧 v1 快照仍可读取。已准备的执行计划不再读取预设；随后修改或删除不影响正在执行的请求。引用已删除预设的新调用明确失败。

参数预设独立生效，不携带 Prompt。新建/编辑/删除使用 expected_revision；冲突不自动覆盖。删除不级联影响模型或其它配置。

## 传输与检查

复用已有适配器：Chat Completions / OpenRouter 为首条 `messages` system，Responses 为 `input` system 消息，Gemini 为 `systemInstruction.parts[].text`。本地预设 ID/版本和备注不会作为供应商报文字段发送。generate 与 stream 使用相同准备和编码链路。

“API 与模型 → 调用检查”可选 System Prompt 并输入一次性测试消息。预览调用本地 prepare，不请求供应商；显式点击发送后才调用上游。发送复用预览中的版本条件，避免预览之后配置变化时静默使用新正文。测试 User Prompt 不保存，离开页面即释放。

正文不会写入引擎日志；请求预览和完成快照按调用方显式请求返回内容，未来业务模块自行决定结果留存。

协议依据：[OpenAI Responses](https://developers.openai.com/api/reference/cli/resources/responses/methods/create)、[OpenRouter 消息格式](https://openrouter.ai/docs/api_reference/overview)、[Gemini System Instruction](https://ai.google.dev/api/generate-content#v1beta.GenerateContentRequest)。本地集成测试验证精确报文，不代表真实商业模型的权限、计费或回答质量已验证。
