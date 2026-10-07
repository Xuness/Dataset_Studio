# Linux 源码开发运行

目标：Linux x86_64、glibc、原生 Tauri 桌面。当前使用新项目和从零创建的数据湖，不搬运 Windows 的 `.local`、`node_modules`、Python 虚拟环境或构建目录。

## 依赖

需要 Node.js 22.12+、pnpm 10.30.3、Rust 1.94+、Python 3.11–3.13。Ubuntu / Debian 系统依赖示例：

```bash
sudo apt update
sudo apt install build-essential pkg-config curl libwebkit2gtk-4.1-dev \
  libgtk-3-dev libssl-dev libxdo-dev libayatana-appindicator3-dev librsvg2-dev \
  libsecret-tools gnome-keyring fonts-noto-cjk fonts-noto-color-emoji
```

其他发行版参考 [Tauri 系统依赖](https://v2.tauri.app/start/prerequisites/#linux)。系统应提供已解锁的 Secret Service 密钥环和桌面 D-Bus 会话，通常由桌面登录负责。仅有 WSLg 的精简环境不一定带有完整的桌面密钥环会话；应先配置密钥环再保存 API Key / Cookie。程序不会创建明文凭据后备文件。

系统默认 Python 为 3.14 时，需要另外安装 3.11–3.13（例如通过 uv），无需替换系统 Python。

仅使用 WSLg、没有完整桌面登录会话时，可以在新的 D-Bus 会话中启动程序：先运行 `dbus-run-session -- bash`，在该 shell 中运行 `gnome-keyring-daemon --start --components=secrets`，再按下文执行启动脚本。首次保存凭据时按系统提示创建或解锁密钥环；不要将密钥环密码写进启动脚本。

## 安装与启动

在 Linux 自己的文件系统中放置源码并运行：

```bash
bash ./启动开发版.sh
```

启动器自动检查 pnpm 依赖和固定版本的 DuckDB，并将每次安装、运行库检查及启动日志写入 `.local/logs/`。可从任意工作目录调用脚本，支持中文和空格路径。参数原样交给现有开发入口：`--web` 使用浏览器，`--engine-profile=debug|release` 选择引擎配置，`--help` 查看说明。系统工具和 GTK / WebKitGTK 依赖需预先安装。

首次采集前准备数据湖 Python 环境（已有环境可跳过）：

```bash
pnpm setup:lake --dev --python=/path/to/python3.13
```

如果 `python3` 本身就是支持的版本，可省略 `--python`。启动器的 DuckDB 安装步骤优先使用 `PYTHON` 指定的解释器，其次使用已有项目虚拟环境，最后使用 `python3`。下载 DuckDB 需要能访问 GitHub Releases，使用 curl 的标准代理设置；也可将对应版本的 `libduckdb-linux-amd64.zip` 放在 `vendor/duckdb/` 后重新执行，仍会校验固定 SHA-256。

`pnpm dev` 自动本地编译 Debug 引擎与原生桌面窗口；前端热更新，后端代码变更后重建并恢复连接。`pnpm dev:web` 提供辅助的本机浏览器开发入口。需要性能测试时可显式选择 `pnpm dev --engine-profile=release`，这只是本地引擎优化配置，不会制作发行包。

第一次启动后，在“设置 → 数据湖 API”选择 `.local/runtime/lake-worker/bin/python` 和新的共享更新状态目录。然后新建项目，在“工具 → 数据湖”从空目录创建所需数据湖。API Key、采集凭据与 Pixiv Cookie 在当前 Linux 用户的密钥环保护下独立保存。

Pixiv 设置中的“通过浏览器登录”会打开独立的原生 WebKitGTK 登录窗口。登录完成后返回 Studio 选择“已登录，验证并保存”。目录选择器和剪贴板也使用桌面原生接口。第三方账号登录和验证码是否放行由网站决定，可继续使用已有 Cookie 导入入口。

## 针对性检查

静态检查：`pnpm check`。Rust、Python、集成套件继续按 [tooling 导航](../../tooling/README.md) 选择。Linux 图形测试额外依赖：

```bash
sudo apt install dbus-x11 xvfb xauth openbox xdotool imagemagick
bash tooling/linux-test-session.sh pnpm test:linux-native
bash tooling/linux-test-session.sh pnpm test:integration llm collections
bash tooling/linux-test-session.sh pnpm test:lake tests/test_linux_credentials.py
```

测试会创建独立显示、D-Bus、密钥环和 XDG 数据目录，不触碰日常凭据。原生测试使用合成 Cookie 和本机页面，不要求真实 Pixiv 账号。产物位于 `.local/test-runs/`。WSL Ubuntu 可以执行这些 Linux 测试；真实 Wayland 桌面、缩放、输入法和账号登录仍需在目标桌面环境核对。
