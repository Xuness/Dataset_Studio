# P2/P3：在线湖、分页查询与正式迁移

日期：2026-09-26。状态：P2/P3 与三湖正式迁移完成；P4 抓取管理尚未开始。

设计见 [ADR 0037](../decisions/0037-versioned-online-lakes.md)。基于 Studio 2ef123d 和 Store 8bdf237；原 Pro 附件保持原样。用户明确允许清理旧 Studio 项目及查询索引。

## 已有证据

- Store 转换可中断恢复，复制 normalized、raw 和 schema；raw 另存长度、SHA-256 与压缩内容。
- 故障测试覆盖元数据发布、缺图补齐与同帖换图；旧视图在中断时可读，重放幂等，永久租约限制回收下界。
- 隔离三湖覆盖四种跨湖排序、字面 Tag、更新前后详情、视图捕获、工作集/任务共享成员、排名和重启恢复。夹具移走 native DuckDB，确认消费者使用在线库。
- 浏览器验收覆盖点击元数据 Tag 打开分页视图，以及保存当前视图为工作集。
- 深页和空结果扫描验证有界工作量；独立成员 writer 不阻塞项目读者。

三湖正式 HTTP 抽样，每页 48 条、10 次暖读：

| 湖 | 浏览暖读 p95 | 元数据详情 | raw | 首次生成 256px 预览 | 预览命中 |
| --- | ---: | ---: | ---: | ---: | ---: |
| Danbooru | 23.1 ms | 7.0 ms | 6.2 ms | 664 ms | 4.6 ms |
| Yandere | 22.4 ms | 6.8 ms | 7.1 ms | 1,027 ms | 4.7 ms |
| Gelbooru | 22.9 ms | 6.5 ms | 6.6 ms | 957 ms | 5.8 ms |

Danbooru/Gelbooru 的 1girl 首页约 31/43 ms。Yandere 本次该词无命中，不把空结果耗时当成功召回性能。三湖各取一幅图测预览；“首次生成”指新的 Studio 预览缓存，不代表操作系统缓存被清空。样本有限，不视为任意条件或长期并发 SLA。

另在三湖各自持有未提交控制写事务时，预热的固定视图仍能连续返回 10 页，每页 16 条，耗时约 13–17 ms。写探针最终回滚，没有改动来源数据；此实验验证读取与写事务重叠，不代替几小时的真实抓取压测。

真实实验发现帖子深页的 NULL/OR 导致全局排序、候选排序提前求值导致热门 Tag 额外开销；均已修正，前后测量存入报告。

## 正式库位置和基线

| 湖 | 在线索引 | 图片和完整归档 | 基线 | 对象数 |
| --- | --- | --- | ---: | ---: |
| Danbooru | F:\Dataset\Danbooru | E:\AI\AI_Dataset\Danbooru | 11413 | 11,593,632 |
| Yandere | F:\Dataset\Yandere | E:\AI\AI_Dataset\Yandere | 143 | 1,154,033 |
| Gelbooru | F:\Dataset\Gelbooru | E:\AI\AI_Dataset\Gelbooru | 250 | 3,773,838 |

三湖均已通过七类表全量摘要、SQLite/FTS5 和关联检查，缺失对象、来源观察、当前观察、raw 引用均为 0；均已启用在线指针，并确认 served_seq 与归档头一致。合计 16,521,503 个来源内图像对象，18,452,579 条完整观察。图片包没有重写。

新库文件共约 101.84 GB，其中 Danbooru 81.88 GB、Yandere 4.42 GB、Gelbooru 15.54 GB。Danbooru 校验中实测发现无界文件映射会使 Windows 工作集过大，最终用 8 GiB 页缓存及 4 GiB 映射完成校验。

已按授权删除 4 个旧项目和 Studio 的 browse/identity、Rating、ranked、query 暂存及目录缓存，删除文件长度合计 154,826,280,329 字节（约 154.83 GB）。全局偏好、提供商、模型、预设、模型目录和 System Prompt 表逐表摘要保持一致，凭据目录保留。生产引擎已换为新二进制，项目列表为空，三湖预检通过。用户可以直接建立新项目。

Store 的原生生产者索引仍服务离线分析与归档工具。P2/P3 完成后，用户进一步要求统一物理位置，已将原 D:\Dataset\Danbooru 连同生产者索引、Python 环境及日任务工作区归并至 F:\Dataset\Danbooru，并在验证后移除 D 盘副本，见 [工作区迁移记录](2026-09-26-danbooru-workspace-relocation.md)。

## 证据与重复执行

本机日志在 .local/logs/online-upgrade-p23-20260926/；报告在 .local/reports/online-upgrade-p23-20260926/，均不随仓库分发。

- tooling/integration-online.mjs 与 online-fixture.py：隔离三湖和消费者契约。
- tooling/smoke-online-ui.mjs <online-fixture-directory>：浏览器分页及工作集。
- tooling/verify-online-lakes.mjs <lake-paths.json>：正式湖有界 API 抽样，不抓取网络数据。
- tooling/verify-online-writes.mjs <lake-paths.json>：短暂控制行写锁与读取重叠，最终回滚写入。

回归包括 pnpm check（含契约、类型、边界、Clippy、234 项 Rust 测试和客户端测试）、23 组已有集成脚本、新增在线集成、前端构建及浏览器验收。在线集成同时复核 MetaRecall v1/v2。Store 全量 223 项通过；随后新增分析水位联动由 9 项在线故障测试复核。具体执行轮次、失败原因及修正后的通过记录见本机报告。

归档了 324 份测试摘要、日志与截图，并清理 56 个本轮测试目录，约 4.05 GB。源码覆盖清单保存两仓库本轮变更文件的 SHA-256。仓库中的脚本会为下一次验证重新生成夹具，不依赖已清理的临时目录。

Python 夹具需要 DuckDB 和 APSW，可用 PYTHON 环境变量选解释器。本机 Store 使用 APSW/SQLite 3.53.4，Studio 捆绑 SQLite 3.53.2；Node 内置 SQLite 仅作只读测试观测，不承担发布。

本轮没有访问三站网络 API，没有运行完整湖排名，也不以微型故障测试替代持续数小时的生产抓取压测。复杂组合的全量扫描、HDD 冷图和解码成本仍存在。三站更新管理界面属于 P4。
