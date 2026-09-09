# Dataset Studio

面向大型图片数据湖的桌面工作环境。项目持续保存来源引用、选择、工作集与处理成果，各工具围绕项目中的数据工作。

当前版本：0.8.1，包含 Danbooru 元数据排名和大型工作集浏览性能修复，继续使用开发模式。修复范围与实测证据见[性能验收](docs/verification-performance-v0.8.1.md)。

“计算工具 → Danbooru 元数据排名”使用所选数据湖、查询结果、工作集或选择，固定元数据后计算 MetaRecall v1。参数配置、结果榜单、统计诊断和单图评分依据已经接入；排名可按精确名额筛选并保存为工作集，支持取消、重试和离线恢复已准备输入。详情见[实施与验证](docs/verification-metarecall-v0.8.md)。

清单与确定性标量算子通过同一注册协议执行，项目成果可分页查看、参与查询和再次消费。工具与查询草稿、查看范围和布局可恢复，正常切换和关闭等待保存。读取服务协调交互和后台工作，提供共享取消、按包偏移批读及 SSD 持久缩略图缓存。

分级基础缓存按数据湖共享，组合筛选使用候选记录缩小查询范围；项目结果按长期、临时或仅本次会话管理。顶部“设置”统一提供容量、缓存管理和查询内存选项。行为与兼容规则见[缓存分层与设置决策](docs/decisions/0008-cache-tiers-settings.md)及[实施与验收清单](plans/cache-settings-v0.7.md)。

## 启动

在 Windows 上双击仓库根目录的 **启动开发版.bat**。

脚本会检查锁文件、依赖与固定版本的元数据运行库，增量编译优化后的 Release 计算引擎，启动前端开发服务和 Tauri 桌面窗口。前端仍然热更新，桌面宿主仍使用开发构建。再次启动时会复用已有开发环境，并重新打开或激活桌面窗口。已有后台时，启动控制台结束后桌面窗口仍保持运行。

启动诊断保存在 `.local/logs/startup-dev-日期时间-进程ID.log`，每次启动独立保存，避免开发窗口持续运行时锁住下一次启动的日志。依赖和运行库检查也使用同次启动的文件后缀；复用后台时的桌面进程输出保存在 `.local/logs/desktop-startup.log`。控制台会显示日志位置；启动失败保留错误退出码并暂停，方便查看原因。

命令行完整入口：

```powershell
Set-Location 'D:\Dataset\Dataset_Studio'
pwsh -File tooling/start-dev.ps1
```

只启动浏览器开发环境时，可运行 `启动开发版.bat --web`。Windows 启动包装器回归检查使用 `pnpm test:launcher`。

需要调试未优化的引擎时，在新的开发环境启动时传入 `--engine-profile=debug`，或设置 `STUDIO_ENGINE_PROFILE=debug`；默认是 `release`。命令行选项优先于环境变量。运行缓存以实际 EXE 的 SHA-256 标识，`.local/dev/development-build.json` 记录构建类型和实例。已有开发环境会被复用，因此切换类型前需结束原开发协调进程。

- 修改前端和 CSS：Vite 热更新。
- 修改 crates 下的 Rust：自动增量编译、重新生成契约、重启开发引擎。已确认的任务检查点保留，界面重新连接。
- 修改桌面宿主：Tauri 开发工具负责重建窗口。
- 关闭窗口会释放项目视图并结束当前桌面开发控制台；引擎独立运行，后台工作完成后释放项目锁。
- 停止开发引擎：在仓库目录运行 `pnpm engine:stop`。已固定输入的任务按检查点恢复，运行中断的查询结果需要重新计算。
- 仅用浏览器调试：`pnpm dev:web`，入口为 `http://127.0.0.1:1420`。
- 已准备依赖时可直接运行 `pnpm dev`；首次使用 Danbooru 元数据前运行 `pwsh -File tooling/setup-duckdb.ps1`。

