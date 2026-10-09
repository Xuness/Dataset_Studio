# 开发与验证工具

以下命令均从仓库根目录运行。日常启动、所需环境及数据目录见[仓库说明](../README.md)，架构与历史验收见[文档导航](../docs/README.md)。

Linux x86_64 入口见 [Linux 开发运行](../docs/architecture/linux-development.md)：`pnpm setup:duckdb`、`pnpm setup:lake --dev`、`pnpm dev`。`bash tooling/linux-test-session.sh <命令>` 为 Linux 测试提供隔离的桌面和密钥环；`pnpm test:linux-native` 验证真实 WebKitGTK 窗口、目录选择器、剪贴板和 Pixiv 登录会话。普通 Linux 界面脚本使用 Playwright Chromium，首次运行需 `pnpm exec playwright install chromium`。Windows 原生检查保留现有入口。

## 日常验证范围

本页是工具索引，不是每次开发都要执行的清单。默认按修改文件、行为及直接依赖选最小检查集；相关检查通过即停止，后续只因代码改动、失败或具体疑点补跑。

| 改动 | 优先检查 |
| --- | --- |
| 文档、文案、简单样式 | 内容、链接或相关界面；不运行后端测试 |
| 前端逻辑 | `pnpm typecheck`、修改文件的 `pnpm exec eslint <文件>`；导入关系变化再运行 `node tooling/boundaries.mjs`，必要时选对应 UI 脚本 |
| Rust 模块 | `pnpm test:rust -p <crate> <过滤器>`；按需对受影响 crate 运行格式/Clippy 检查 |
| Python 服务 | `pnpm test:lake tests/<文件>.py`，支持节点 ID 和 `-k <过滤器>`；路径相对 `services/lake-worker` |
| HTTP/SDK、任务或存储链路 | `pnpm test:integration <套件名...>`，仅选相关链路；公共 DTO/路由变化先 `pnpm contracts` |
| 开发脚本 | 修改文件的语法/Lint 检查及一条相关执行路径 |

跨模块影响广泛或明确要求完整验收时才使用 `pnpm check:full`、`pnpm test:integration:all`；CI 保留这两个完整入口。普通开发不重复安装依赖、生成契约或构建 Release；前端构建按打包、资源、依赖变化决定。已经由完整入口覆盖且代码未再改变的检查不重复执行。历史计划中的阶段验收要求以 [AGENTS.md](../AGENTS.md) 的当前规则为准。

原图导出回归：`pnpm test:integration exports` 使用两个隔离合成湖，覆盖共享目录保护、图片与元数据冲突、元数据中断后的部分完成及同任务重试。文件发布、取消清理和流式错误日志的单元回归为 `pnpm test:rust -p studio-engine exports::files::tests`。

查看器回归：`node tooling/test-image-viewer.mjs` 在 2560×1440、100% 缩放下加载实际共享组件，验证美学大图快捷键，以及普通图、小图、1 像素图和长图的 1:1 与连续缩放。文件对话框和剪贴板使用测试替身，不触碰桌面，也不构成原生平台能力验收。

数据湖存储维护：`node tooling/lake-storage.mjs` 提供 `build / verify / compare / release / activate / cleanup / handoff / retire`。构建不需要旧生产者索引；`handoff` 和 `retire` 默认预览，`--apply` 才提交交接或回收。`cleanup` 只清理本工具未启用的准备库并先归档小型证据。操作约束与命令示例见 [0050](../docs/decisions/0050-archive-rebuild-and-producer-retirement.md)。

Pixiv 后端：`node tooling/source-collections.mjs` 调用同一个采集服务；`pnpm test:integration collections` 使用离线夹具验证 HTTP/SDK、来源读取和任务控制。`node tooling/validate-pixiv-public.mjs --author 10109777 --output '<isolated output>'` 是显式联网的小样本验证，不属于自动测试。`lake-storage build --site pixiv` 支持只靠归档重建在线 3。参数、会话边界和使用例见[采集接入说明](../docs/architecture/source-collections.md)。

