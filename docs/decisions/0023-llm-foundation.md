# LLM 接入与调用基础层

日期：2026-09-16。状态：已实现；验收见 `docs/verification-llm-foundation.md`。

## 范围

应用级供应商连接、独立模型配置、远端模型目录、命名参数预设、参数描述与验证、标准消息调用、流式响应、取消及网络资源控制。设置入口位于现有“设置 → API 与模型”。

本阶段不实现 System Prompt / User Prompt 的保存、管理、变量渲染、拼接或业务模板，不读取工作集、不发布标注成果、不实现 Agent 工具循环。调用者直接提供准备好的消息和可选函数声明。设置中的调用检查仅发送固定的一次性消息，须点击才会调用供应商。

## 模块与依赖

- `studio-domain::llm`：连接、模型、参数描述、调用快照、消息、用量和结果类型，无 HTTP、数据库或 DTO 依赖。
- `studio-application::llm`：仓储、凭据、远端调用端口；配置管理、参数解析及调用准备。`LlmService` 是业务执行器使用的公共入口。
- `studio-llm`：Chat Completions、Responses、Gemini 原生编码与解析；OpenRouter 扩展；模型发现；HTTP 连接池、SSE、限流及 Windows 凭据适配器。
- `studio-storage::llm`：应用注册表的配置仓储；不使用项目偏好 JSON 作为供应商数据库。
- `studio-protocol::llm`：本机 DTO，单独映射领域对象，生成 OpenAPI 与 TypeScript。
- 引擎 `api/llm`：路由与宿主组装；调用 ID、活动请求与取消由宿主管理。
- SDK `client/src/llm`：配置接口及流式传输。设置功能仅调用 SDK。

新能力按职责组织目录；现有 `api.rs`、SDK `index.ts` 和各 crate 入口仅增加组装与导出。远程调用不进入同步 `Operator::row()`。

## 供应商、模型与协议

连接支持 OpenAI、通用 OpenAI 兼容服务、OpenRouter 和 Gemini。前三者使用 Bearer 凭据；Gemini 使用 `x-goog-api-key`。无凭据的本地服务也可使用。

模型配置的唯一键是连接 ID、远端模型 ID、协议。一个 OpenAI 连接可分别保存 Chat Completions 和 Responses 配置；OpenRouter 当前走 Chat Completions 加路由扩展；Gemini 使用原生协议。

远端目录和本地模型配置分表保存。刷新目录不会创建、删除或覆盖用户的模型配置；失败保留旧目录；连接修订不匹配时，晚到的刷新结果拒绝写入。目录显示获取时间和连接修订。手工模型不依赖目录或自动探测。

Gemini 处理 `nextPageToken`；支持 `has_more/last_id` 的兼容目录按游标继续读取。目录最多 100 页、10000 个去重模型；本地每个连接最多 1024 个模型配置、应用最多 128 条连接和 256 个参数预设。

## 参数语义

解析次序：应用协议默认值 → 模型配置 → 可选命名预设 → 本次覆盖。官方 OpenAI 的 `store=false` 是应用默认值，也进入调用快照。

省略键表示继承，键值 `null` 表示从结果中移除该参数，其他值表示覆盖。0 和 false 都是有效值。扩展对象整体替换，不进行隐含的深层合并。

通用字段只在语义可映射时统一。`max_output_tokens` 在 Chat Completions 中按服务类型或 `token_limit_field` 转换；Responses 输出格式映射为 `text.format`；Gemini 参数进入 `generationConfig`。OpenRouter、Gemini 专有字段有各自命名空间。未注册的字段在发送前拒绝；不允许扩展覆盖模型、消息、认证或流控制。

参数描述由后端提供，包含类型、范围、枚举、分类、支持状态与依据。能力有 supported、unsupported、unknown 三种状态，用户覆盖优先于与连接修订匹配的目录信息。缺少元数据保持 unknown；已知不支持明确拒绝，未知设置保留并生成诊断。配置保存检查结构，调用准备再检查最终层叠后的能力和互斥约束。

