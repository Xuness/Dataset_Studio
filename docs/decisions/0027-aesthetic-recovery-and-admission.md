# 0027：评审创建恢复、存储准入与候选处置

日期：2026-09-19。范围：整体改进路线图 R0/R1，补充 ADR 0025/0026。项目数据库升级到 **v12**，评审账本升级到 **v3**；项目 manifest 版本不变。

## 创建意图与双库恢复

项目事务同时保存完整、无凭据的冻结配置、请求、工作集总量及工作集/来源保护引用。评审账本随后幂等创建 stage，再把意图标记为已建成；建成确认完成前不能启动执行器。创建与放弃共用项目写入锁，避免放弃与补建互相覆盖。

意图状态含义：

| 状态 | 含义 | 缺少 ledger stage 时 |
| --- | --- | --- |
| `pending` | 冻结意图已提交，尚未确认建成；不能收费 | 核对上游、补建 stage 和引用，恢复为暂停 |
| `materialized` | 已确认建成，可能已存在付费尝试 | 要求恢复账本，不能补建空记录 |
| `abandoned` | 未建成意图被明确放弃 | 保留墓碑，不重建，不保留输入引用 |
| `cancelled` | 已建成阶段被取消，账本可能仍含付费历史 | 要求恢复账本；取消不等于删除付费证据 |

同键请求必须与原请求一致。终止键返回 `OBJECT_REMOVED`；不同请求复用键返回 `IDEMPOTENCY_CONFLICT`。两个数据库的冻结配置和总量不一致时停止对账。恢复/同步会补齐仍需要的输入保护，不把“账本暂时缺少 stage”解释成允许释放来源。

v11 历史项目可能只有引用，没有冻结配置。若 ledger stage 存在，使用其真实冻结配置补充意图；若 stage 缺失，保留保护并返回 `EVALUATION_CREATION_INCOMPLETE`。用户可通过 `abandonCreation` 明确放弃未建成键后重新创建。不能按当前模型/模板伪造历史冻结配置。整个账本文件缺失且存在已建成或无法证明未建成的历史记录时，仍报 `EVALUATION_MISSING`。

## 发送准入与故障域

引擎关闭和存储健康分别记录。最终发送准入在上传等待之后执行；共享互斥屏障覆盖健康检查和 attempt 的持久提交，关闭准入与该提交点互相序列化。越过该点的请求视为在途，即使供应商限流还让其等待。阶段暂停通过同一个 ledger writer 的状态事务与 attempt 事务排序，后到的 attempt 不能越过暂停状态。

| 故障 | 准入范围 | 恢复方式 |
| --- | --- | --- |
| SQLite BUSY/LOCKED、队列条数或字节满 | 当前账本 | 保留返回，退避重试落盘，写探针成功且待保存返回排空后恢复 |
| SQLite FULL | 相同卷的账本 | Windows 使用实际卷 GUID；网络卷使用卷根；释放容量后同样探测并排空 |
| IOERR/无法打开存储、通用 IO 错误 | 保守暂停所有账本 | 无法证明故障只影响一个卷；故障账本恢复后解除 |
| 账本损坏、writer 退出 | 当前账本，标记需人工恢复 | 排空能保存的返回，重新打开/重启并校验；损坏库需恢复备份 |
| 引擎 shutdown | 全部 | 存储探针不能撤销关闭状态 |

归一化返回和明确失败都走同一有界保留/重试路径，继续占用原有请求内存许可。重试只针对本地持久化；不会调用模型。COMMIT 后丢失确认的重试仍按原 attempt 幂等。一个返回保存成功不能提前解除其他返回仍未保存的故障。

阶段收尾的 ledger 状态提交与项目引用同步也会重试临时写入错误；既有完成、取消或暂停状态不会因重复收尾而退回运行/待处理。这样一次收尾 BUSY 不会留下没有执行器的 `running` 阶段。

指标新增 `dispatch_health`、`storage_error_code`、`retained_outcomes`。请求许可/保留字节仍为执行器全局指标，健康状态与当前项目及共享故障域相关。普通打开账本先做 `quick_check`；迁移还校验计数及外键。队列满与 writer 已断开使用不同错误码。

若存储一直不可用而用户关闭引擎，进程内尚未提交的返回无法保证保存；关闭会明确记日志并结束，重启后已有 `sent` 记录进入 `outcome_unknown`。不会通过自动再次收费弥补。完整原始 HTTP 回执及项目恢复包属于 R2。

## 候选处置与计数

候选状态为 `active / needs_review / rejudge / excluded`，保留具体原因。接受观察里的不可评判不增加有效曝光；只有至少两张可判断图片形成比较时，才增加对应候选的曝光。单候选 Rating 或没有可用比较对手时，不购买无意义的单图相对比较。