Pixiv 归档规模验证：使用项目 Python 执行 `tooling/benchmark-pixiv-archive.py --output '<new child of .local/test-runs>' --works 10000 100000`。合成详情、媒体清单和共用小 PNG，验证生产组批 writer、发布与独立重建；不访问源站，不代表完整调度器或大图下载吞吐。结果和边界见[审查修复验收](../docs/verification/2026-10-03-pixiv-review-upgrade.md)。

三站更新服务已纳入 `services/lake-worker`，运行源码随引擎打包，不需要另一个 Store 仓库。首次运行 `pwsh -File tooling/setup-lake-worker.ps1 -Dev` 安装本项目的 Python 依赖，也可用 `STUDIO_LAKE_TEST_PYTHON` 指定已有环境。`pnpm test:lake <文件或过滤器>` 覆盖相关编码、归档或恢复行为；`pnpm test:integration lake-updates` 验证内置运行器与 HTTP/SDK；`pnpm test:lake-ui` 验证真实界面和命名策略预设。按变更范围选择，不默认连续执行。夹具仅使用 `.local/test-runs/` 隔离湖，无正式 API 抓取。完整 Python 服务测试属于 `pnpm check:full`。在线读取回归用 `pnpm test:integration online`。

三站更新工作台：`node tooling/smoke-lake-updates-ui.mjs`，使用本项目的 Python 环境与本机 Edge，端口 1453。脚本创建隔离真实在线湖、控制库和项目，验证范围冻结、跨项目/重启、三湖提交、计划、凭据及不同窗口尺寸；不会使用正式凭据或调用源站 API。

更新生命周期回归：`services/lake-worker/tests/test_update_lifecycle.py` 已包含在 `pnpm test:lake`，覆盖原子命令、100/1000 项跨湖积压、取消阶段交错、真实子进程清理中断和 Windows junction 边界。`integration-lake-updates.mjs` 使用 `lake-lifecycle-fixture.py` 在停止的隔离控制库中准备取消/暂停任务，验证 HTTP/SDK 清理回执和重启回收。见 [R2 验收](../docs/verification/2026-09-29-astra-r2-update-lifecycle.md)。

脚本使用自身位置定位仓库根目录，统一保留在 tooling 顶层。测试源码和夹具生成器随 Git 保存；运行产生的数据、截图和日志使用 `.local/`。

在线湖验收：`pnpm test:integration online`。默认使用 `setup-lake-worker.ps1 -Dev` 安装的项目 Python（DuckDB、APSW、pytz），也可用 `PYTHON` 显式指定。它也验证查询视图并发创建、分页重试、发布后续页、重启和过期。`node tooling/smoke-online-ui.mjs <online-fixture-directory>` 检查分页视图和保存工作集；`node tooling/verify-online-lakes.mjs <lake-paths.json>` 对正式湖做有界只读抽样。路径 JSON 是含 site/index/media 的数组，不随仓库保存本机数据。协议与边界见 [P2/P3 验收](../docs/verification/2026-09-26-online-upgrade-p23.md)及 [R1 查询可靠性验收](../docs/verification/2026-09-29-astra-r1-query-reliability.md)。

## 环境与常用命令

工作台前端验收：按受影响功能选择 `node tooling/smoke-workbench-ui.mjs <analysis-fixture>` 或 `node tooling/smoke-aesthetic-ui.mjs <evaluation-fixture>`。后端未变且夹具仍有效时复用已有隔离项目；需要新夹具时运行对应的 `pnpm test:integration aesthetic-analysis` 或 `pnpm test:integration aesthetic`。前者使用端口 1447，后者使用 1439；均只操作隔离合成项目。`ui-workbench.mjs` 提供功能标签/菜单的共享测试导航。`ui-ranking-reading.mjs` 扩展工作台检查：缩略图密度、键盘跨页、大图缩放平移、滚动恢复、逐图复核草稿、丢失响应后同键重试、固定保护队列和真实离线快照对照。