需要 Node.js 22.12 或更新版本、pnpm 10.30.3、Rust 1.94 或更新版本、Visual Studio C++ 工具链与 WebView2。本机已经验证这些构建能力。脚本只调整子进程的 MSVC 编译环境。

## 首次使用

1. 新建项目，或打开包含 project.json 的项目目录。
2. 添加完整数据湖。Danbooru 使用 SSD 索引目录及图片湖目录；内置参考资料用于验证基础功能。
3. 在项目中浏览、查看单图、选择对象、保存工作集。选择属于项目，在切换来源和重开项目后保留。
4. 点击图片后，右侧属性面板的“元数据”可切换来源记录与历史观察、查看标签及来源尺寸，按需读取原始 JSON。滚动面板查看完整内容；记录切换不改变项目选择。
5. “查询”面板按字段能力设置条件、观察规则和身份排序，保存定义并在后台构建结果。可查看结果、全选并排除个别对象，或对工作集和结果范围进行集合操作。
6. “计算工具”可使用数据湖、工作集、查询结果或项目选择。清单与标量任务固定成员、所需字段和参数，任务面板提供取消、失败后的显式重试及成果入口。
7. “项目成果”查看类型、覆盖范围和来源依据，也可继续计算或把标量列用于查询。释放前检查持久引用。
8. 顶部“设置”调整缓存与查询工作内存；“缓存管理”预建 G/S/Q/E、调整查询类别、固定或清理结果。各工具可独立使用。

当前机器的 Danbooru 位置：

```text
SSD 索引：D:\Dataset\Danbooru
图片湖：  E:\AI\AI_Dataset\Danbooru
```

数据湖按只读方式接入。当前浏览单位是 catalog.sqlite 中去重后的存储对象，使用 SHA-256 身份与包偏移定位图片；没有把历史观察条数或当前帖子数显示成图片总数。

相同内容可以关联多条来源记录和多次观察，检查面板保留这些区别。来源尺寸来自所选观察，实际存储尺寸尚未检查时显示未知。同条目的其他历史观察可能对应不同图片内容。

## 数据与缓存

- `.local/dev/registry.sqlite`：开发环境的项目注册表与本机数据源位置。
- `.local/dev/projects/`：默认新建项目的位置；也可以在新建时选择其他父目录。
- 项目内的 project.json、project.sqlite、artifacts 与 .staging 分别保存身份、项目状态、正式成果与恢复所需暂存。
- 显式打开旧项目时，数据库 v1–v7 按顺序升级至 v8；升级前的一致备份保存在项目内 `.backups/v*-to-v8-*`，包含 WAL 中已提交的数据，保留以前的迁移账本。清单 format_version 仍为 1。关闭项目的缓存维护也可在取得项目锁后升级 v6–v7。
- 注册表单独使用模式 v2，旧注册表备份位于应用数据目录的 `.backups/registry-v*-to-v2-*`。最近项目列表不隐式打开或升级项目。
- 查询定义、结果材料、选择基底与排除项、范围依据保存在项目数据库。结果释放前检查引用，已固定的工作集和任务不会随重新查询而变化。
- 工具草稿和项目会话保存在项目数据库；布局偏好保存在应用注册表。未知草稿版本保留原文，并发冲突需明确选择重新载入或保留本地。
- `.local/dev/preview-cache/`：独立的可重建缩略图及索引，默认配额 2 GiB。清理不进入项目成果和备份。缓存设置与未完成清理意图独立保存，缓存索引损坏后可继续恢复。
- `.local/dev/rating-cache/` 与 `browse-index/`：同一数据湖共享的分级基础候选、常用标签及浏览排序索引。Rating/Tag 组合在相应缓存内筛选，来源库保持只读，基础缓存按来源水位更新。
- 总缓存默认 64 GiB，可配置至 1024 GiB；初始长期预算 48 GiB、临时 8 GiB，并保留原缩略图预算。长期默认不按时间过期，临时默认最后使用后 24 小时清理，或选择仅本次项目会话。调整类别不复制成员；固定、正在查看和项目引用保护优先于普通清理，因此实际占用可能暂时超过预算。
- `.local/engine-binaries/`：开发引擎的可重建二进制快照，避免运行中的 EXE 阻止增量链接。
- `.local/logs/`：构建、检查和临时命令日志；启动依赖检查写入其中的 startup-install-日期时间-进程ID.log。
- `.local/test-runs/`：自动测试及一次性验证的隔离运行目录。完成后将需要保留的报告、截图归档到 `.local/reports/`，再逐级清理夹具与缓存。
- `.local/dev/engine.log`：与开发运行环境一起保存的引擎日志。

