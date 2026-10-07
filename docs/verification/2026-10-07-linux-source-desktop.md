# Linux x86_64 源码桌面验收

日期：2026-10-07。范围与平台决定见 [0069](../decisions/0069-linux-source-desktop.md)，启动步骤见 [Linux 开发运行](../architecture/linux-development.md)。

## 环境与结果

Linux 验证使用本机 WSL 的 Ubuntu 26.04.1 x86_64，Node 22.22.1、Rust 1.94.0、Python 3.13.16。原生检查在独立 Xvfb / Openbox、D-Bus 和 GNOME Keyring 会话运行，使用真实 Tauri / WebKitGTK。Windows 使用当前源码 checkout 的独立测试数据。

| 检查范围 | 结果 |
| --- | --- |
| Linux Debug 引擎、桌面宿主与原生探针编译 | 通过 |
| Linux 源码开发入口 | `tooling/dev.mjs --engine-profile=debug` 启动实际应用；本机引擎连接、前端工作目录、原生窗口和进程内存状态通过，截图已检查 |
| Linux 原生桌面 / Pixiv | 13 项检查通过：窗口最大化及恢复、中文剪贴板、GTK 目录选择器、登录页及子窗口加载、Cookie 共享与隔离、远端窗口 IPC 限制、保存及取消 |
| Linux Rust | 凭据 2 项、数据源 45 项通过 |
| Linux Python | Linux 凭据、空湖和采集账号 25 项；更新凭据 2 项通过 |
| Linux 集成 | foundation、llm、collections、lake-updates、ranking、lake-recovery 通过；包含 Rust/Python 凭据封装互读、重启、空湖创建、采集、排名和解释器更换 |
| Windows 回归 | 凭据 1 项、数据源 45 项、运行环境 6 项 Rust 检查通过；凭据 Python、foundation / LLM / collections / lake-recovery 集成以及原生 Pixiv 6 组回归通过 |
| 静态检查 | `pnpm check`、修改 Python 文件的 Ruff、Windows/Linux 受影响 Rust crates 的 Clippy（all-targets，warnings 为错误）通过 |

联调修复了 POSIX 虚拟环境解释器最后一段符号链接被完全解析的问题：启动基础 Python 会丢失 venv 依赖。新的路径处理保留入口，同时校验真实目标的目录边界。恢复测试还改为通过目标 Python 的 `sysconfig` 定位 site-packages，修正旧 schema 11 断言为当前 14。

额外复现并修复了 Linux 主引擎异常退出后任务子进程继续运行的差异。foundation 回归从引擎各线程的 `/proc/.../children` 捕获真实执行器，强制结束主引擎后确认子进程已结束，再验证项目移动与任务恢复。

严格静态检查暴露的原有 OpenRouter 条件嵌套、分析任务行类型、测试模块位置及单批评审参数数量提示做了保持行为的整理；不改变美学评分、任务调度或网络协议。

## 边界

- 原生 Pixiv 使用本机离线页面和合成 Cookie，没有登录真实账号；网站风控、验证码、第三方账号登录未验证。
- 当前图形验收为 X11；独立 Wayland 桌面、不同缩放和输入法仍需目标环境验证。
- 使用新建项目与隔离湖；没有迁移日常 Windows 数据湖，没有执行全湖扫描、容量压测或完整 `check:full`。
- Linux CI 已加入 Ubuntu 24.04 源码编译和针对性检查，此次未推送，未声称 GitHub Actions 已运行通过。没有制作发行安装包。

本机日志在 `.local/logs/linux-compat-*`；截图与原生报告在 `.local/reports/linux-compat/`，均不随仓库分发。Linux 独立构建目录下的 `.local/logs/` 与 `.local/test-runs/` 保存原始 Linux 检查输出。
