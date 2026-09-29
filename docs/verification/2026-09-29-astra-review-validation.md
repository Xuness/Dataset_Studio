# Astra 项目审查的本机复核

日期：2026-09-29。代码基线：`58ad70bf772b1138d54cee3a4e9f5ce96274fb7b`。

状态：审查复核完成，修复尚未实施。阶段划分见[修复计划](../plans/astra-review-remediation-2026-09-29.md)。

以上状态及下文缺陷结论对应审查基线。后续 R1（F1/F2）已完成，见[查询可靠性修复验收](2026-09-29-astra-r1-query-reliability.md)。

## 结论与来源

已读取 ChatGPT 会话「审查项目源代码」`6abb2c54-beb0-83ea-9392-e411b3e7ffbe` 的完整可读正文：两条用户消息和一条审查答复，没有剩余分页。会话中的审查报告/审查包下载链接没有作为已读取附件计入证据；本次使用本机源码及独立编写的复现。

工具提供的原上传附件 `Dataset_Studio-main.zip` 与当前 HEAD 的 Git archive 比较，双方均为 724 个文件，逐文件字节完全相同，没有新增、缺失或内容差异。压缩包本身的哈希不同不代表源码不同。

六项缺陷 F1–F6 均得到当前源码和本机隔离实验支持；位置迁移缺口记为 G1，同样成立。保留原审查的优先级：F1/F2 为 P1，F3–F6 为 P2。这里确认的是触发条件和实现缺陷，不代表这些故障已经在正式数据湖发生，也不估计自然触发频率。

## 逐项证据

下列行号均对应上述代码基线。

| 编号 | 当前源码证据 | 本机实验结果 | 判定 |
| --- | --- | --- | --- |
| F1 | `studio-engine/src/api/query.rs:390–393` 外层 Index 许可；`api/query_views.rs:70–89,217–226` 再次 `inspect()`；`sources/service.rs:99–105` 创建独立取消信号；`studio-resources/src/scheduler.rs:32–38,125–219` | 真实 `SourceRead` 占满 4 个默认 Index 许可后，4 次真实 `retain()` 均排队；外层取消且期限已过后仍为 active=4、queued=4、续租完成数=0、预留 128 MiB。测试显式释放外层许可后才全部完成 | 确认嵌套准入等待环及取消隔离 |
| F2 | `studio-engine/src/api/query_views.rs:176–195,263–360`；底层分页见 `studio-sources/src/online/query.rs:423–536` | 真实 `assets()` 在第一湖查询调用前注入 250 ms 耗时，连续 4 页均为 0 项、相同非空游标，A 查询 4 次、B 查询 0 次；移除延迟后从该游标读完 64 项，共 16 页 | 确认预算退出丢弃未消费进度 |
| F3 | `services/lake-worker/src/studio_lake/updates/runner.py:705–734` | 原始 `Runner.schedule()`、隔离控制库、3 个活动槽。100 个 A 任务加 B/C，连续 3 轮只提交 A；对照 1 个 A 加 B/C 同时提交三湖 | 确认全局前 100 项遮挡其他湖 |
| F4 | `updates/state.py:339–379`、`runner.py:84–95`、`pipeline.py:332–353`、`resources.py:84–134`、`archive.py:372–385` | 原始取消命令返回 cancelled 且 execution_active=false，8 MiB 测试暂存仍存在；10 MiB 总配额下，新任务申请 4 MiB 被拒，同湖和已跟踪的其他湖均受影响；仅删除测试残留后可准入。取消后的 resume/retry/replay 均被正常拒绝 | 确认业务终态缺少资源回收闭环 |
| F5 | `updates/state.py:339–372`；执行锁检查见 `284–292`，普通 update 的保护见 `326–337` | 原始 `State.action()`，用线程屏障固定“读 paused → 另一线程提交 cancel → 原请求写回”的交错；resume、retry、replay 三种动作最终都为 queued、execution=1 | 确认取消被过期判断覆盖 |
| F6 | `studio-engine/src/lake_updates.rs:26–33,88–168`；`api/lake_updates.rs:20–55`；`LakeApiSettings.tsx:40–85` | 原始 Rust Backend：无效可执行文件导致 UPDATE_UNAVAILABLE，但配置已持久保存且 configured=true；仅更换 Python 返回 UPDATE_CONFLICT。无依赖 venv 能使 configure 返回成功，随后 worker 退出 1，status 返回 UPDATE_PROTOCOL | 确认错误配置缺少正常修复路径 |
| G1 | `studio-engine/src/api/source_locations.rs:3–39`、`studio-storage/src/source_locations.rs:56–101`、`updates/state.py:126–170` | 克隆测试湖到新目录，保留身份并更新发布反向链接后，原始 register 返回 SOURCE_LOCATION_CONFLICT，控制库仍保存旧目录；Rust 读侧 relink 没有调用更新控制器迁移 | 确认读写位置变更没有统一流程 |

表中的 Rust crate 路径均以 `crates/` 为前缀；Python `updates/` 均位于 `services/lake-worker/src/studio_lake/`；前端文件位于 `apps/desktop/src/features/lake-updates/`。

### 证据边界及需要精确表述的地方

