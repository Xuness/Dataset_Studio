# System Prompt 预设验收

日期：2026-09-17。范围与接口见 [0024](decisions/0024-system-prompt-presets.md) 和 [LLM 接入说明](architecture/llm-integration.md)。

## 使用入口

- 设置 → System Prompt：新建、编辑名称/备注/正文、复制、搜索、删除。切换设置分类保留草稿，成功保存后重启仍有效。
- 设置 → API 与模型 → 已保存模型的调用检查：选择 System Prompt 预设、填写仅本次使用的测试消息；先预览，再按需发送测试请求。
- SDK `client.llm.systemPrompts` 提供管理接口，调用请求可引用预设 ID 与版本。User Prompt 随本次 messages 传入，不写入配置库。

## 自动化结果

- `pnpm contracts`：OpenAPI 与 TypeScript 从后端生成。
- `pnpm check`：类型、导入边界、ESLint、契约、格式、Clippy、149 项 Rust 测试及 SDK 测试通过。
- `pnpm test:integration`：17 组集成测试通过，其中既有 LLM 基础层 12 项，新增 System Prompt 10 项检查。
- `node tooling/smoke-llm-ui.mjs`：11 项界面检查通过，无页面运行错误；包含并发编辑冲突、重新进入设置获取最新版本、预览版本固定和发送验证。
- `pnpm build`：前端生产构建通过。

协议覆盖官方 OpenAI Chat Completions / Responses、通用兼容服务 Chat Completions / Responses、OpenRouter Chat Completions、Gemini 原生，共六种组合。分别断言 prepare 的原生预览、generate 的真实 HTTP 请求和 stream 的真实 HTTP 请求；验证 system 角色、User Prompt 独立传递以及中文、emoji、CRLF/LF、空白和占位符字面量原样保留。本地 ID、版本和备注不作为上游字段传输。

异常覆盖空正文、UTF-8 字节上限、禁止保存 user_prompt 字段、过期修订、缺失预设、System/Developer 冲突、加入预设后的消息数和总输入上限。所有拒绝均在发送上游之前发生。验证参数预设与 System Prompt 可独立组合，未选预设时原有消息接口仍有效。

验证复制后的独立编辑、调用开始后修改/删除预设不改变已有快照、重启后正文和修订保留。注册表 v0/v1/v2/v3 升级至 v4、升级前备份、失败保留旧版本、重复启动不再次升级均由存储测试覆盖；现有 LLM 配置行原样保留。旧 v1 调用快照仍可反序列化。

## 证据与边界

验收报告、最终截图和日志归档于 `.local/reports/system-prompts-20260917/`。原始命令输出位于 `.local/logs/system-prompts-*.log`，一次性测试目录归档后清理。

所有上游测试使用本机 mock 服务，没有真实商业 API 调用。上述结果证明 SDK、存储、引擎及原生报文传输链路，未验证特定账号的模型权限、实际计费或模型是否遵循具体 System Prompt。Prompt 不执行变量替换，任务级模板和业务工作流仍由后续模块负责。
