# Dataset Studio

面向大型图片数据湖的桌面工作环境。项目持续保存来源引用、选择、工作集与处理成果，各工具围绕项目中的数据工作。

当前版本：0.3 项目数据范围层。当前交付和日常迭代均使用开发模式。

正在实施：[0.4 工具扩展与成果基础层](docs/plans/tool-foundation-v0.4.md)，涵盖功能模块与算子注册、成果与派生数据管理、工具草稿与会话恢复、读取调度与持久缓存。当前完整交付基线仍为 0.3，新增能力将随本轮验证完成更新。

## 启动

在 Windows 上双击仓库根目录的 **启动开发版.bat**。

脚本会检查锁文件、依赖与固定版本的元数据运行库，增量编译 Debug 引擎，启动前端开发服务和 Tauri 桌面窗口。再次启动时会使用已有开发环境。开发控制台可查看编译结果。

命令行完整入口：

```powershell
Set-Location 'D:\Dataset\Dataset_Studio'
pwsh -File tooling/start-dev.ps1
```

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
2. 添加完整数据湖。Danbooru 需要 SSD 索引目录和机械盘图片湖目录；内置参考资料用于验证基础功能。
3. 在项目中浏览、查看单图、选择对象、保存工作集。选择属于项目，在切换来源和重开项目后保留。
4. 点击图片后，右侧属性面板的“元数据”可切换来源记录与历史观察、查看标签及来源尺寸，按需读取原始 JSON。滚动面板查看完整内容；记录切换不改变项目选择。
5. “查询”面板按字段能力设置条件、观察规则和身份排序，保存定义并在后台构建结果。可查看结果、全选并排除个别对象，或对工作集和结果范围进行集合操作。
6. “生成数据清单”可直接使用数据湖、工作集、查询结果或项目选择。任务固定实际成员并独立运行，任务面板显示进度、取消和成果下载。各工具可独立使用。

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
- 显式打开旧项目时，数据库 v1/v2 按顺序升级至 v3；升级前的一致备份保存在项目内 `.backups/v*-to-v3-*`，包含 WAL 中已提交的数据，保留以前的迁移账本。清单 format_version 仍为 1。
- 注册表单独使用模式 v1，旧注册表备份位于应用数据目录的 `.backups/registry-v0-to-v1-*`。最近项目列表不隐式打开或升级项目。
- 查询定义、结果材料、选择基底与排除项、范围依据保存在项目数据库。结果释放前检查引用，已固定的工作集和任务不会随重新查询而变化。
- `.local/engine-binaries/`：开发引擎的可重建二进制快照，避免运行中的 EXE 阻止增量链接。
- `.local/logs/`：构建、检查和临时命令日志；启动依赖检查写入其中的 startup-install.log。
- `.local/dev/engine.log`：与开发运行环境一起保存的引擎日志。

根目录仅保留工程入口、说明及工具所需配置。首次搭建留下的散落日志已归档至 .local/logs/root-archive-*，后续检查输出也统一放入日志目录。

`.local/dev` 包含实际项目数据，不是一个整体可丢弃的构建缓存。Git 忽略项目数据、设计参考、依赖、日志与二进制。

项目保存逻辑数据湖引用，本机位置留在应用注册表中。项目移动后可以重新打开；换到另一台机器时，需要重新关联本机数据湖位置。任务计划使用项目内相对路径，并校验固定输入内容。

数据湖行的关联按钮可更换本机位置。新位置必须仍为同一个逻辑数据湖；修改会影响当前应用登记的所有相关项目，界面会说明该共享范围。

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

测试覆盖旧项目与注册表升级、只读元数据、查询类型和多观察语义、结果状态与分页、范围运算和输入固定、取消与恢复、共享来源位置，以及异常项目旁健康工作的故障隔离。`test:integration` 包含原有回归与范围协议的独立引擎进程验证。

真实数据湖可使用 `node tooling/verify-metadata.mjs --index-root <索引根目录> --media-root <图片湖根目录> --asset <SHA256>` 做有界验证，最多传入 8 个 `--asset`。脚本使用隔离运行目录与真实引擎 API，报告写入 `.local/metadata-verification-*`。

查询路径的指定样本检查使用 `node tooling/verify-scopes.mjs --index-root <索引根目录> --media-root <图片湖根目录> --asset-id <SHA256> --post-id <帖子ID>`，先记录同一编译器的查询计划，再通过引擎执行两条身份受限的查询，报告写入 `.local/scope-verification-*`。示例检查要求该对象是 WebP，且关联帖子有来源宽高均不小于 1000 的观察；它不是通用数据质量断言。

目录职责与实现边界见：

- [基础架构方案](docs/architecture/foundation-proposal.md)
- [0.1 实现状态](docs/architecture/foundation-status.md)
- [0.2 实现与验证](docs/verification-project-data-layer-v0.2.md)
- [0.3 实现与验证](docs/verification-project-data-scopes-v0.3.md)
- [开发模式决策](docs/decisions/0001-development-foundation.md)
- [项目升级与元数据读取决策](docs/decisions/0002-project-metadata-layer.md)
- [查询、数据范围与项目生命周期决策](docs/decisions/0003-project-data-scopes.md)
- [项目数据层计划及验收标准](docs/plans/project-data-layer-v0.2.md)
- [0.3 项目数据范围层计划](docs/plans/project-data-scopes-v0.3.md)
- [下一阶段：0.4 工具扩展与成果基础层](docs/plans/tool-foundation-v0.4.md)

## 当前边界

元数据公式、大模型打标和审美评估尚未实现。

结果每页最多 128 项，界面可选 12/48/96 项。单项选择修改每次最多 1000 项；全选结果通过后端引用完成，不枚举全部 ID。查询协议最多 8 个来源、12 个联合条件，界面当前支持单湖编辑。每个引擎同时运行一个结果构建和一个清单执行器。预览对单图原始字节设有 64 MiB 上限，对解码内存和同时读取数量设有预算。

成果始终保存在项目 artifacts 目录中。界面另存下载目前限制为 64 MiB，较大成果直接使用项目内的原文件，以控制前端内存。

已经验证本机真实 Danbooru 的只读分页、包偏移读取、内容校验、预览和指定条件的小范围查询，也验证了较大合成目录上的结果和范围操作。尚未完成千万级真实数据全量性能或长时间运行压测。

DuckDB 1.5.4 已通过应用自身的原生运行库接入元数据检查。每次读取分别建立 SQLite / DuckDB 只读事务并核对版本，不提供跨库原子历史快照。元数据默认 20 条来源记录、10 次观察分页；原始 JSON 查看上限为 128 KiB。源忙碌、离线和版本变化会明确提示。

查询结果保留生成时的定义与版本。来源变化后需要重新计算才能直接使用查询范围，已经固定到工作集、选择和任务的成员不变。较大元数据连接受 256 MB 原生内存预算约束，可能明确返回资源不足；预算与取消使用协作中断，不是整个进程的内存或响应时间硬上限。
