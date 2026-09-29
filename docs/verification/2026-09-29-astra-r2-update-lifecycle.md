# R2：更新任务生命周期修复验收

日期：2026-09-29。实施基线：`4f8b62686d6da0ada824781226240fd2403b552a`。范围：F5 原子命令，然后联合 F3 调度与 F4 取消回收。实现及兼容决定见 [0046](../decisions/0046-update-command-handoff-and-cancel-cleanup.md)。

状态：R2 实现完成，最终源码通过阶段验收。交付提交由本文件所在 Git 提交及本机证据 `summary.json` 固定；未推送远端。

## 实现和证据入口

| 缺陷 | 实现 | 回归证据 |
| --- | --- | --- |
| F5 | `commands.py` 中执行锁交接、归档回执核对和原子命令；`State.claim()` 原子领取 | resume/retry/replay 读旧状态后取消先提交；条目重置后故障整笔回滚；重复命令不增代次；取消与领取、完成竞争；旧媒体失败回执不会覆盖新重试 |
| F3 | `dispatch.py` 逐湖取得到期队首并持久记录轮转次序；`Runner.schedule()` 隔离异常 Future | A 积压 100/1000 项，B/C 同轮补位；A/C 保持活动时 B Future 异常，D 接替；B 到期重试重新进入；B/C 长任务未结束而 A 完成时，D 先于 A 的旧积压获得槽位；轮转次序跨重启保留；终止任务清理先于同湖后续积压 |
| F4 | v5 控制库持久清理责任；`cleanup.py` 实际持锁核对归档；`spool.py` 有界路径检查和回收 | 下载、正在编码、待发布、归档已提交/控制回执未落地、结束写状态等五个取消窗口；四个清理断点强制终止子进程再恢复；文件占用重试；非法文件、子目录、控制器归属和真实 Windows junction 拒绝；旧取消任务升级补回收 |

上述 Python 文件均位于 `services/lake-worker/src/studio_lake/updates/`；正式回归位于 `services/lake-worker/tests/test_update_lifecycle.py`。原有重启、暂停、网络续传、媒体错误和流水线测试继续参与统一检查。

10 MiB 共享配额实验先保留 8 MiB 取消暂存，使另一湖的 4 MiB 申请被拒；经过两个有界清理轮次，取消目录消失，新请求可以准入。260 个小收据用于保证确实跨过每轮 256 文件边界，不以清空内存预留代替实际文件回收。

HTTP/SDK 的 `integration-lake-updates.mjs` 通过隔离夹具追加三类任务：API 取消、引擎停止期间已取消、暂停保留。重启后验证不可恢复已取消任务、重复取消、清理完成回执、目录消失以及暂停文件仍在；范围均为本地空查询，无源站网络访问。

## 检查结果

| 检查 | 最终结果 |
| --- | --- |
| `pnpm contracts` / `contracts:check` | 生成并验证新增清理状态 DTO，TypeScript 契约一致 |
| `pnpm check` | exit 0；类型、边界、契约、rustfmt、Clippy、Rust workspace、客户端及 Python 测试全部通过 |
| Python 服务 | **169 passed，114.99 s**；其中新增 **30** 个生命周期参数化用例 |
| Rust 补充回归 | 新增 1 个 WAL 协议竞争/真正损坏的续租错误注入用例通过，原有续租期限与取消测试继续通过 |
| `pnpm test:integration` | exit 0；标准链路 **25 个脚本全部通过**，没有跳过更新 worker |
| 在线读取集成 | 9 组检查，包括四路视图/固定构建并发、更新与重启后的续页 |
| 更新 HTTP/SDK 集成 | 8 组检查，包括真实 worker 源码一致性、公开取消、重启清理与暂停保留 |
| 内置 worker 核验 | 实际安装的 **45 个文件**及 revision 与当前源码逐字节一致；覆盖故障注入构建恢复普通构建后的运行器 |
| 静态补充 | 修改的 Python 文件 Ruff、Git diff whitespace 和 6 份文档相对链接检查通过 |

最终完整检查日志为 `.local/logs/astra-r2-delivery-check.log` 和 `.local/logs/astra-r2-delivery-integration.log`。过程日志也归档，包括修正旧 schema 断言、将旧重试夹具设为实际待复核状态，以及契约更新后刷新 TypeScript 增量缓存的记录。

第一轮完整集成在已有 R1 在线并发分页用例出现一次 `locking protocol`；使用同一二进制单独重跑该脚本通过，不能据此认为竞争根因已经消失。根据 [SQLite 官方错误码定义](https://www.sqlite.org/rescode.html#protocol)，这是 WAL 开始事务时的锁竞争耗尽，不应报告来源格式损坏。本阶段补正 `studio-sources` 的错误分类为 `SOURCE_BUSY`，续租沿原有有界重试处理；新增故障注入验证该错误可重试而真实损坏不重试。未修改集成脚本以吞掉错误，也没有声称已经消除 SQLite 内部竞争或测得其自然发生概率。

端到端更新集成还发现一次构建缓存污染：此前隔离审查副本复用了同一 target 目录，普通构建的 worker 生成文件仍使用 `.local/test-runs/ar29/source/` 的绝对依赖与 include 路径。美学故障注入套件切回普通构建后，实际解出的 worker 仍只接受 v4；本地夹具升级至 v5 后，RPC 拒绝打开控制库。`studio-engine/build.rs` 已改成相对依赖跟踪，并通过当前 `CARGO_MANIFEST_DIR` 引用源码；更新集成新增逐文件字节与完整 revision 校验，避免仅凭源码测试或构建成功判定交付。旧副本和失败运行目录没有作为修复手段删除。

## 验证边界

全部新增实验使用 `.local/test-runs/` 下的隔离控制库和合成湖。固定交错由线程屏障控制，清理中断使用独立 Python 子进程与硬退出码 91；不估计这些竞争在正式负载下的自然触发概率。文件占用由确定性 PermissionError 注入，Windows junction 使用真实目录重解析点。

调度公平性实验执行真实控制 SQL 和 `Runner.schedule()`，Future 的完成/异常由测试控制，不发网络请求或处理 1000 次实际抓取。它证明队首选择、活动槽补位和轮转顺序；实际多湖端到端吞吐与长任务等待分布仍属于 R4。

没有修改正式湖、凭据或运行配置，没有启动正式抓取、付费评审或重启用户服务。任务详情文字与刷新条件经过类型/静态检查；本阶段未执行真实桌面 UI 操作。R3A（F6）、R3B（G1）和 R4 真实规模组合负载仍未实施。

归档、源码覆盖清单、日志与本轮临时目录清理结果集中于 `.local/reports/astra-r2-update-lifecycle-20260929/`，不随仓库分发。

必要证据归档后，已完成本轮 **83 个新建测试目录、25,528 个文件**的逐级清理，逻辑文件总量约 9.98 GB（不是实测卷空闲量）。删除前核对绝对路径、时间戳、文件清单、重解析点和进程引用；只逐个删除文件及空目录。此前审查/R1 被拦截的旧目录和复用的静态测试目录不在本轮删除范围内。保留的 `persisted-lifecycle-proof.json` 还直接核对了最终控制库：两项取消的清理均 complete，暂停任务未登记清理且暂存仍在，随后才删除整个隔离夹具。