浏览布局回归集成在 `node tooling/smoke-ranking-browse-ui.mjs` 中，由 `ui-browser-layout.mjs` 验证。首要基准为 2560×1440、100% 缩放，另覆盖任务栏可用高度、150% 远程缩放和较小窗口；实际发送滚轮事件，检查右侧标签、草稿随停靠/隐藏/刷新恢复、筛选输入可达、图片滚动、查询侧栏、分页及全局底栏边界。业务回归同时验证 Rating 和 Tag 包含/排除筛选。

Windows 开发环境需要 PowerShell 7、Node.js 22.12 或更新版本、pnpm 10.30.3、Rust 1.94 或更新版本、Visual Studio C++ 工具链和 WebView2。集成测试及部分界面验证还需要 **Python 3.11 x64**，`python --version` 应能找到对应解释器；本机夹具验证使用 3.11.9，CI 选择 3.11 x64。

传统 DuckDB Python 夹具只依赖标准库，通过 ctypes 加载 `vendor/duckdb/duckdb.dll`。先运行 `pwsh -File tooling/setup-duckdb.ps1` 准备固定版本的 DLL，再执行相关验证。在线与更新夹具还依赖上述 Python / Store 环境及 APSW 等包。界面冒烟脚本需要已安装的 Microsoft Edge，原生窗口验证需要可交互的 Windows 桌面。

| 入口                                      | 用途                                                             |
| ----------------------------------------- | ---------------------------------------------------------------- |
| `pwsh -File tooling/start-dev.ps1`        | 安装依赖、准备 DuckDB 并启动开发环境；也可双击根目录启动器       |
| `bash ./启动开发版.sh` / `bash tooling/start-dev.sh` | Linux 源码启动，检查依赖和 DuckDB，支持 `--web`；日志写入 `.local/logs/` |
| `pnpm dev` / `pnpm dev:web`               | 已准备依赖后的桌面 / 浏览器开发环境                              |
| `pnpm engine:stop`                        | 结束当前开发引擎                                                 |
| `pnpm contracts` / `pnpm contracts:check` | 生成公开契约 / 核对契约漂移                                      |
| `pnpm check`                              | 轻量类型、Lint、依赖边界和 Rust 格式；无引擎构建或后端测试 |
| `pnpm check:full`                         | 完整静态检查、契约、Clippy、Rust/SDK 与 Python 测试 |
| `pnpm test:rust -p <crate> <过滤器>`       | 指定 Rust crate 或用例 |
| `pnpm test:lake tests/<文件>.py -k <过滤器>` | 指定 Python 文件或用例；无参数时运行完整服务测试 |
| `pnpm test:integration <套件名...>`        | 只运行指定集成；`--list` 列出名称，`--dry-run` 预览范围 |
| `pnpm test:integration:all`                | 完整 28 组集成，包含千万行容量专项 |
| `pnpm test:capacity`                      | 美学恢复回归及千万行容量专项 |
| `pnpm test:launcher`                      | Windows 启动器与引擎进程管理回归                                 |
| `pnpm test:clipboard`                     | Windows 原生剪贴板及 Win+V 历史验证，会写入系统剪贴板            |
| `pnpm build`                              | 验证前端构建，不生成安装包                                       |

## 开发、构建与生成