`decideCandidate` 只处置已经暂停、无未完成批次占用的待复核候选，按处置键追加审计记录：

- `exclude`：显式排除，停止为该候选安排比较；不修改旧 observation、usage、提名或曝光。
- `rejudge`：重新进入派发；生成新的逻辑批次，优先使用尚需曝光的同 Rating 候选，必要时加入已达目标曝光的对照图。对照图的实际有效曝光仍累计，总调用预算照常约束所有尝试。Rating 缺失/冲突不允许跨 Rating 重评。

已接受的原批次不可按 retry 覆盖。追加重评需要先做候选处置，再显式 start；处置接口本身不调用模型。正常候选、明确排除和未决候选的阶段计数通过同事务触发器增量维护，避免每个批次都扫描全阶段。

| 字段 | 分母和含义 |
| --- | --- |
| `total / frozen` | 创建时固定的总量 / 已冻结行数 |
| `eligible` | 冻结时 Rating 明确的数量，之后不改变这个历史计数 |
| `comparable` | 已有有效曝光、Rating 明确且未被显式排除的候选数量；不代表图连通或排名已收敛 |
| `excluded` | 当前明确排除的候选数量 |
| `unresolved` | 已冻结且未排除，但曝光未达目标、被批次失败阻塞或仍待处置/重评的数量；不含尚未冻结行 |

所有未决候选及未完成批次处理完后，阶段进入 `completed` 或 `completed_with_exclusions`。后者不等于原冻结全集都有可用排名。R1 排除用于评审推进；旧离线快照和原有全池排名分母保持不变，排名总体/筛选语义在 R3 处理。保护提名仍为独立观察事实。

v1/v2 账本升级时，从真实 accepted observation 的不可评判记录补出 `needs_review`，不把所有 blocked 候选误认为不可评判。迁移只扫描一次批次建立临时索引，再按候选主键更新；临时索引可丢弃。既有 accepted evidence、raw normalized receipt 和快照不改写。

## 能力、冻结模板与请求摘要

`capabilities` 和 `preflight` 的版本为 1；创建与最终 attempt 准入共用整个阶段 **1,000,000** 候选上限，离线估计器引用同一常量。1,000,001 在收费前拒绝。预检返回机器可读拒绝原因和输入摘要；可在 create 中带 `expected_input_version`。创建会重新查询输入，并在项目事务里再次核对成员总量、来源描述及项目输入摘要；冻结图片时继续核对外部来源版本。

预检当前覆盖容量和输入版本，不代表图片存在性、编码后字节大小或货币预算已验收。每张 2 MiB、整请求 12 MiB 的发送前检查保持生效；字节预算组批和图片问题隔离在 R2。

新阶段的 config 版本为 2，记录模板、业务 schema、原生编码器、采样器版本和 input version；实际发送保留阶段存储的全部文本消息，并把图片标签和图片追加到其冻结 User 消息。旧阶段优先使用其原有消息；缺少可证明的冻结 User 文本或执行版本不受支持时，明确拒绝继续。

每个新 attempt 在发送提交时保存 `semantic_request_hash`，绑定 config hash、固定批次/图片摘要和实际原生请求体的 SHA-256。账本只保存最终摘要，不保存图片和凭据；旧 attempt 的摘要为 `null`，不补造。HTTP JSON 解析失败保留供应商 request ID；完整原始 HTTP 字节保全仍待 R2。

## API 与工程入口

项目 API 前缀 `/v1/projects/{project_id}/aesthetic`：

| 新入口 | SDK |
| --- | --- |
| `GET /capabilities` | `client.aesthetic.capabilities(pid)` |
| `POST /preflight` | `client.aesthetic.preflight(pid, request)` |
| `POST /stages/{id}/candidates/{ordinal}/disposition` | `client.aesthetic.decideCandidate(pid, id, ordinal, decision)` |
| `POST /creation-intents/{id}/abandon` | `client.aesthetic.abandonCreation(pid, id)` |

公开协议使用显式字段映射，OpenAPI/TypeScript 由 `pnpm contracts` 生成。纯生命周期、容量和模板规则在 application；项目引用/意图与 ledger 事务在 storage；engine 负责准入屏障、资源许可、媒体和运行器。

`test-faults` 是显式 Cargo feature，仅供测试二进制读取隔离故障文件，普通构建的端口为空操作，不开放 HTTP 注入接口。新增集成脚本完成后恢复普通引擎构建。测试及预算见 [R0/R1 验收记录](../verification/2026-09-19-r0-r1.md)。