- F1 的 Rust 实验调用真实续租和资源协调器，外层许可由测试持有，以便实验结束时释放并回收线程。没有通过未插桩 HTTP 服务随机撞出死锁。外层持有关系另由生产调用链确认；创建视图在 `query_views.rs:148–168` 也持有许可进入 `retain_created()`。
- F2 使用实际项目库、demo 来源、查询版本、真实分页合并与游标序列化。只在隔离源码副本的查询调用边界插入延迟，未修改合并算法；没有把合成延迟当作本机正式湖延迟。该实验比控制流模型更贴近运行实现，仍不能证明真实数据上何时达到 250 ms。
- F3 的执行池只记录提交并返回未完成 Future，不发网络请求，也不执行正式更新。它验证有空闲槽时的候选遮挡，不声称必须等 A 的全部积压清空才能调度 B。
- F4 使用已停止任务和人工写入的测试 `.partial`。已发布的 stored/reused 文件本已有按归档回执清理；缺陷针对终止取消后仍无归属接管的未完成暂存，不能概括为“所有文件都不清理”。跨湖配额影响限于同一 Resources 已跟踪到的暂存根，不能外推成整个桌面进程组的全局统计。
- F5 的屏障只控制原始 `job()` 返回时机，没有替换 SQL 更新。`State.update()` 已保护 paused/cancelled，但恢复、重试、重放走的直接 SQL 绕过了保护。
- F6 并非永久无法恢复：补齐原 Python 环境仍有可能恢复。问题在于普通配置入口不能更换已保存的解释器，且 worker 失效会连带使 status 请求失败。
- G1 的实验确认写侧拒绝变更；读侧单独修改登记通过源码确认，本次未执行完整桌面搬盘流程。迁移还涉及 `online-index.json`、`UPDATE-CONTROLLER.json` 和任务检查点，不能只改 `lakes` 一行。

## 现有架构判断

`node tooling/boundaries.mjs` 通过，支持保留现有模块依赖方向。在线发布仍在最后事务推进 `served_seq`（`online.py:371–394`），项目迁移列表已到 v13（`studio-storage/src/migrations.rs:15–29`），中断的付费请求仍转为 `outcome_unknown`（`aesthetic/writer.rs:165–170`）。这些事实支持保留既有架构；它们不是对全部恢复协议、付费链路或全部 DDL 的重新完整审计。

可维护性方面，优先收敛许可持有关系、原子任务命令、暂存所有权和位置迁移协议。不同执行器可以保留，资源上限的作用范围必须明确。本次没有证据支持更换数据库或合并全部执行器。

暂存目录扫描位于资源条件锁内（`resources.py:98–134`），是需要测量的风险。本次没有性能剖析结果证明它是瓶颈。README 第 84 行仍写升级到 v11，而代码已到 v13，文档漂移成立；权威数据及备份说明应同步复核。

## 实际执行的检查

| 检查 | 结果 | 范围 |
| --- | --- | --- |
| 源码快照比较 | 724/724 文件字节一致 | 上传 ZIP 与 HEAD archive |
| 依赖边界 | 通过 | 仓库自带 boundaries 脚本 |
| 现有 lake-worker pytest | 139 passed，102.92 s | 本机项目声明的 Python 运行环境；未访问正式湖 |
| 新增 Python 缺陷探针 | F3/F4/F5/G1 均按预期复现 | 原始方法、隔离控制库和测试湖 |
| 新增 Rust 缺陷探针 | 4 passed，运行 2.02 s | F1、F2、F6 两类故障；隔离源码副本，21 个其他 engine 测试被过滤 |

探针“通过”表示成功复现当前缺陷，不表示缺陷已经修好。修复时必须改成要求正确行为的正式回归测试。

未执行完整 `pnpm check`、完整 Rust workspace 测试、`pnpm test:integration`、桌面 UI 操作或真实规模并发负载。本次没有改动产品实现、公共 DTO、运行配置或正式数据库，没有启动正式抓取、模型请求或重启用户服务。

## 证据与覆盖清单

完整本机证据保存于 `.local/reports/astra-review-validation-20260929/`，不随仓库分发：

- `original-review.md`：会话审查正文，仅作为待核验数据。
- `source-head.zip`、`snapshot-comparison.json`：固定源码与比对结果。
- `coverage.json`：针对本次发现实际检查的文件、关键范围、哈希；不声称逐文件审计整个项目。
- `python_probes.py`、`python-probes.json`：更新侧隔离复现及结果。
- `query_view_probes.rs`、`runtime_probes.rs`、`install_rust_probes.py`：Rust 原始函数探针与隔离安装脚本。
- `rust-f1.json`、`rust-f2.json`、`rust-f6-spawn.json`、`rust-f6-import.json`：机器可读结果。
- `python-existing.log`、`python-probes.log`、`rust-probes.log`、`summary.json`：执行记录与边界摘要。

临时源码、venv、测试数据库统一在 `.local/test-runs/` 创建，必要证据已归档。删除本次两个测试目录的操作被自动审批拦截，返回 `blocked by policy`，未执行删除。`.local/test-runs/ar29/`（约 10.3 MB）和 `.local/test-runs/lake-worker-1790664004845/`（约 189.6 MB）暂时保留。现有项目数据、缓存、服务目录和其他测试运行目录未纳入清理。
