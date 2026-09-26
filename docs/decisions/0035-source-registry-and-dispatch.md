# 0035：来源注册、站点配置与统一读取入口

日期：2026-09-26。对应[多站点计划](../plans/multibooru-sources-2026-09-26.md)与[验收记录](../verification/2026-09-26-multibooru.md)。

Studio 已支持 Danbooru、Yandere、Gelbooru 的统一格式数据湖，保留内置 demo。图片、来源记录和观察分别表达，工作集成员使用 `source_id + asset_id`；跨湖相同 SHA 和帖子 ID 不合并。

## 模块与依赖

- `studio-domain::sources` 定义能力、描述、标签编码与变更锚点；`studio-application::sources` 保存注册表、小端口和读取上下文。
- `studio-sources::profiles` 集中维护站点、规范化版本、缺失字段与字段标签；三个站点共用 `backends/canonical/catalog.rs` 和 SQLite/DuckDB reader。同格式的新站点通过配置及语义验证接入。
- `studio-sources::registry` 装配具体 Reader；引擎通过 `SourceService` 解析来源，`SourceDispatcher` 统一准入，`SourceRead` 持有资源租约。API、查询、预览、普通任务、排名与美学读取均经过此入口。
- `SourceIndexService` 从项目查询执行器拆出身份、帖子排序和 Rating 构建；查询成员、固定排名范围和项目发布仍由项目模块管理。
- `studio-resources::media_transform` 负责缩略图变换；PreviewService 继续负责订阅与渲染缓存。
- 元数据返回字段标签，旧 `danbooru.*` 与查询字段 ID 保持兼容；新站点使用自己的命名空间。美学分组政策归入 application，后端提供聚合事实并保留 SQL 下推。

边界检查阻止核心层反向依赖来源/资源实现，以及引擎业务直接创建具体 Reader。使用非 SHA 身份、仅支持浏览/媒体的测试 provider 验证了端口扩展边界。

## 识别、版本与保真

`GET /v1/source-adapters` 返回注册类型及能力；`POST /v1/source-probes` 只读识别目录、验证身份和水位。HF 站点来自正式湖的来源清单，目录名称不作身份依据。旧 Danbooru 无 HF 清单时显式选择类型。

新 HF 来源添加/重连时重新验证 catalog 与 analysis 水位，再登记到项目。旧 Danbooru 保留仅有图片目录也可登记的兼容入口；完整预检及元数据操作仍验证分析索引，不能将缺失分析索引解释为元数据可用。

`Source.kind`、library_id 和既有项目格式不变。新 HF 元数据查询带 `semantics_version`；旧 v1 的缺省值不进入持久化指纹。固定项目成果查询继续遵守 ADR 0021。

原始 JSON 保持原字符串，界面不通过 JavaScript 数字解析重新序列化。原始 Arrow schema 按二进制 hex 提供，上限 64 KiB；raw JSON 保留 128 KiB 上限，超限明确报告，原始文件仍保留在湖内。

标签按字面 U+0020 分隔。支持的特殊空白保留在标签内；UI 支持双引号 JSON 转义，并可从元数据生成精确查询草稿。数量、字节与字符限制仍有效。未知字段不填假值；无时区时间展示归一化缺失及其问题说明。HF 存储尺寸读取已有头部验证证据，与来源尺寸分别显示。

## 调度、缓存与生命周期

复用一个 ReadCoordinator 和 QueryBudget，按 Index / NativeQuery / Media / Decode 分类准入。湖数量不扩大媒体并发。上下文携带取消、优先级、请求身份和可选截止时间；资源等待、原生查询与媒体分块保留相应检查。

来源构建在线程启动前排队；身份/排序任务按来源、修订和类型合并。正式湖与应用派生缓存的所有权分开，清理不能进入原湖。

**实现中的生命周期决定：** 身份/排序的 `prepare` 是应用级基础设施构建请求。HTTP 返回“准备中”后，由索引服务持有任务，单个页面离开不撤销其他项目需要的构建。引擎停止取消构建，显式 Rating 预构建提供取消入口。预览继续使用“最后一个订阅者退出即取消”。这里没有把短暂轮询当作构建租约，避免每次返回准备状态就撤销构建。

Studio 自有缓存仍在应用数据目录，未迁移现有 Danbooru 缓存。元数据追加不使内容未变的预览失效；项目固定成员和排名索引不会因共用一个湖而混用。

## 工具与范围

`POST /v1/projects/{id}/source-requirements` 按实际输入范围判断投影要求。Danbooru MetaRecall 仍要求专属投影，前后端都拒绝不符合要求的新湖或混合范围。

本次不提供远端 API 补全、每日更新或动态插件加载。不同物理格式的高级查询/索引需要对应后端实现；媒体端口不强迫它们实现 Booru 字段。
