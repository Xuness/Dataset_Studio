# 文档导航

使用方式、当前能力与限制见[仓库说明](../README.md)，开发命令和检查入口见[工具导航](../tooling/README.md)。

本目录保存工程说明、设计依据和验收摘要。计划与历史记录中的版本、日期和“当前”均对应文档当时的基线；判断现状时，以仓库说明、最新模块接入文档和后续架构决策为准。

## 当前模块接入

数据湖更新：[工作台](decisions/0039-lake-update-workbench.md)、[内置服务与图片配方](decisions/0040-owned-lake-worker-and-image-recipes.md)、[运行环境与保存策略](../services/lake-worker/README.md)。

下载调度升级：[流水线与共享参数](decisions/0041-lake-download-pipeline.md)、[实施计划](plans/lake-download-pipeline-2026-09-27.md)、[2026-09-27 验收](verification/2026-09-27-lake-pipeline.md)。

下载恢复：[图片断点与后台自动接续](decisions/0042-resumable-lake-transfers.md)、[2026-09-28 验收](verification/2026-09-28-lake-resume.md)。

数据湖来源：[统一接口与调度](decisions/0035-source-registry-and-dispatch.md)、[Yandere/Gelbooru 验收](verification/2026-09-26-multibooru.md)、[多湖交互与查询缓存补齐](verification/2026-09-26-multibooru-followup.md)。

在线架构升级：[阶段计划](plans/数据库架构审查升级-2026-09-26/README.md)、[发布与预览并发边界](decisions/0036-online-read-path-preparation.md)、[在线湖与固定成员协议](decisions/0037-versioned-online-lakes.md)、[P1 验收](verification/2026-09-26-online-upgrade-phase1.md)、[P2/P3 迁移验收](verification/2026-09-26-online-upgrade-p23.md)、[Danbooru 工作区归并](verification/2026-09-26-danbooru-workspace-relocation.md)。

| 主题               | 入口                                                                                                                                                                                                                                             |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| LLM 调用与模块集成 | [后端和 SDK 接入](architecture/llm-integration.md)、[LLM 基础层](decisions/0023-llm-foundation.md)、[System Prompt 预设](decisions/0024-system-prompt-presets.md)                                                                                |
| 美学评审与离线统计 | [总体导航](plans/aesthetic-ranking/README.md)、[第一阶段评审](plans/aesthetic-ranking/phase-1-implementation.md)、[第二阶段后端与 SDK](plans/aesthetic-ranking/phase-2-backend.md)、[离线统计决策](decisions/0026-aesthetic-offline-analysis.md) |
| 元数据排名与浏览   | [MetaRecall v2](decisions/0018-metarecall-v2-metadata-ranking.md)、[持久排名索引](decisions/0017-persistent-ranked-scope-indexes.md)、[榜单位置锚点](decisions/0022-ranking-position-anchors.md)                                                 |
| 查询缓存与项目清单 | [固定查询依赖和缓存清单](decisions/0021-fixed-query-dependencies-and-cache-inventory.md)、[缓存分层与设置](decisions/0008-cache-tiers-settings.md)                                                                                               |

美学评审与离线统计后端已接入；[工作台前端](design/frontend-workbench.md)已接入连续排名浏览、侧栏保护复核、已发布快照对照与工作集派生。基础预检、异常候选处置及离线实验变体已接入；按轮次动态采样、追加计划与排名有效性摘要已接入，真实质量校准仍待完成。

## 目录职责

| 目录         | 保存内容                                                   |
| ------------ | ---------------------------------------------------------- |
| architecture | 总体架构、阶段底座说明和模块接入说明                       |
| decisions    | 按编号保存的架构决策，后续决策补充或修订早期规则           |
| plans        | 实施计划与阶段交付范围；已完成和仍待实施的部分由各文档标明 |
| design       | 公式、实验与设计原稿，保留原有推导和历史上下文             |
| verification | 有日期、条件和边界的历史验收摘要                           |

## 架构说明

首版采样与结果语义：[ADR 0032](decisions/0032-aesthetic-adaptive-sampling.md)及[2026-09-24 验收](verification/2026-09-24-aesthetic-sampling.md)。

邻近细排与加速拟合：[ADR 0033](decisions/0033-aesthetic-neighbor-refinement.md)及[150 万模拟与集成验证](verification/2026-09-24-aesthetic-refinement.md)。

