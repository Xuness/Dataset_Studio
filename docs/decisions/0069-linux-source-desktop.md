# 0069：Linux x86_64 源码桌面运行

日期：2026-10-07

## 范围

Windows 与 Linux x86_64（glibc）共用前端、Rust 引擎、Tauri 桌面宿主和 Python 采集服务。Linux 保留原生窗口、目录选择器、剪贴板及独立 Pixiv 登录窗口。此次支持源码开发运行，不制作安装包，不迁移现有 Windows 数据湖、凭据或在途任务；Linux 从新项目和空数据湖开始。

## 平台边界

- `tooling/platform.mjs` 负责可执行文件后缀、动态库、Python 和界面测试浏览器的选择。引擎 sidecar 使用本机 Rust target；不在开发入口隐式交叉编译。
- DuckDB 1.5.4 分平台下载并检查固定 SHA-256，Linux 加载 `libduckdb.so`。`STUDIO_DUCKDB_LIBRARY` 为通用覆盖项，继续接受原有 `STUDIO_DUCKDB_DLL`。
- Windows 保留原有默认 Release 计算引擎；Linux 默认 Debug，便于源码迭代。二者桌面宿主均为开发构建。可用 `--engine-profile=debug|release` 显式选择本地引擎优化配置。
- 系统字体沿用 Windows 字体优先级，补充 `system-ui`、Noto Sans CJK SC / Noto Sans SC；Linux 环境必须安装中文字体。CSS 名称不能代替字体安装。
- Python 运行环境规范化父目录但保留解释器最后一段路径；POSIX 虚拟环境的 `bin/python` 通常是符号链接，完全解析会失去虚拟环境。目录隔离校验同时检查入口和真实解释器目标。
- Linux 任务子进程通过 `PR_SET_PDEATHSIG` 随拥有它的引擎退出，并检查 fork 与安装通知之间的父进程退出竞态；Windows 保留 Job Object。强制结束引擎后，不允许旧执行器与恢复任务重叠。

## 凭据

Windows 继续使用当前用户 DPAPI。Linux 使用 Secret Service（`secret-tool`）保存随机 AES-256 密钥，Rust 与 Python 使用相同的 AES-GCM 封装。密钥不保存在项目、控制数据库、命令行或环境变量中；系统密钥环必须在当前桌面会话可用。密钥环锁定、依赖缺失和读取失败均报错，不降级为明文。

用户数据目录 `dataset-studio/credential-key` 只保存随机密钥 ID；跨进程初始化使用同一个 `flock`。先成功保存和读取系统密钥，再原子发布 ID。每个封装包含 `DSC1`、32 字节小写十六进制密钥 ID、12 字节随机 nonce 和带 16 字节标签的密文；头部作为附加认证数据。密钥 ID 在密文中保留，即使 ID 文件丢失也能读取旧凭据；读取失败不得覆盖旧密钥。

Linux 采集请求去重记录使用 `protected:` 标记，Windows 继续写入 `dpapi:`，读取器支持两者；控制数据库结构不变。Linux 与 Windows 密文不承诺跨系统迁移。删除某个账号不删除系统主密钥，其他账号和历史请求仍需使用它。

## Pixiv 原生窗口

Linux 使用 WebKitGTK，Windows 使用 WebView2。主登录窗口均使用隐私会话，Cookie 由原生宿主传递给本机引擎，不经前端。启动前验证独立 Cookie 存储并等待清空完成。

WebKitGTK 的新独立隐私窗口不会自动共享 Cookie；登录页的子窗口通过 `window_features` / related view 继承父窗口的隐私上下文，并在 GTK 主线程创建。保留导航白名单、最多三个子窗口、会话超时、主窗口 IPC 限制与关闭回收。Windows 继续采用其异步弹窗路径。

第三方登录、验证码和网站自身的嵌入式浏览器政策仍可能影响登录；离线原生测试不代表真实账号已成功登录。

## 验证边界

Linux 测试使用隔离的 X11 显示、窗口管理器、D-Bus 和密钥环，运行产物位于 `.local/test-runs/`。`pnpm test:linux-native` 使用真实 Tauri / WebKitGTK 验证窗口、目录对话框、中文剪贴板、登录子窗口共享和多次登录的隔离；不以普通 Chromium 浏览器代替原生 WebView。

保留 Windows 凭据、引擎、采集与桌面回归。Linux CI 使用源码编译与针对性测试，不产出发行安装包。独立桌面的 Wayland、缩放、输入法和实际账号登录需要结合具体环境验收。