根目录仅保留工程入口、说明及工具所需配置。首次搭建留下的散落日志已归档至 .local/logs/root-archive-*，后续检查输出也统一放入日志目录。

`.local/dev` 包含实际项目数据，不是一个整体可丢弃的构建缓存。Git 忽略项目数据、设计参考、依赖、日志与二进制。

项目保存逻辑数据湖引用，本机位置留在应用注册表中。项目移动后可以重新打开；换到另一台机器时，需要重新关联本机数据湖位置。任务计划使用项目内相对路径，并校验固定输入内容。

数据湖行的关联按钮可更换本机位置。新位置必须仍为同一个逻辑数据湖；修改会影响当前应用登记的所有相关项目，界面会说明该共享范围。

可在启动前设置进程环境变量 `STUDIO_CACHE_DIR` 指定 SSD 缩略图目录，或为独立引擎传入 `--cache-dir`。目标必须是空目录或已有的 Dataset Studio 缓存目录；配额在“设置 → 缓存与存储”中调整。磁盘命中仍检查项目来源权限，离线命中保留上次验证时间并标记状态。

## 开发与检查

```powershell
pnpm install --frozen-lockfile
pwsh -File tooling/setup-duckdb.ps1
node tooling/prepare-sidecar.mjs
pnpm contracts
pnpm check
pnpm test:integration
pnpm build
```

`pnpm build` 只构建前端资产用于验证。默认脚本不生成发布版或安装包。

接口以 Rust DTO 和 Utoipa 定义为准。生成的 OpenAPI 和 TypeScript 类型纳入 Git，CI 重新生成后检查漂移。前端功能通过 SDK 调用引擎，原生目录选择由应用层注入。

测试覆盖旧项目升级、只读元数据、范围与结果、算子注册和固定字段、成果发布恢复、草稿冲突、读取公平性和取消、持久缓存与引用隔离。`test:integration` 包含 9 组独立引擎脚本，覆盖增量查询、分层预算、每日分级更新、真实 SDK 会话重连、MetaRecall 排名和有界范围排序。夹具默认使用 Debug，引擎已构建时可用 `STUDIO_ENGINE_PROFILE=release` 验证优化产物。排名界面检查为 `node tooling/smoke-ranking-ui.mjs`，使用独立引擎与无头 Edge 上下文；真实有界元数据检查为 `tooling/verify-ranking.mjs`。可选的原生设置窗口检查为 `node tooling/smoke-settings-ui.mjs`，先关闭现有开发窗口与前端服务；它只使用独立的合成图片与项目。

真实数据湖可使用 `node tooling/verify-metadata.mjs --index-root <索引根目录> --media-root <图片湖根目录> --asset <SHA256>` 做有界验证，最多传入 8 个 `--asset`。脚本使用隔离运行目录与真实引擎 API，报告写入 `.local/test-runs/metadata-verification-*`。

查询路径的指定样本检查使用 `node tooling/verify-scopes.mjs --index-root <索引根目录> --media-root <图片湖根目录> --asset-id <SHA256> --post-id <帖子ID>`，先记录同一编译器的查询计划，再通过引擎执行两条身份受限的查询，报告写入 `.local/test-runs/scope-verification-*`。示例检查要求该对象是 WebP，且关联帖子有来源宽高均不小于 1000 的观察；它不是通用数据质量断言。