当前数量准入上限为 1000 万候选：[ADR 0034](decisions/0034-aesthetic-ten-million-admission.md)及[数量边界与分块发布验收](verification/2026-09-24-aesthetic-capacity-10m.md)。

最新传输与恢复：[R2 验收](verification/2026-09-23-aesthetic-transport.md)及 [ADR 0031](decisions/0031-aesthetic-transport-and-recovery.md)。

最新接入验收：[评审执行与离线实验](verification/2026-09-23-aesthetic-execution.md)；前一轮：[连续看图、保护复核与快照对照](verification/2026-09-23-aesthetic-reading.md)。

- [UE 5 风格工作台设计与首轮范围](design/frontend-workbench.md)
- [审美评审模型输出格式](design/aesthetic-model-output.md)
- [0030：评审执行与离线实验接入](decisions/0030-aesthetic-execution-ui.md)

- [工程基础架构草案](architecture/foundation-proposal.md)
- [0.1 工程底座实现状态](architecture/foundation-status.md)
- [使用 LLM 基础层](architecture/llm-integration.md)

## 架构决策

按编号查阅；涉及同一主题时结合后续决策阅读。

- [决策 0001：首版底座与开发模式](decisions/0001-development-foundation.md)
- [项目升级与只读元数据](decisions/0002-project-metadata-layer.md)
- [项目数据范围与生命周期](decisions/0003-project-data-scopes.md)
- [0004：注册算子、项目成果与编辑会话](decisions/0004-tools-artifacts-session.md)
- [0005：读取协调与可重建预览缓存](decisions/0005-read-coordination-cache.md)
- [0006 · Query and browsing usability](decisions/0006-frontend-query-usability.md)
- [0007 — Incremental query membership and Danbooru ordering](decisions/0007-incremental-query-membership.md)
- [0008 — Cache tiers and application settings](decisions/0008-cache-tiers-settings.md)
- [0009 — Population tools and MetaRecall ranking artifacts](decisions/0009-metarecall-population-artifacts.md)
- [0010：范围浏览与排名执行的性能边界](decisions/0010-bounded-browse-and-ranking-performance.md)
- [0011：项目对象管理与选择历史](decisions/0011-object-management-and-selection-history.md)
- [0012：排名工作集的浏览顺序与 ID 起点](decisions/0012-ranked-workset-browsing.md)
- [桌面剪贴板接入](decisions/0013-desktop-clipboard.md)
- [查询结果交接与缓存保护计时](decisions/0014-query-result-handoff.md)
- [开发引擎升级时等待实际退出](decisions/0015-development-engine-restart.md)
- [排名浏览复用、有界读取与成员写入](decisions/0016-bounded-ranking-reads-and-member-writes.md)
- [固定范围的持久化排名索引](decisions/0017-persistent-ranked-scope-indexes.md)
- [0018：MetaRecall v2 的元数据阶段](decisions/0018-metarecall-v2-metadata-ranking.md)
- [排名范围复用与可恢复缓存清理](decisions/0019-ranking-cache-lifecycle.md)
- [0020：重复帖热度、最新分级与评分浏览口径](decisions/0020-duplicate-post-ranking-evidence.md)
- [0021：固定查询依赖、项目缓存明细与无引用输入回收](decisions/0021-fixed-query-dependencies-and-cache-inventory.md)
- [0022：工作集排名位置定位](decisions/0022-ranking-position-anchors.md)
- [LLM 接入与调用基础层](decisions/0023-llm-foundation.md)
- [System Prompt 预设与按次传输](decisions/0024-system-prompt-presets.md)
- [0025：美学评审的持久执行与图片准入](decisions/0025-aesthetic-evaluation-ledger.md)
- [0026：美学证据的离线估计、快照与工作集发布](decisions/0026-aesthetic-offline-analysis.md)
- [0027：评审创建恢复、存储准入与候选处置](decisions/0027-aesthetic-recovery-and-admission.md)
- [0028：公共工作台与编辑器布局](decisions/0028-frontend-workbench.md)
- [0029：连续看图、保护复核与快照对照](decisions/0029-aesthetic-reading-and-review.md)

## 实施计划

早期阶段计划保留当时的范围与验收标准；美学排序目录同时维护阶段说明及后续计划。

