# 项目升级与只读元数据

日期：2026-09-07。对应开发版本 0.2.0。

## 项目版本与备份

项目清单继续使用 format_version=1，项目数据库使用 PRAGMA user_version=2。两个版本分别管理：清单 v1 可对应数据库 v1 或 v2；数据库 v1 打开时升级，其他未知版本拒绝。

v2 建立 schema_migrations 账本，记录版本、执行时间与备份相对目录。新建项目直接在一个事务中建立 v2。schema.sql 固定保留原始 v1 模式，作为初始化基础和旧项目测试夹具；以后不在这个文件中追加新版定义。

升级过程先取得项目独占锁，以只读连接检查版本，再打开现有数据库的写连接。未知版本不会切换 journal_mode。升级前使用 SQLite Online Backup API 复制已提交的数据库状态，包括 WAL 中的内容；备份转换为独立可读取的 SQLite 文件，执行 integrity_check，保存原始 project.json 和 backup.json，并同步文件。随后在 IMMEDIATE 事务内执行迁移、外键检查、账本写入和版本更新。

备份位于项目内 .backups/v1-to-v2-*。迁移失败回滚，错误包含诊断目录。重复打开 v2 不再次备份或迁移。此次升级不修改 artifacts、.staging 或清单文件，备份也不复制这些成果文件。

恢复材料的使用边界：失败事务已回滚，原项目通常可以在修正问题后直接重试。需要手工恢复到升级前状态时，先关闭占用该项目的引擎，完整保留当前项目目录，再从备份建立一个独立恢复目录，使用备份的 project.json/project.sqlite，并从原项目保留对应 artifacts/.staging；不要把旧数据库直接覆盖到仍打开的项目或遗留 WAL 上。新版本再次打开 v1 会重新执行迁移。

## 身份与关联

现有 AssetKey 保持不变：数据湖 ID + 存储内容 SHA-256。工作集、选择、任务输入均继续引用这个身份。

| 层次 | 来源 | 语义 |
| --- | --- | --- |
| 存储对象 | SQLite objects.sha256 | 去重后的存储字节；可对应多个来源记录 |
| 来源记录 | DuckDB assets.asset_id | 一次图片归档关联，携带 post_id、observation_id、源 MD5 与存储配置 |
| 来源条目 | post_id | Danbooru 逻辑条目编号；多个观察可能记录不同时间和内容 |
| 历史观察 | observations.observation_id / row_id | 单条元数据观察及其时间精度、来源、入库时间与提交水位 |
| 原始元数据 | raw_metadata.observation_id | 观察对应的原始 JSON、格式标记与 schema_id |

服务返回同一 SHA 的全部关联，采用有界分页。同条目的历史观察标记为 same_post；与所选 assets.observation_id 相同的观察标记为 asset_origin。界面会说明 same_post 可能对应已变化的图片。服务不借助 current_posts 选择唯一正确记录，也不把观察条数显示成图片数量。

公共字段使用 rating、tags、source_width、source_height、source_url 等语义名称，来源专有字段使用 danbooru.*。每项带类型、原始列名、缺失原因及截断标志。整数字段通过十进制字符串传输，避免 JavaScript 丢失 64 位精度。时间按 UTC 输出，并保留 time_quality。空值、空字符串、空标签和 false 分别表达。

真实存储尺寸本轮保持“尚未检查图片”。observations.image_width/image_height 仅作为来源尺寸返回，不能证明压缩或转码后的实际尺寸。示例图片的 640×800 来自确定的生成器。

## 读取会话与版本边界

每次元数据请求：

1. 从登记位置解析数据湖身份和 CURRENT，打开 SQLite 只读事务，读取 seq 与存储对象。
2. 以 READ_ONLY 打开同一 generation 的 DuckDB，建立读取事务，读取 MAX(applied.seq)。
3. 两个水位必须相等；携带的版本或分页游标必须匹配本次读取。
4. 查询完成后重新读取 CURRENT，并通过新 SQLite 连接检查水位；发现切换则拒绝返回旧结果。
5. 返回数据湖身份、generation、两个水位及读取语义，随后关闭数据库连接。

该协议提供每个请求内的事务读与版本检查。它不形成跨 SQLite/DuckDB 的原子事务，也不提供历史快照；同 generation、相同水位不是独立的快照证明。不同请求会重新建立事务，依赖数据湖按其提交协议维护水位。

DuckDB 原生 DLL 由应用自行管理，固定为 1.5.4，通过官方压缩包的 SHA-256 校验安装。DLL 可留在进程内，数据库句柄按请求释放，以便归档写入进程重新取得文件锁。归档 runtime/Python 不参与产品调用链。

参考：[DuckDB 进程并发模型](https://duckdb.org/docs/current/connect/concurrency)、[C API 生命周期](https://duckdb.org/docs/current/clients/c/connect)、[C API 结果释放要求](https://duckdb.org/docs/current/clients/c/query)、[SQLite Online Backup API](https://www.sqlite.org/backup.html)。

## 读取成本与限制

本机索引的 assets 只有 asset_id 与 post_id 索引，没有 SHA 反向 ART 索引；SHA 关联的查询计划仍含列扫描。observations 有 observation_id 主键以及 row_id、post_id 索引；raw_metadata 以 observation_id 为主键。SQLite 的 observation_lookup 不能直接承担 SHA 反查。

本轮使用投影查询与精确记录查找，避免读取整张 observations、raw_metadata 或 details_json。四个 SHA 位置的小样本结果足以支持当前单对象检查入口，尚不需要创建全湖反向索引。后续高频浏览、批量查询或实际延迟不满足需求时，再设计应用侧可重建索引，并记录建设成本。

- 每个引擎同时进行一个元数据读取；占用时返回 SOURCE_BUSY。媒体和任务使用各自通路。
- 每页来源记录默认 20、最多 50；观察默认及最多 10；采用身份游标，不使用 OFFSET。
- DuckDB 配置单线程、256 MB 内存预算、禁用临时溢写与外部访问。256 MB 是 DuckDB 内存管理器预算，不是整个引擎的 RSS 硬上限。
- 原生读取预算为 8 秒，以 duckdb_interrupt 协作中断。DLL/数据库打开耗时计入预算；操作系统文件 I/O、连接关闭和调度不受硬实时保证，不能把它解释为 HTTP 必然在 8 秒内返回。
- 通用文本与标签列最多显示 8192 字符，并返回截断标志；原始 JSON 按需读取，最多 128 KiB，超限返回 too_large 和字节数，不返回截断 JSON。
- 每次原生查询最多转出 101 行、64 列和 2 MiB 文本。前端只缓存短期检查结果，焦点切换会取消过期 HTTP 请求；已进入原生读取的请求由预算约束完成。

## 模块边界

MetadataAdapter 是 application 层端口。来源实现管理读取会话、关联与字段语义，协议层生成 OpenAPI 和 TypeScript 类型，SDK 负责传输，useMetadata 管理检查状态，MetadataInspector 负责显示。

三个 GET 接口均先检查来源属于当前项目，再检查记录属于当前存储对象、观察属于所选记录。检查模块没有选择、工作集或任务的写入接口。

当前不包含复杂元数据筛选、结果引用、完整算子注册、公式、模型标注、派生列，也不解码 source_schemas.schema_ipc。后续工具的使用次序保持开放。
