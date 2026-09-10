# 桌面剪贴板接入

2026-09-10

## 问题与验证

Windows 上在 Studio 选中文本后按 Ctrl+C，其他应用可直接粘贴，但 Win+V 历史没有收录。使用相同文字在独立 Tauri / WebView2 窗口中对照：WebView2 默认 Ctrl+C 与 `navigator.clipboard.writeText()` 均只有当前剪贴板内容；宿主通过 `tauri-plugin-clipboard-manager` 写入后，Windows 历史接口立即返回对应文字。原生 HTML 写入同时保留 Text 和 HTML Format。

这将问题定位到 WebView2 默认复制路径与 Windows 历史收录的衔接。[上游同类报告](https://github.com/MicrosoftEdge/WebView2Feedback/issues/5650) 提出了窗口归属解释；本次不将尚未验证的 Windows 内部判定机制当作结论。

## 决定

- `apps/desktop/src/platform/clipboard.ts` 统一调用 Tauri 原生写入；功能模块与 UI 包不直接调用 Tauri。
- UI 包提供 `ClipboardProvider` 和可注入的 `ClipboardWriter`。所有 `CopyButton` 使用同一个接口；独立浏览器模式保留 Web Clipboard API。
- 桌面模式通过冒泡阶段的可信 `copy` 事件处理文字选择，覆盖键盘复制与触发同一事件的复制命令，不只监听 Ctrl+C 按键。
- 原生调用前同步阻止 WebView2 默认写入，避免两条路径互相覆盖。只传递当前选择的文字及 HTML 片段，不监听或延迟重写全局剪贴板。
- 普通输入框和文本域使用选区偏移；Chromium 的 number/email 输入使用 Selection 文本，因为这些类型不提供 selectionStart/selectionEnd。
- 保留密码字段保护、自定义已取消的复制事件和纯图片复制行为。空选区不会误用页面上的旧选区。原生失败显示可重试提示。
- 只授予原生文字与 HTML 写入权限；不为产品增加剪贴板读取或历史读取权限。

HTML 文字复制保留片段标记、内联样式和换行；此接入不承担页面外部样式表的打包。项目数据格式、引擎协议和数据湖访问不受影响。

## 验收

`pnpm test:clipboard` 构建 `clipboard_probe` 原生示例，加载实际的 UI Provider、复制按钮和平台接口。示例使用生产依赖与权限，但使用独立 WebView2 配置，不连接引擎或项目。测试覆盖中文、多行和 emoji、普通选区、可编辑区域、富文本、数字和邮箱输入、共享复制按钮、空选区、密码字段、组件自定义复制、失败提示与恢复，并通过 Windows 历史 API 核对条目。

Windows 示例需要和主程序一样链接 Common Controls v6 清单；`build.rs` 将 tauri-build 已生成的资源库同时传给 MSVC example 目标。

该测试会复制少量合成文字，因此作为显式执行的本机验收，不加入 `pnpm check`。普通静态检查和前端构建继续执行 `pnpm check`、`pnpm build`。
