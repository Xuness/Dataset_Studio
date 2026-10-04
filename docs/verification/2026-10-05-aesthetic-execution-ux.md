# 美学评审执行与使用流程修正验证

日期：2026-10-05。范围与兼容规则见 [ADR 0059](../decisions/0059-aesthetic-streaming-and-recovery.md)。

本轮实现与验收已完成。代码检查、完整集成流程和两套界面回归均通过。

## 范围

覆盖阶段级传输/恢复配置、SSE 原始回执及离线重解析、分层超时、同轮批次隔离、持久化有限重试、暂停/重启、异常筛选与候选定位、批量处理、归档/命名、当前阶段快照跳转、图片复核，以及固定复核水位的 Top N 预览和派生。

验证全部使用仓库内的合成图片、隔离项目和本地 HTTP 模拟供应商。没有调用商业模型，不能据此声称真实模型审美质量、生产吞吐或特定供应商的完整兼容性已验收。

## 验证入口

- `pnpm check`
- `pnpm test:integration`
- `node tooling/integration-aesthetic-execution.mjs`
- `node tooling/smoke-aesthetic-execution-ui.mjs <successful integration-aesthetic-execution run>`

## 已通过的检查

| 检查 | 结果与范围 |
| --- | --- |
| `pnpm check` | TypeScript、导入边界、生成契约、Rust 格式和 Clippy 全部通过；272 项 Rust 测试通过、0 忽略；424 项数据湖测试通过。 |
| 暂停修正后的代码复验 | 再次通过类型、Lint、契约、Rust 格式、Clippy、272 项 Rust 测试及客户端基础/媒体检查；未改动的数据湖代码不重复执行。 |
| `pnpm test:integration` | 最终完整运行的 28 个脚本全部通过；包含美学基础 10 项、离线分析 9 项、故障恢复 14 项、传输 6 项，以及下面列出的新执行和采样检查。 |
| `integration-aesthetic-execution.mjs` | 19 项通过：SSE/JSON、Chat/Responses/Gemini、尾帧用量、离线重解析、首包/空闲/总时限、同轮隔离、持久化重试、队列/暂停/重启、连接版本重绑定、固定水位预览/派生和阶段管理。 |
| `integration-aesthetic-sampling.mjs` | 4 组通过，共 212 次本地模拟调用；覆盖 145 图并发采样、暂停重启、比较连接、追加评审和细排模式。暂停时未发送的批次没有失败状态或候选锁。 |
| `smoke-aesthetic-execution-ui.mjs` | 7 组通过；浏览器未报告脚本异常。验证被锁候选定位、批量恢复、阶段快照上下文、梯队看图、执行设置/复制、归档恢复/结束确认及 Top N 预览和预算拦截。 |
| `smoke-aesthetic-ui.mjs` | 3 组通过；原批次证据、保护池、创建并冻结及草稿恢复流程保持可用。预算不足的创建场景明确选择小预算试跑。 |

截图人工检查覆盖执行设置、梯队证据、大图、异常候选及派生预览。模拟图片为人工合成色块，只验证加载和交互，不用于判断审美排序质量。

新增账本回归覆盖：历史未知结果迁移、阻塞候选可见性、自动重试持久化与单份证据、恢复时限到期、失败批次终结后的真实曝光、批量操作跨页幂等，以及元数据/执行设置不改写冻结标准。

集成过程中另修复了两个发送前的恢复问题：存储准入失败未生成网络 attempt 时保留队列；暂停取消本地图片读取时也保留队列。它们都不能记为需要付费重试的批次错误，也不能阻塞尚未发送的候选。

## 实现与验证对应

| 范围 | 实现入口 | 验证依据 |
| --- | --- | --- |
| 执行设置与冻结标准 | `crates/studio-application/src/aesthetic/execution.rs`、`crates/studio-engine/src/aesthetic/execution.rs` | 语义变化拒绝重绑定；网络参数修订保留配置摘要和已有证据。 |
| 原始流与时间预算 | `crates/studio-llm/src/recorded.rs`、`crates/studio-llm/src/transport/http.rs` | 确定性的首包延迟、心跳、部分流、用量尾帧、无终止事件、供应商并发等待。 |
| 持久重试与候选生命周期 | `crates/studio-engine/src/aesthetic/mod.rs`、`crates/studio-storage/src/aesthetic/recovery_policy.rs` | 每次发送的预算/attempt、暂停与重启、重试耗尽、终结旧批次、迟到结果及单份接受证据。 |
| 账本升级与展示计数 | `crates/studio-storage/src/aesthetic/` 下的 `schema_v8.sql`、`management.rs`、`tests/execution.rs` | 旧未知结果迁移、稳定局部编号、增量计数、批量跨页与幂等。 |
| 工作台与离线选择 | `apps/desktop/src/features/aesthetic/`、`crates/studio-engine/src/aesthetic/analysis.rs` | SDK 集成和两套界面回归；Top N 的并列/保护计数与固定复核水位派生一致。 |

## 本机证据

长期证据归档到 `.local/reports/aesthetic-execution-20261005/`，包含检查日志、隔离运行报告、截图、变更文件清单和一次真实项目的只读计数核验；这些文件不随仓库分发。临时项目的删除被本机自动审批策略拒绝，本轮保留对应目录，范围记录在 `cleanup-targets.json`；使用上述入口可以重新生成测试项目。

真实项目核验仅检查迁移版本、既有尝试/有效证据状态和增量计数是否对应明细。没有修改阶段执行设置，没有重试原有未知结果，也没有启动商业模型调用。

原有阶段采用新策略时，先在“执行设置”保存并关联当前连接，再在异常批次中明确安排重试，最后点击“开始评审”。如果模型、端点或请求参数已经改变评审标准，应复制配置创建新阶段。

## 证据边界

模拟服务可以确定性验证首包延迟、数据心跳、部分流、终止标记、用量尾帧、Retry-After、供应商并发排队和认证失败。真实上游是否发送心跳、多久返回首包、断开后是否继续执行与计费，仍由具体供应商行为决定。

恢复次数、请求总时限与阶段调用预算是运行边界；它们不保证图片覆盖、比较图连通或统计精度。未完成条件继续明确显示，不以暂缓、数值收敛或已结束请求代替排名质量确认。
