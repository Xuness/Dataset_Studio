# Dataset Studio

面向大型图片数据湖的桌面工作环境。项目持续保存来源引用、选择、工作集与处理成果，各工具围绕项目中的数据工作。

当前版本：0.1 工程底座。当前交付和日常迭代均使用开发模式。

## 启动

在 Windows 上双击仓库根目录的 **启动开发版.bat**。

脚本会检查锁文件和依赖，增量编译 Debug 引擎，启动前端开发服务和 Tauri 桌面窗口。再次启动时会使用已有开发环境。开发控制台可查看编译结果。

命令行等效入口：

```powershell
Set-Location 'D:\Dataset\Dataset_Studio'
pnpm dev
```

- 修改前端和 CSS：Vite 热更新。
- 修改 crates 下的 Rust：自动增量编译、重新生成契约、重启开发引擎。已确认的任务检查点保留，界面重新连接。
- 修改桌面宿主：Tauri 开发工具负责重建窗口。
- 关闭窗口会结束当前桌面开发控制台；引擎独立运行，任务不依附页面。
- 停止开发引擎：在仓库目录运行 `pnpm engine:stop`，下次启动恢复未完成任务。
- 仅用浏览器调试：`pnpm dev:web`，入口为 `http://127.0.0.1:1420`。

需要 Node.js 22.12 或更新版本、pnpm 10.30.3、Rust 1.94 或更新版本、Visual Studio C++ 工具链与 WebView2。本机已经验证这些构建能力。脚本只调整子进程的 MSVC 编译环境。

## 首次使用

1. 新建项目，或打开包含 project.json 的项目目录。
2. 添加完整数据湖。Danbooru 需要 SSD 索引目录和机械盘图片湖目录；内置参考资料用于验证基础功能。
3. 在项目中浏览、查看单图、选择对象、保存工作集。选择属于项目，在切换来源和重开项目后保留。
4. 工具中的“生成数据清单”固定当前选择，使用独立执行器生成带来源版本的 JSONL 成果。任务面板显示进度、取消和成果下载。

当前机器的 Danbooru 位置：

```text
SSD 索引：D:\Dataset\Danbooru
图片湖：  E:\AI\AI_Dataset\Danbooru
```

数据湖按只读方式接入。当前浏览单位是 catalog.sqlite 中去重后的储存对象，使用 SHA-256 身份与包偏移定位图片；没有把历史观察条数或当前帖子数显示成图片总数。

## 数据与缓存

- `.local/dev/registry.sqlite`：开发环境的项目注册表与本机数据源位置。
- `.local/dev/projects/`：默认新建项目的位置；也可以在新建时选择其他父目录。
- 项目内的 project.json、project.sqlite、artifacts 与 .staging 分别保存身份、项目状态、正式成果与恢复所需暂存。
- `.local/engine-binaries/`：开发引擎的可重建二进制快照，避免运行中的 EXE 阻止增量链接。
- `.local/startup-install.log`、`.local/dev/engine.log`：启动与引擎日志。

`.local/dev` 包含实际项目数据，不是一个整体可丢弃的构建缓存。Git 忽略项目数据、设计参考、依赖、日志与二进制。

项目保存逻辑数据湖引用，本机位置留在应用注册表中。项目移动后可以重新打开；换到另一台机器时，需要重新关联本机数据湖位置。任务计划使用项目内相对路径，并校验固定输入内容。

## 开发与检查

```powershell
pnpm install --frozen-lockfile
node tooling/prepare-sidecar.mjs
pnpm contracts
pnpm check
pnpm test:integration
pnpm build
```

`pnpm build` 只构建前端资产用于验证。默认脚本不生成发布版或安装包。

接口以 Rust DTO 和 Utoipa 定义为准。生成的 OpenAPI 和 TypeScript 类型纳入 Git，CI 重新生成后检查漂移。前端功能通过 SDK 调用引擎，原生目录选择由应用层注入。

测试覆盖项目隔离、中文路径、独占写入、选择版本冲突、集合分页、只读数据包定位、路径边界、幂等提交、引擎中断、项目移动、检查点恢复、成果验证、取消与事件续接。

目录职责与实现边界见：

- [基础架构方案](docs/architecture/foundation-proposal.md)
- [0.1 实现状态](docs/architecture/foundation-status.md)
- [开发模式决策](docs/decisions/0001-development-foundation.md)

## 当前边界

这是可运行的工程底座。元数据公式、大模型打标和审美评估尚未实现。

查询每页最多 128 项，界面每页 48 项；选择修改每个请求最多 1000 项，可分次累积。任务逐批物化选择成员并生成清单，当前调度同时运行一个执行器。预览对单图原始字节设有 64 MiB 上限，对解码内存和同时读取数量设有预算。

成果始终保存在项目 artifacts 目录中。界面另存下载目前限制为 64 MiB，较大成果直接使用项目内的原文件，以控制前端内存。

已经验证本机真实 Danbooru 的只读分页、包偏移读取、内容校验与预览。尚未完成千万级全量性能、长时间运行和大规模选择的压测。

DuckDB 1.5.4 已完成真实归档只读兼容探测。完整元数据查询尚未接入浏览接口。可选诊断运行时通过 tooling/setup-duckdb.ps1 下载官方归档并校验固定 SHA-256；应用基础启动不依赖该 DLL。