读取检查使用 `node tooling/verify-reads.mjs --index-root <索引根目录> --media-root <图片湖根目录> --asset <SHA256> --max-source-bytes 2097152`，最多八个显式对象；在读图前核对总预算，并比较冷应用缓存、暖缓存和引擎重启。报告位于 `.local/test-runs/read-verification-*`。

目录职责与实现边界见：

- [基础架构方案](docs/architecture/foundation-proposal.md)
- [0.1 实现状态](docs/architecture/foundation-status.md)
- [0.2 实现与验证](docs/verification-project-data-layer-v0.2.md)
- [0.3 实现与验证](docs/verification-project-data-scopes-v0.3.md)
- [0.4 实现与验证](docs/verification-tools-resources-v0.4.md)
- [开发模式决策](docs/decisions/0001-development-foundation.md)
- [项目升级与元数据读取决策](docs/decisions/0002-project-metadata-layer.md)
- [查询、数据范围与项目生命周期决策](docs/decisions/0003-project-data-scopes.md)
- [算子、成果与会话决策](docs/decisions/0004-tools-artifacts-session.md)
- [读取协调与缓存决策](docs/decisions/0005-read-coordination-cache.md)
- [增量成员与排序决策](docs/decisions/0007-incremental-query-membership.md)
- [缓存分层与设置决策](docs/decisions/0008-cache-tiers-settings.md)
- [总体工具与排名成果决策](docs/decisions/0009-metarecall-population-artifacts.md)
- [项目数据层计划及验收标准](docs/plans/project-data-layer-v0.2.md)
- [0.3 项目数据范围层计划](docs/plans/project-data-scopes-v0.3.md)
- [0.4 工具扩展与成果基础层计划](docs/plans/tool-foundation-v0.4.md)

## 当前边界

任意元数据公式、模型打标和审美评估尚未实现。

结果每页最多 128 项，界面可选 12/48/96 项。单项选择修改每次最多 1000 项；全选结果通过后端引用完成，不枚举全部 ID。查询协议最多 8 个来源、12 个联合条件，界面当前支持单湖编辑。每个引擎同时运行一个结果构建和一个算子执行器。预览单图及单批编码输入上限均为 64 MiB，每批最多 16 项，另有解码及队列预算。

标量成果采用有模式的 JSONL 加 SQLite 整数投影。MetaRecall 使用独立的固定输入表、浮点评分与排名表和摘要文件，提供多列分页及工作集转换；任意宽表算子、向量成果尚未接入。正常关闭等待已接受的草稿保存；强制终止或页面突然重载仅保证此前已落盘的编辑。运行时第三方插件和完整模型环境继续按实际工具需求扩展。

成果始终保存在项目 artifacts 目录中。界面另存下载目前限制为 64 MiB，较大成果直接使用项目内的原文件，以控制前端内存。

已经验证本机真实 Danbooru 的只读分页、包偏移读取、内容校验、预览及大规模分级查询，也验证了较大合成目录上的结果和范围操作。各验收报告标明构建类型、查询条件和缓存边界，不代表全条件或长时间运行压测。

DuckDB 1.5.4 已通过应用自身的原生运行库接入元数据检查。每次读取分别建立 SQLite / DuckDB 只读事务并核对版本，不提供跨库原子历史快照。元数据默认 20 条来源记录、10 次观察分页；原始 JSON 查看上限为 128 KiB。源忙碌、离线和版本变化会明确提示。

查询结果保留生成时的定义与版本。来源变化后需要重新计算才能直接使用查询范围，已经固定到工作集、选择和任务的成员不变。查询工作内存默认 12 GiB，可在设置中调整为 1–64 GiB；默认原生查询分配 10 GiB、结果整理最多 2 GiB。原生溢写和结果暂存各有 8 GiB 磁盘预算。预算与取消使用协作中断，不是整个进程的内存或响应时间硬上限。
