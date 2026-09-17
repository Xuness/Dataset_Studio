# 使用 LLM 基础层

相关决定：[0023](../decisions/0023-llm-foundation.md)、[0024](../decisions/0024-system-prompt-presets.md)。提供 System Prompt 预设的保存和按次引用；User Prompt 由任务传入。

## 前端与 SDK

功能模块接收应用注入的 `StudioClient`，通过 `client.llm` 调用。不要导入 Tauri 或自行请求供应商。

```ts
const providers = await client.llm.providers.list(signal);
const models = await client.llm.models.list(provider.id, signal);
const parameters = await client.llm.models.parameters(model.id, signal);
const systemPrompts = await client.llm.systemPrompts.list(signal);
const systemPrompt = systemPrompts.items[0]; // 实际由业务模块让用户选择，也可不选。

// preparedMessages 由业务模块准备；基础层不查找工作集或读取图片。
const input = {
  model_id: model.id,
  expected_model_revision: model.revision,
  expected_provider_revision: provider.revision,
  system_prompt_id: systemPrompt?.id ?? null,
  expected_system_prompt_revision: systemPrompt?.revision ?? null,
  messages: preparedMessages,
  tools: [],
  overrides: { temperature: 0 },
};

const preview = await client.llm.prepare(input, signal);
// preview.snapshot：版本、参数和能力提示；preview.native_request：无认证信息的报文。

const result = await client.llm.generate(input, signal);
// 或使用独立的一次流式调用：
for await (const event of client.llm.stream(input, signal)) {
  switch (event.type) {
    case "started": break;
    case "delta": onDelta(event); break;
    case "completed": onComplete(event.response); break;
    case "failed": onFailure(event.error); break;
  }
}
```

`generate` 与 `stream` 各自发起一次请求，应按需要选择一个。流中 delta 仅是增量展示，completed 内的结果才是权威最终内容。流失败由 failed 事件表达；本机网络、解析和连接失败可能抛出异常，调用方也应捕获。

`prepare` 不保存执行任务。后续通过 SDK 发送时重新解析配置；expected revisions 可阻止配置在两次操作之间被修改。调用 ID 可显式传入，也可由 SDK 生成，只用于本次请求与取消。

AbortSignal 会取消本机传输并通知引擎；退出异步迭代器也会发送取消。业务后台任务应在引擎内调用服务，避免其生命周期依赖前端组件。

## 管理接口

- `providers.list/save/remove`：全局连接与凭据。save 的 API Key 是写入字段，读取仅返回 credential_set。
- `models.list/save/remove/parameters`：独立模型配置及能力描述。
- `models.catalog`：读取已有目录；`refreshModels(providerId, revision, signal)`：显式从供应商获取目录。
- `presets.list/save/remove`：命名参数预设。通过 invocation 的 preset_id/expected_preset_revision 应用。
- `systemPrompts.list/get/save/remove`：独立的 System Prompt 预设，包含名称、备注和原文。通过 system_prompt_id/expected_system_prompt_revision 按次引用，与参数预设无关。
- `parameters(protocol, kind)`：没有保存模型时查询协议参数描述。
- `cancel(invocationId)`：幂等取消当前调用，不触发重试。

配置写入传 expected_revision；新建为 0，更新/删除使用读到的版本。Revision conflict 后重新载入并让用户决定，不自动覆盖。

选择 System Prompt 预设时，`messages` 只传任务消息，不能同时传 system/developer。引擎将预设原文解析成首条 system 消息，并把 ID、版本及实际消息固定在快照中。不选预设时仍可直接传入原有标准消息；不会自动套用默认预设。

`prepare`、`generate`、`stream` 使用同一套解析逻辑。准备与发送之间如需固定预设，请传 `expected_system_prompt_revision`；版本不匹配或预设已删除会在请求供应商之前报错。已得到的 Rust plan 不会因预设随后编辑或删除而改变。接口不进行变量替换或业务模板拼接，User Prompt 不写入配置库。

参数键缺失为继承，null 为不发送，具体值为覆盖。协议未定义字段直接拒绝；如需支持新字段，扩展后端参数描述与对应适配器并添加协议测试，不在 UI 拼接原始报文。

## Rust 调用方

宿主组装并注入 `studio_application::llm::LlmService`。

```rust,ignore
// prepare 和配置仓储是有界的同步数据库操作，异步宿主应放在阻塞执行资源中。
let plan = service.prepare(request)?;
let snapshot = plan.snapshot.clone();
let cancel = studio_application::llm::LlmCancellation::default();
let result = service.generate(plan, cancel).await;
```

`LlmInvocationPlan` 包含仅供后端使用的连接配置，不能作为前端 DTO 或项目成果序列化。需要持久化的是不含密钥的 snapshot 及实际结果；业务任务可以另存输入/输出大对象引用，避免每行重复保存图片数据。

任务编排、输入固定、逐项恢复和成果发布使用已有任务与成果边界。基础层处理单次请求的网络执行，不创建项目任务，不根据业务错误重试整批数据。

## 增加协议或参数

1. 在 domain 定义必要的稳定语义，保持业务无关。
2. 在 application 增加参数描述、兼容和覆盖规则。
3. 在 `studio-llm/protocols` 实现编码、完整响应、流响应和目录读取；供应商扩展归入 `providers`。
4. 更新本机 DTO 时运行 `pnpm contracts`，不手工编辑生成文件。
5. 用本机 mock 验证精确报文和失败边界，再由使用者显式发起真实模型检查。

当前支持文本/图片输入及函数调用封装，音频、图片生成、供应商托管工具、远端会话和多平台系统凭据需独立扩展。真实模型的可用性和参数支持以相应账号、端点及模型为准。
