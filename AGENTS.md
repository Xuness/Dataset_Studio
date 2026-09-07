# Dataset Studio

- 产品术语：项目是持久化工作空间；工作集是项目内数据对象；工作区布局是呈现方式。工具不预设业务先后顺序。
- 前端功能模块只依赖公开 SDK 和 UI 接口；Tauri 调用集中在 platform，网络传输集中在 client。
- domain 和 application 不依赖 Tauri、HTTP 路由、SQLite 实现或传输 DTO。通过公开端口组装基础设施。
- 数据湖默认只读。新增来源通过适配器接入；项目写入由引擎统一负责。图片身份、来源条目、历史观察分别表达。
- 选择与查询采用后端引用及有界分页；避免全湖枚举、大 OFFSET、无界缓存、机械盘随机扫描。
- 修改公共 DTO/路由后运行 pnpm contracts；生成的 schema.d.ts 和 openapi.json 不手工编辑。
- pnpm check 执行类型、导入边界、契约、Rust 静态检查和测试。改动任务、存储或协议时运行 pnpm test:integration。
- Windows 的构建脚本通过 tooling/cargo.mjs 处理 MSVC 环境，保持当前机器全局配置不变。
- 参考风格、真实项目数据、缓存、模型、运行日志与生成二进制不提交到 Git。
- 记录会影响项目格式、兼容性、生命周期或模块职责的决定；不要把未完成能力描述为已实现。