- 启动与进程：[start-dev.ps1](start-dev.ps1)、[start-dev.sh](start-dev.sh)、[dev.mjs](dev.mjs)、[stop-engine.mjs](stop-engine.mjs)、[engine-process.mjs](engine-process.mjs)、[engine-profile.mjs](engine-profile.mjs)。
- Rust 工具链与 sidecar：[cargo.mjs](cargo.mjs)、[cargo-run.mjs](cargo-run.mjs)、[prepare-sidecar.mjs](prepare-sidecar.mjs)。
- 原生元数据运行库：[setup-duckdb.ps1](setup-duckdb.ps1)，下载时核对固定 SHA-256。
- 公开契约：[contracts.mjs](contracts.mjs)；OpenAPI 和 schema.d.ts 由源码生成并纳入 Git，不手工编辑。
- 图标：[icons.mjs](icons.mjs)；通过 `pnpm icons` 从 UI SVG 同步桌面图标。
- 前端导入边界：[boundaries.mjs](boundaries.mjs)，由 `pnpm lint` 调用。

## 自动检查与隔离集成

Rust 测试位于 crates 内，日常用 `pnpm test:rust -p <crate> <过滤器>`。`pnpm test` 保留全部 Rust/SDK 测试，用于完整验收；SDK 变更可直接选择 [client-foundations.mjs](client-foundations.mjs) 或 [client-media.mjs](client-media.mjs)。启动器检查为 [test-engine-process.mjs](test-engine-process.mjs) 和 [test-launcher.mjs](test-launcher.mjs)。

`pnpm test:integration ranking-browse` 仅运行排名浏览；`pnpm test:integration llm system-prompts` 运行指定的两组。无参数或名称错误时停止并提示，不会意外跑全套。[test-integration.mjs](test-integration.mjs) 维护名称及完整运行顺序，执行前为普通套件准备一次对应引擎；`--dry-run` 不构建、不启动引擎。下表索引主要脚本，均使用独立引擎与合成数据：

| 范围                       | 脚本                                                                                                                                                                                                                                                                                                                                                                                                       |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 项目、范围、工具和读取资源 | [integration.mjs](integration.mjs)、[integration-scopes.mjs](integration-scopes.mjs)、[integration-tools.mjs](integration-tools.mjs)、[integration-resources.mjs](integration-resources.mjs)、[integration-artifact-scale.mjs](integration-artifact-scale.mjs)                                                                                                                                             |
| 查询缓存、设置和缓存清单   | [integration-query-cache.mjs](integration-query-cache.mjs)、[integration-cache-settings.mjs](integration-cache-settings.mjs)、[integration-cache-inventory.mjs](integration-cache-inventory.mjs)                                                                                                                                                                                                           |
| 排名与浏览                 | [integration-ranking.mjs](integration-ranking.mjs)、[integration-ranking-v2.mjs](integration-ranking-v2.mjs)、[integration-scoped-browse.mjs](integration-scoped-browse.mjs)、[integration-ranking-browse.mjs](integration-ranking-browse.mjs)、[integration-ranked-cache-lifecycle.mjs](integration-ranked-cache-lifecycle.mjs)、[integration-ranking-duplicates.mjs](integration-ranking-duplicates.mjs) |
| 对象管理                   | [integration-management.mjs](integration-management.mjs)                                                                                                                                                                                                                                                                                                                                                   |
| LLM 与系统指令             | [integration-llm.mjs](integration-llm.mjs)、[integration-system-prompts.mjs](integration-system-prompts.mjs)                                                                                                                                                                                                                                                                                               |
| 美学评审与离线统计         | [integration-aesthetic.mjs](integration-aesthetic.mjs)、[integration-aesthetic-analysis.mjs](integration-aesthetic-analysis.mjs)                                                                                                                                                                                                                                                                           |

另外包括 [integration-aesthetic-recovery.mjs](integration-aesthetic-recovery.mjs)：默认验证 R0/R1 创建中断、存储故障、发送屏障、候选处置、冻结模板和请求 ID。真实 10,000,000 / 10,000,001 行容量准入由 `pnpm test:capacity` 显式启用，也包含在 `pnpm test:integration:all` 中；普通运行报告明确标记容量检查 `not_run`。该脚本自行构建 Debug `test-faults` 引擎，通过隔离文件注入故障，结束后恢复普通构建；不调用商业供应商，不证明百万图片吞吐。

