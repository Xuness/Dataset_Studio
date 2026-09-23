# 开发与验证工具

以下命令均从仓库根目录运行。日常启动、所需环境及数据目录见[仓库说明](../README.md)，架构与历史验收见[文档导航](../docs/README.md)。

脚本使用自身位置定位仓库根目录，统一保留在 tooling 顶层。测试源码和夹具生成器随 Git 保存；运行产生的数据、截图和日志使用 `.local/`。

## 环境与常用命令

工作台前端验收：先运行 `node tooling/integration-aesthetic-analysis.mjs` 和 `node tooling/integration-aesthetic.mjs`，再将各自成功的 `.local/test-runs/` 目录传给 `node tooling/smoke-workbench-ui.mjs <analysis-fixture>` 与 `node tooling/smoke-aesthetic-ui.mjs <evaluation-fixture>`。前者使用端口 1447，后者使用 1439；均只操作隔离合成项目。`ui-workbench.mjs` 提供功能标签/菜单的共享测试导航。

浏览布局回归集成在 `node tooling/smoke-ranking-browse-ui.mjs` 中，由 `ui-browser-layout.mjs` 验证。首要基准为 2560×1440、100% 缩放，另覆盖任务栏可用高度、150% 远程缩放和较小窗口；实际发送滚轮事件，检查右侧标签、草稿随停靠/隐藏/刷新恢复、筛选输入可达、图片滚动、查询侧栏、分页及全局底栏边界。业务回归同时验证 Rating 和 Tag 包含/排除筛选。

Windows 开发环境需要 PowerShell 7、Node.js 22.12 或更新版本、pnpm 10.30.3、Rust 1.94 或更新版本、Visual Studio C++ 工具链和 WebView2。集成测试及部分界面验证还需要 **Python 3.11 x64**，`python --version` 应能找到对应解释器；本机夹具验证使用 3.11.9，CI 选择 3.11 x64。

Python 夹具只依赖标准库，通过 ctypes 加载 `vendor/duckdb/duckdb.dll`。先运行 `pwsh -File tooling/setup-duckdb.ps1` 准备固定版本的 DLL，再执行相关验证；无需 pip 安装。界面冒烟脚本还需要已安装的 Microsoft Edge，原生窗口验证需要可交互的 Windows 桌面。

| 入口                                      | 用途                                                             |
| ----------------------------------------- | ---------------------------------------------------------------- |
| `pwsh -File tooling/start-dev.ps1`        | 安装依赖、准备 DuckDB 并启动开发环境；也可双击根目录启动器       |
| `pnpm dev` / `pnpm dev:web`               | 已准备依赖后的桌面 / 浏览器开发环境                              |
| `pnpm engine:stop`                        | 结束当前开发引擎                                                 |
| `pnpm contracts` / `pnpm contracts:check` | 生成公开契约 / 核对契约漂移                                      |
| `pnpm check`                              | 类型、lint、依赖边界、契约、Rust 格式与 Clippy、Rust 和 SDK 测试 |
| `pnpm test:integration`                   | 20 组隔离引擎集成脚本                                            |
| `pnpm test:launcher`                      | Windows 启动器与引擎进程管理回归                                 |
| `pnpm test:clipboard`                     | Windows 原生剪贴板及 Win+V 历史验证，会写入系统剪贴板            |
| `pnpm build`                              | 验证前端构建，不生成安装包                                       |

## 开发、构建与生成

- 启动与进程：[start-dev.ps1](start-dev.ps1)、[dev.mjs](dev.mjs)、[stop-engine.mjs](stop-engine.mjs)、[engine-process.mjs](engine-process.mjs)、[engine-profile.mjs](engine-profile.mjs)。
- Rust 工具链与 sidecar：[cargo.mjs](cargo.mjs)、[cargo-run.mjs](cargo-run.mjs)、[prepare-sidecar.mjs](prepare-sidecar.mjs)。
- 原生元数据运行库：[setup-duckdb.ps1](setup-duckdb.ps1)，下载时核对固定 SHA-256。
- 公开契约：[contracts.mjs](contracts.mjs)；OpenAPI 和 schema.d.ts 由源码生成并纳入 Git，不手工编辑。
- 图标：[icons.mjs](icons.mjs)；通过 `pnpm icons` 从 UI SVG 同步桌面图标。
- 前端导入边界：[boundaries.mjs](boundaries.mjs)，由 `pnpm lint` 调用。

## 自动检查与隔离集成

Rust 测试位于 crates 内，`pnpm test` 同时执行 SDK 检查 [client-foundations.mjs](client-foundations.mjs) 和 [client-media.mjs](client-media.mjs)。启动器检查为 [test-engine-process.mjs](test-engine-process.mjs) 和 [test-launcher.mjs](test-launcher.mjs)。

`pnpm test:integration` 按 package.json 中的顺序执行以下脚本，使用独立引擎与合成数据：