预设只绑定协议，不绑定业务或 Prompt；应用到不兼容服务时明确拒绝其专有字段。

## 调用契约

`prepare` 解析配置并返回无凭据的版本化快照和原生请求预览，不发网络请求。`generate` 返回完整结果，`stream` 返回 started、delta、completed 或 failed。completed 包含权威最终内容、用量、结束原因和同一次调用快照。

输入支持有角色的文本消息、图片、函数调用与函数结果；输出支持文本、上游推理摘要、拒绝与函数调用。标准接口不执行函数。图片内容由调用方准备；Gemini 接受内联 data URL 或供应商文件 URI，基础层不下载或转换项目图片。

Gemini 不支持独立 developer 优先级，不能通过拼接或降级伪装；该情况明确拒绝。Gemini 函数调用的 thought signature 保留；不提供跨供应商隐式会话迁移。音频、图片生成、供应商托管工具和远端会话管理未包含在本阶段支持范围。

缺失用量是 null，不按零计费。请求失败保留稳定错误码、HTTP 状态、上游请求 ID、重试建议和 outcome_unknown。上游错误正文、API Key、消息和图片不写入日志。

调用快照固定模型、连接、预设的修订及实际解析后的参数和消息；调用结果由未来业务模块决定如何持久化。快照不包含凭据和附加认证信息，也不代表远端模型版本或非确定性输出一定可复现。

## 执行、取消与重试

连接池按连接 ID 和修订复用，修订变化建立新客户端。引擎最多接受 128 个活动/排队调用，单连接并发 1–32；可配置最小请求间隔、连接超时、读取空闲超时、总时限和 HTTP(S) 代理。总时限包括排队与重试。所有队列、输入、响应及流帧都有上限。

不自动重试 500、网络结果不明或已经输出内容的流。唯一推理自动重试是用户配置的明确 HTTP 429，默认 0 次、最多 3 次，遵守有界等待；可能已受理的超时、取消和断流保留 outcome_unknown。

SDK AbortSignal 会同时结束本机读取并请求引擎取消。提前到达的取消记录短期保留，防止请求注册竞态。流消费者退出、客户端 dispose、设置调用检查卸载和引擎关闭都会释放相应调用资源。取消不保证供应商停止远端计费。

调用 ID 仅防止同时重复提交，不是跨重启的账单幂等键；已完成调用没有基础层自动恢复或重放。未来批任务应在现有任务层保存检查点和结果。

## 存储与凭据

应用注册表由 v2 升级至 v3，新增 llm_providers、llm_models、llm_presets、llm_catalogs。v0/v1 同样可升级。迁移沿用 SQLite 一致备份和事务流程；项目数据库格式保持不变。

供应商修改使用 expected_revision；删除仍有模型配置的连接会被阻止。所有项目共享本机连接，项目成果不包含 API Key。

Windows 凭据通过当前用户 DPAPI 加密，文件位于引擎数据目录的 `llm-credentials`。注册表仅保存引用，DTO 只返回 credential_set；密钥修改使用新引用，数据库写入失败时清理新凭据。其他平台尚无系统凭据实现，保存密钥时明确报不可用，不退回明文。

## 官方协议依据

- [OpenAI Chat Completions](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create)
- [OpenAI Responses 与 Chat Completions 的差异](https://developers.openai.com/api/docs/guides/migrate-to-responses)
- [OpenRouter 参数](https://openrouter.ai/docs/api/reference/parameters)及[路由参数支持](https://openrouter.ai/docs/guides/routing/provider-selection)
- [Gemini generateContent](https://ai.google.dev/api/generate-content)及[模型目录](https://ai.google.dev/api/models)

协议支持依据本阶段实际实现和验收夹具；未用真实账号验证任何特定商业模型的权限、额度或全部参数组合。