普通夹具引擎默认使用 Debug；需要验证已有 Release 产物时可设置 `STUDIO_ENGINE_PROFILE=release`。单独执行集成脚本前，应先准备 DuckDB 和相应引擎构建。

## 界面与原生窗口验证

按需要用 `node tooling/<脚本名>.mjs` 执行。各脚本的桌面、端口和引擎前置条件以脚本开头及仓库说明为准。

- 排名：[smoke-ranking-ui.mjs](smoke-ranking-ui.mjs)、[smoke-ranking-browse-ui.mjs](smoke-ranking-browse-ui.mjs)、[smoke-ranking-pagesize-ui.mjs](smoke-ranking-pagesize-ui.mjs)。
- 参数预设：[smoke-preset-recovery-ui.mjs](smoke-preset-recovery-ui.mjs) 使用隔离项目、真实预设 API 和流式项目事件，验证 V2 参数保存、跨窗口预设刷新与后台浏览结果失效互不干扰；查询结果的过期状态在 HTTP 边界回放，返回资料浏览时仍执行恢复。
- 管理与设置：[smoke-management-ui.mjs](smoke-management-ui.mjs)、[smoke-settings-ui.mjs](smoke-settings-ui.mjs)。原生设置窗口验证前需关闭已有开发窗口与前端服务。
- 数据湖：[smoke-lake-updates-ui.mjs](smoke-lake-updates-ui.mjs) 验证 Booru 更新、范围、调度和设置；[smoke-pixiv-ui.mjs](smoke-pixiv-ui.mjs) 验证 Pixiv 浏览与采集。新增 [smoke-zero-lake-ui.mjs](smoke-zero-lake-ui.mjs) 使用端口 1461，在 2560×1440 与 1706×960 验证四源空湖创建、项目关联、元数据目录选择与任务传递；[zero-lake-fixture.py](zero-lake-fixture.py) 只在该脚本创建的隔离湖内通过真实运行器及模拟响应入湖，禁止源站请求。执行前准备 Debug 引擎和 lake-worker 测试环境。
- 模型与评审：[smoke-llm-ui.mjs](smoke-llm-ui.mjs)、[system-prompts-ui.mjs](system-prompts-ui.mjs)、[smoke-aesthetic-ui.mjs](smoke-aesthetic-ui.mjs)。
- 评审接入小样本：[smoke-evaluation-workflow.mjs](smoke-evaluation-workflow.mjs)。独立生成 16 张同 Rating 合成图，经本机 mock、真实 SDK/HTTP/SQLite 验证预检、创建/处置/实验断线恢复、重评和离线双变体导航；使用端口 1449，不访问商业模型。
- 剪贴板：[smoke-clipboard-ui.mjs](smoke-clipboard-ui.mjs)，通过 `pnpm test:clipboard` 先构建所需原生探针；要求开启系统剪贴板历史。
- 原生辅助脚本：[native-ui-controls.ps1](native-ui-controls.ps1)、[clipboard-history.ps1](clipboard-history.ps1)。

## 指定真实数据的有界验证