| 范围                       | 脚本                                                                                                                                                                                                                                                                                                                                                                                                       |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 项目、范围、工具和读取资源 | [integration.mjs](integration.mjs)、[integration-scopes.mjs](integration-scopes.mjs)、[integration-tools.mjs](integration-tools.mjs)、[integration-resources.mjs](integration-resources.mjs)、[integration-artifact-scale.mjs](integration-artifact-scale.mjs)                                                                                                                                             |
| 查询缓存、设置和缓存清单   | [integration-query-cache.mjs](integration-query-cache.mjs)、[integration-cache-settings.mjs](integration-cache-settings.mjs)、[integration-cache-inventory.mjs](integration-cache-inventory.mjs)                                                                                                                                                                                                           |
| 排名与浏览                 | [integration-ranking.mjs](integration-ranking.mjs)、[integration-ranking-v2.mjs](integration-ranking-v2.mjs)、[integration-scoped-browse.mjs](integration-scoped-browse.mjs)、[integration-ranking-browse.mjs](integration-ranking-browse.mjs)、[integration-ranked-cache-lifecycle.mjs](integration-ranked-cache-lifecycle.mjs)、[integration-ranking-duplicates.mjs](integration-ranking-duplicates.mjs) |
| 对象管理                   | [integration-management.mjs](integration-management.mjs)                                                                                                                                                                                                                                                                                                                                                   |
| LLM 与系统指令             | [integration-llm.mjs](integration-llm.mjs)、[integration-system-prompts.mjs](integration-system-prompts.mjs)                                                                                                                                                                                                                                                                                               |
| 美学评审与离线统计         | [integration-aesthetic.mjs](integration-aesthetic.mjs)、[integration-aesthetic-analysis.mjs](integration-aesthetic-analysis.mjs)                                                                                                                                                                                                                                                                           |

另外包括 [integration-aesthetic-recovery.mjs](integration-aesthetic-recovery.mjs)：R0/R1 创建中断、存储故障、发送屏障、候选处置、冻结模板、请求 ID 和真实百万行容量准入。该脚本显式构建 Debug `test-faults` 引擎，通过隔离文件注入故障，结束后恢复普通构建；不调用商业供应商，不证明百万图片吞吐。

普通夹具引擎默认使用 Debug；需要验证已有 Release 产物时可设置 `STUDIO_ENGINE_PROFILE=release`。单独执行集成脚本前，应先准备 DuckDB 和相应引擎构建。

## 界面与原生窗口验证

按需要用 `node tooling/<脚本名>.mjs` 执行。各脚本的桌面、端口和引擎前置条件以脚本开头及仓库说明为准。

- 排名：[smoke-ranking-ui.mjs](smoke-ranking-ui.mjs)、[smoke-ranking-browse-ui.mjs](smoke-ranking-browse-ui.mjs)、[smoke-ranking-pagesize-ui.mjs](smoke-ranking-pagesize-ui.mjs)。
- 管理与设置：[smoke-management-ui.mjs](smoke-management-ui.mjs)、[smoke-settings-ui.mjs](smoke-settings-ui.mjs)。原生设置窗口验证前需关闭已有开发窗口与前端服务。
- 模型与评审：[smoke-llm-ui.mjs](smoke-llm-ui.mjs)、[system-prompts-ui.mjs](system-prompts-ui.mjs)、[smoke-aesthetic-ui.mjs](smoke-aesthetic-ui.mjs)。
- 剪贴板：[smoke-clipboard-ui.mjs](smoke-clipboard-ui.mjs)，通过 `pnpm test:clipboard` 先构建所需原生探针；要求开启系统剪贴板历史。
- 原生辅助脚本：[native-ui-controls.ps1](native-ui-controls.ps1)、[clipboard-history.ps1](clipboard-history.ps1)。

## 指定真实数据的有界验证

以下入口按参数读取明确指定的数据湖或对象，参数与预算说明见[仓库说明](../README.md#开发与检查)。运行结果只证明所指定样本与条件下的路径，不代表全湖质量或吞吐。

- [verify-metadata.mjs](verify-metadata.mjs)：指定对象的元数据读取。
- [verify-scopes.mjs](verify-scopes.mjs)：指定对象 / 帖子条件的查询路径。
- [verify-reads.mjs](verify-reads.mjs)：指定对象的读取预算、应用缓存和重启。
- [verify-ranking.mjs](verify-ranking.mjs)：真实元数据的有界排名试点。

## 夹具与结果归档

- [engine-fixture.mjs](engine-fixture.mjs)：独立引擎启动、请求、等待与停止。
- [client-fixture.mjs](client-fixture.mjs)：测试使用的 SDK 装载。
- [llm-fixture.mjs](llm-fixture.mjs)：本机模拟供应商服务，不调用商业模型。
- [ui-fixture.py](ui-fixture.py)、[query-fixture-update.py](query-fixture-update.py)：生成与推进隔离 UI / 查询数据湖。
- [ranking-fixture.py](ranking-fixture.py)、[ranking-fixture-update.py](ranking-fixture-update.py)：生成有界排名数据湖及追加模拟更新。

运行产物统一保存在 `.local/test-runs/`，命令日志放在 `.local/logs/`。需要长期保留的验收摘要、日志和截图归档到 `.local/reports/` 后，再按明确范围清理本次临时文件与空目录。仓库内的历史验收摘要放在 `docs/verification/`，本机证据位置用普通文本注明。

`.local/dev/` 保存实际项目、成果、注册表和凭据，不能作为测试输出整体清理。夹具脚本的目录校验用于约束隔离运行位置，调用时也应明确指定本次测试目录。
