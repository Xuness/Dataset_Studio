# 2026-09-24：1000 万候选数量准入验收

状态：2026-09-25 验证通过。范围见 [ADR 0034](../decisions/0034-aesthetic-ten-million-admission.md)。本轮与此前尚未提交的 [邻近细排和 Davidson v2](2026-09-24-aesthetic-refinement.md) 一起提交。

这是数量准入与读写完整性验收，不是 1000 万真实图片端到端压测；不调用商业模型，不修改真实项目或已付费证据。

本机日志：`.local/logs/aesthetic-capacity-*`。补充证据与组批对照：`.local/test-runs/aesthetic-capacity-10m-20260924/`。最终归档位于 `.local/reports/aesthetic-capacity-10m-20260924/`，均不随 Git 分发。

## 已验证

- `pnpm contracts`、`pnpm check`、`pnpm build`、`pnpm test:integration` 通过：212 项 Rust 测试、10 项客户端基础检查、4 项媒体检查、22 个集成脚本。
- 工作流 UI 9 项通过，主视口 2560×1440，保留较小视口回归；原采样默认及 v2 可选入口保持既有行为。
- 真实 SQLite 工作集 10000000 条成员预检通过，balanced、adaptive、refine、refine_balanced 均通过；添加第 10000001 条后，预检及创建均拒绝，没有写入评审创建意图或增加模型请求。对应 `.local/test-runs/aesthetic-recovery-1790265824283/report.json`。
- 排名、分量与复核边界允许旧百万范围以上的有效参数，拒绝超出 1000 万候选范围的 ordinal；最大候选编号为 9999999，名次上限为 10000000。
- 20 万真实候选账本行和总序列化大小超过 32 MiB 的诊断成功分块发布；候选与检查点读取覆盖最后一条记录，单次 writer 负载没有突破原限制，末尾候选可被正常领取。
- 暂存槽位未发布时不可领取；取消、引擎重启后可清理重算。替换取消计划只清理未发布暂存，已发布摘要、接受证据和调用计数保持不变。
- 组批等价性：2080 图，balanced/adaptive/refine × perfect/mixed × 两个种子，共 12 对试验；优化后的生产实现与原参考实现的逐批证据哈希和最终排名指标全部一致。对应补充证据目录 `parity/report.json`。

## 边界

没有执行 1000 万图片的完整冻结、排名拟合、图像传输或付费评审压测；真实 SQL 成员边界测试与分块发布测试不能代替该容量验收。每批图片数量、每图曝光上限、用户调用配置、并发、传输、writer 队列和结果不明重试规则均未放宽。旧的 150 万实验报告和原始哈希保留原状。