以下入口按参数读取明确指定的数据湖或对象，参数与预算说明见[仓库说明](../README.md#开发与检查)。运行结果只证明所指定样本与条件下的路径，不代表全湖质量或吞吐。

- [verify-metadata.mjs](verify-metadata.mjs)：指定对象的元数据读取。
- [verify-scopes.mjs](verify-scopes.mjs)：指定对象 / 帖子条件的查询路径。
- [verify-reads.mjs](verify-reads.mjs)：指定对象的读取预算、应用缓存和重启。
- [verify-ranking.mjs](verify-ranking.mjs)：真实元数据的有界排名试点。

全湖性能入口为 [benchmark-ranking.mjs](benchmark-ranking.mjs)，与上述有界试点分开使用。先构建 Release 引擎，再传入 `--index-root`、`--media-root`、`--parameters <OperatorRun.json>`，可设置 `--memory-gib` 和 `--timeout-seconds`。默认在独立项目运行完整全湖排名，保留报告、成果、二进制摘要与首屏记录；不清空操作系统缓存，也不改写图片或湖中元数据。

`--reuse-run <已成功的运行目录>` 使用该运行的项目验证固定输入复用，仍重新计算和发布。`--live-connection <开发引擎的 engine.json> --project-id <项目 ID> --source-id <来源 ID>` 会向明确指定的实际项目提交任务；它核对开发引擎二进制，保持当前资源设置，不启动或关闭该引擎。真实数据运行会生成持久成果和版本租约；归档报告后应通过正常项目/任务生命周期清理不需要的隔离项目，不直接删除仍有引用的湖租约。

## 夹具与结果归档

排名工作集性能入口为 [benchmark-ranked-worksets.mjs](benchmark-ranked-worksets.mjs)。传入 `--connection <engine.json> --project-id <项目 ID> --artifact-id <已发布排名成果 ID> --label <运行说明>`，使用实际开发引擎验证保存、冻结 Rating 筛选、五种排名分页、深位定位和自然帖子排序，并对照同来源的数据湖首屏。运行要求开发引擎与当前 Release 二进制一致，不改变资源设置。默认通过正常接口删除本轮创建的工作集和查询；`--keep-workset` 仅保留本轮完整范围工作集。它复用现有成果，不重新排名、不清除系统缓存；接口时间不包含缩略图解码。协议见 [ADR 0058](../docs/decisions/0058-fixed-ranking-membership-recipes.md)。

数据库在线升级 P1 的规模实验与并发测试由 Rust 测试集维护，包含在 `pnpm check:full`。独立复跑命令、SQLite 版本与测量边界见 [P1 验收](../docs/verification/2026-09-26-online-upgrade-phase1.md)；日常存储或预览改动只选择相关 Rust 用例及受影响集成，不自动运行全套。

- [engine-fixture.mjs](engine-fixture.mjs)：独立引擎启动、请求、等待与停止。
- [client-fixture.mjs](client-fixture.mjs)：测试使用的 SDK 装载。
- [llm-fixture.mjs](llm-fixture.mjs)：本机模拟供应商服务，不调用商业模型。
- [ui-fixture.py](ui-fixture.py)、[query-fixture-update.py](query-fixture-update.py)：生成与推进隔离 UI / 查询数据湖。
- [ranking-fixture.py](ranking-fixture.py)、[ranking-fixture-update.py](ranking-fixture-update.py)：生成有界排名数据湖及追加模拟更新。

运行产物统一保存在 `.local/test-runs/`，命令日志放在 `.local/logs/`。阶段收尾时一次性清理本轮明确拥有的临时文件和空目录；只将必要报告或失败日志留在 `.local/reports/`，普通测试不额外制作压缩包、哈希清单和审计报告。需要长期保存的阶段验收摘要放在 `docs/verification/`，本机证据位置用普通文本注明。

`.local/dev/` 保存实际项目、成果、注册表和凭据，不能作为测试输出整体清理。夹具脚本的目录校验用于约束隔离运行位置，调用时也应明确指定本次测试目录。

## 评审 R2 验证

- [integration-aesthetic-transport.mjs](integration-aesthetic-transport.mjs)：大图请求预算、逐图身份核对、原始/部分/超限回执、本地重解析、坏图预检、项目恢复和成果摘要。用 `pnpm test:integration aesthetic-transport` 选择，全程使用本机模拟供应商。
- [probe-aesthetic-workset.mjs](probe-aesthetic-workset.mjs)：最多 512 张指定真实图片的隔离传输验证。参数依次为元数据审计 JSON、应用数据目录、System Prompt 名称。审计文件包含 collection_name、project_id、summary.errors 和 rows[].key；只读应用注册表及真实图片，评审状态写入独立测试项目，服务端仅监听 loopback，不调用商业 API。此入口不验证模型审美质量。

项目恢复使用公开 SDK 的 aesthetic.recoveryPackage(projectId) 和 aesthetic.restore(packageDirectory, destination)。后者要求同身份项目已关闭、目标目录不存在；验证成功后显式 openProject(destination)。旧的 aesthetic.backup 仍只备份评审账本。外部数据湖和凭据不在恢复包内。

## 按轮次采样与追加评审

- [integration-aesthetic-sampling.mjs](integration-aesthetic-sampling.mjs)：145 张合成图片，真实 SDK/HTTP/SQLite、16 请求并发上限、暂停重启、追加预算与幂等、原始回执和有效性摘要；在同一 48 次调用上限下比较旧组批、均衡轮次和动态轮次。只调用 loopback mock，不验证真实审美正确性。
- `node tooling/cargo-run.mjs run -p studio-storage --example aesthetic_sampling_probe -- COPIED_LEDGER STAGE_ID`：对显式复制到 `.local/test-runs/` 内的账本作离线补测规划。入口拒绝该目录之外的路径，不包含模型客户端；检查付费证据和配置哈希没有变化。运行前用 SQLite Backup API 从真实账本的只读连接复制，不能传入真实项目路径。
- 工作流 UI 验证现包含追加计划提交丢失响应、刷新恢复与精确重试；保存计划不会派发。规则见 [ADR 0032](../docs/decisions/0032-aesthetic-adaptive-sampling.md)。

- 同一采样集成入口也验证 `refine` / `refine_balanced` 的 v2 冻结版本、预算停靠、可选敏感度字段和 Davidson v2 快照；工作流 UI 在 2560×1440 验证新估计器及跨版本追加计划。算法与实验边界见 [ADR 0033](../docs/decisions/0033-aesthetic-neighbor-refinement.md)。

- `integration-aesthetic-recovery.mjs` 的数量边界夹具会在独立测试项目中生成真实的 10000000/10000001 条工作集成员，验证所有轮次模式的预检以及超限时不创建付费意图；该部分需要额外的本机磁盘和时间。它不执行千万图模型评审。`studio-storage` 单元测试另覆盖超过 32 MiB 的诊断分块写入及暂存恢复，见 [ADR 0034](../docs/decisions/0034-aesthetic-ten-million-admission.md)。

## 多站点数据湖

- [smoke-multibooru-ui.mjs](smoke-multibooru-ui.mjs)：使用 `multibooru-fixture.py` 或 `integration-multibooru.mjs` 生成的隔离三站湖，验证添加、查询与筛选；同时检查元数据排名在不兼容来源下仍可修改输入与参数、禁止提交，并能切回 Danbooru，覆盖 v1/v2 和草稿刷新恢复。
- [smoke-pixiv-ui.mjs](smoke-pixiv-ui.mjs)：Pixiv 合成湖的真实界面验收，复用资料浏览、查询、数据湖任务／计划和 API 设置；同时验证三种 Booru 与 Pixiv 的排序偏好隔离、旧会话和刷新恢复。不访问远端，不使用真实凭据。
- `pnpm test:pixiv-login` 构建原生验收宿主并运行 [smoke-pixiv-login.mjs](smoke-pixiv-login.mjs)：真实 WebView2、登录桥接及设置组件，验证私有配置隔离、Cookie 交接、取消、子窗口回收、错误与响应丢失后的幂等确认；网站和验证服务均为本机夹具，不使用真实凭据。单独原生规则测试为 `node tooling/cargo-run.mjs test -p studio-desktop --bin studio-desktop`。
- 原生宿主构建后，`node tooling/smoke-pixiv-login.mjs --live-login-page` 显式打开 Pixiv 官方登录页，检查邮箱／密码表单并关闭窗口；不会填写密码或提交登录验证。该命令会联网，不属于自动测试，也不证明真实账号登录成功。
- [validate-pixiv-production.mjs](validate-pixiv-production.mjs)：显式指定正在运行的引擎连接文件、正式两根目录、项目、作者及报告目录，执行公开目录采集、暂停续跑和增量复查。真实网络与归档写入不属于自动测试；同一报告目录禁止并发运行。命令示例见[来源采集](../docs/architecture/source-collections.md)。

- [integration-lake-recovery.mjs](integration-lake-recovery.mjs)：运行环境失败与替换、任务/凭据保留、读写位置交接、旧路径离线及重启补完；已加入完整集成。可设置 STUDIO_RELOCATION_TEST_ROOT 为另一卷中专用的 `.local/test-runs/r3-cross-volume-<唯一编号>`，外部夹具路径会写入本轮报告，须一并清理。
- [smoke-lake-updates-ui.mjs](smoke-lake-updates-ui.mjs)：现包含设置页的解释器错误/成功验证和数据湖迁移错误/成功操作，使用真实本地引擎与内置 worker。
- [lake-load-fixture.py](lake-load-fixture.py) 与 [verify-lake-combined-load.mjs](verify-lake-combined-load.mjs)：显式、非默认的大规模组合验收。先以 `<.local/test-runs/astra-load-唯一编号> prepare <来源清单.json>` 创建完整在线索引副本及选定媒体包，再把该目录传给 Node 验收入口。来源清单包含 kind、index_root、media_root；SQLite 源连接只读，更新和评审分别使用记录样本与 loopback mock。副本是负载镜像，缺少未选中的归档媒体，不能用于正式恢复。此测试可能占用上百 GB，结束后归档指标和日志，再清理专用目录。

- [integration-multibooru.mjs](integration-multibooru.mjs)：三站点隔离 fixture、预检/登记、身份、标签、原始 schema、缓存和重启；用 `pnpm test:integration multibooru` 选择。
- [multibooru-fixture.py](multibooru-fixture.py)：用标准库和仓库 DuckDB 运行库构建小型三站点夹具，不依赖真实湖或网络。
- [smoke-multibooru-ui.mjs](smoke-multibooru-ui.mjs)：参数为上述集成生成的 `multibooru-*` 目录，在独立应用与 Edge 无头浏览器中验收连续添加三个湖、多湖查询编辑/恢复/缓存、共同字段、精确标签、旧草稿迁移，以及无关来源离线时的工作集筛选；只对指定测试夹具临时模拟离线并恢复。
- [verify-multibooru.mjs](verify-multibooru.mjs)：显式传入 JSON 样本清单才读取真实湖；每湖最多 64 个记录、8 个预览。每项声明索引/图片根目录、library_id、generation 以及样本身份、原始 JSON/schema 摘要、标签和尺寸。只写独立测试应用及报告；首次会按需构建完整的应用身份/排序索引，未进行全湖媒体扫描。

实现和验证边界见 [ADR 0035](../docs/decisions/0035-source-registry-and-dispatch.md)及[验收记录](../docs/verification/2026-09-26-multibooru.md)。

美学评审执行与恢复：`pnpm test:integration aesthetic-execution` 使用本地模拟供应商验证 SSE/JSON 回执、分层超时、有限重试、同轮隔离、暂停/恢复及精确筛选预览。`node tooling/smoke-aesthetic-execution-ui.mjs <successful run>` 复用其隔离项目验证工作台并保存截图，不启动模型调用。设计与边界见 [0059](../docs/decisions/0059-aesthetic-streaming-and-recovery.md)。

OpenRouter 缓存专项：`pnpm test:integration aesthetic-cache`，使用 16 张合成图和 2 次本地模拟调用，验证阶段会话、冻结配置重关联及缓存费用汇总；不访问真实模型。