- [数据库架构升级：在线读取、热更新与三站更新管理（2026-09-26）](plans/数据库架构审查升级-2026-09-26/README.md)
- [三站增量补全与更新管理（后端已交付）](plans/lake-incremental-updates-2026-09-26.md)
- [三站更新 UI / UX 规划（已接入）](plans/lake-updates-ui-ux-2026-09-27.md)
- [全局数据湖工作台与固定输入（ADR 0039）](decisions/0039-lake-update-workbench.md)
- [三站更新前端验收（2026-09-27）](verification/2026-09-27-lake-updates-ui.md)
- [全局更新控制与归档运行器（ADR 0038）](decisions/0038-lake-update-control.md)
- [三站更新后端与正式湖小批次验收（2026-09-27）](verification/2026-09-27-lake-updates-backend.md)

- [Yandere / Gelbooru 数据湖接入（2026-09-26，已接入并验收）](plans/multibooru-sources-2026-09-26.md)
- [源码审查后的整体改进路线图（2026-09-19）](plans/architecture-improvement-2026-09-19.md)
- [cache-settings-v0.7](plans/cache-settings-v0.7.md)
- [frontend-usability-v0.5](plans/frontend-usability-v0.5.md)
- [metarecall-v0.8](plans/metarecall-v0.8.md)
- [object-management-v0.9](plans/object-management-v0.9.md)
- [performance-v0.8.1](plans/performance-v0.8.1.md)
- [project-data-layer-v0.2](plans/project-data-layer-v0.2.md)
- [project-data-scopes-v0.3](plans/project-data-scopes-v0.3.md)
- [query-incremental-v0.6](plans/query-incremental-v0.6.md)
- [ranking-browse-v0.9.1](plans/ranking-browse-v0.9.1.md)
- [tool-foundation-v0.4](plans/tool-foundation-v0.4.md)
- [美学排序：需求、架构、容量与阶段接入](plans/aesthetic-ranking/README.md)

## 公式与设计原稿

原始公式、实施口径和 v2 实验设计分别保留，使用时应核对对应实现决策。

- [GPT-6 Astra Pro v1.0](design/metarecall/GPT-6%20Astra%20Pro%20v1.0.md)
- [MetaRecall v2.0 实验设计](design/metarecall/MetaRecall%20v2.0%20实验设计.md)
- [实施设计 v0.8](design/metarecall/实施设计%20v0.8.md)

## 历史验收

下列记录证明对应日期、版本与样本条件下的行为，不作为当前版本已经重新执行全部测试的声明。美学排序的阶段验收随[第一阶段](plans/aesthetic-ranking/phase-1-implementation.md)和[第二阶段](plans/aesthetic-ranking/phase-2-backend.md)接入文档保存。

- [2026-09-19：R0/R1 基线、故障矩阵与验收](verification/2026-09-19-r0-r1.md)
- [2026-09-23：UE 风格工作台与排名前端](verification/2026-09-23-workbench.md)

- [performance-metarecall-workset-v0.8](verification/performance-metarecall-workset-v0.8.md)
- [ux-review-task-progress-v0.8](verification/ux-review-task-progress-v0.8.md)
- [verification-2026-09-07](verification/verification-2026-09-07.md)
- [verification-duplicate-ranking](verification/verification-duplicate-ranking.md)
- [verification-llm-foundation](verification/verification-llm-foundation.md)
- [verification-metarecall-v0.8](verification/verification-metarecall-v0.8.md)
- [verification-metarecall-v2](verification/verification-metarecall-v2.md)
- [verification-object-management-v0.9](verification/verification-object-management-v0.9.md)
- [verification-performance-v0.8.1](verification/verification-performance-v0.8.1.md)
- [verification-project-data-layer-v0.2](verification/verification-project-data-layer-v0.2.md)
- [verification-project-data-scopes-v0.3](verification/verification-project-data-scopes-v0.3.md)
- [verification-ranking-browse-v0.9.1](verification/verification-ranking-browse-v0.9.1.md)
- [verification-ranking-cache-lifecycle](verification/verification-ranking-cache-lifecycle.md)
- [verification-system-prompts](verification/verification-system-prompts.md)
- [verification-tools-resources-v0.4](verification/verification-tools-resources-v0.4.md)

## 本机证据与维护约定

仓库保存方法、结论、可复跑命令和必要的汇总指标。真实项目数据库、升级备份、逐图明细、完整日志和截图位于被 Git 忽略的 `.local/reports/` 等本机目录，克隆仓库不会获得这些材料。历史文档中的本机路径仅用于定位原始证据。

新增文档请按目录职责归档，并维护本导航或所属模块导航。移动文件时同步修改相对链接和正文中的仓库路径；本机证据位置使用普通文本并说明其本机范围，避免把本机路径作为远端可用链接。
