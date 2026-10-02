# 契约一 数据关系与 schema

版本：1。定稿日期：2026-10-03。适用：新 Pixiv 湖的统一归档 2 与在线 3；旧湖通过原读取路径兼容。本文是实现规范，配套 [SQL](online-v3.sql) 是空库目标结构，不能对既有湖直接执行。

## 身份与编码

库及任务控制 ID 使用 UUID。Pixiv 用户和作品 ID 使用无前导零的十进制字符串，最多 20 位，不经过 JavaScript 浮点数。内部不可变记录 ID 使用 64 位十六进制 SHA-256；数据库整数行号只用于定位和索引，不作为跨重建身份。

`H(...)` 沿用 `studio_lake.util.stable_id`：参数组成 JSON 数组，UTF-8、`ensure_ascii=False`、紧凑分隔符，再取 SHA-256。参数中的控制对象先递归按键排序并序列化为规范字符串，禁止 NaN；ID 由后端统一生成，客户端将其视为不透明值。标签等来源字符串不做大小写或 Unicode 归一化。时间规范为带六位小数的 UTC `...Z`，来源原值保留在原文中。

| 身份        | 确定方式                                                                                           |
| ----------- | -------------------------------------------------------------------------------------------------- |
| 捕获 ID     | `H("capture-v1", library_id, request_receipt_id)`；每次实际响应有独立收据，重放沿用原收据          |
| 作品观察 ID | `H("work-observation-v1", capture_id, work_id, normalizer_version)`                                |
| 作者观察 ID | `H("author-observation-v1", capture_id, author_id, normalizer_version)`                            |
| 媒体清单 ID | `H("media-manifest-v1", capture_id, work_id, detail_observation_id或空串, normalizer_version)`     |
| 媒体条目 ID | `H("media-entry-v1", manifest_id, slot_key)`；静态页为 `page:0` 等，Ugoira 为 `animation:0`        |
| 资产记录 ID | `H("asset-record-v2", media_id, representation, recipe_id, acquisition_receipt_id, stored_sha256)` |
| 发现快照 ID | `H("discovery-v1", capture_id, stream_key, "pixiv-plan-v1")`                                       |
| 文件对象 ID | 对实际保存字节计算 SHA-256；不使用 URL、作品 ID 或来源声称的哈希替代                               |

资产 ID 包含取得收据，因此再次验证相同文件可以保存新的验证时间；同一收据重放不会新增记录。两页即使文件哈希相同，也因媒体条目不同保留不同来源资产。来源原件 SHA 与编码后 SHA 分开。

捕获保存 request_receipt_id，资产保存 acquisition_receipt_id，规范化观察与清单保存各自 normalizer_version；捕获的 adapter_version 只描述当时取数代码。重新解析同一原文产生带新规范化版本的派生记录，不改写旧捕获。当前投影只从当前 profile 声明支持的规范化版本中选择，不能以记录 ID 的字典序随机选择新旧解析语义。

## 事实记录与在线结构

[online-v3.sql](online-v3.sql) 完整定义字段、索引、外键与基础约束。事实表为：

- `visibility_contexts`：非敏感账号引用、实际观察到的显示条件、验证程度与可比较摘要。未知条件保持未知；凭据及登录页面不进入这里。
- `captures`：接口、非敏感请求条件、响应状态、时间、解析版本和完整原文。认证与挑战响应只进入脱敏运行诊断；普通业务响应原文进入捕获档案。
- `authors`、`author_observations`、`works`、`work_observations`：实体身份与历史观察。完整作品详情产生作品观察；搜索缩略条目只用于发现，不覆盖完整详情。
- `tags`、`work_tags`：字面标签字典及每次作品观察的标签顺序、翻译和锁定信息。
- `media_manifests`、`media_entries`、`animation_frames`：一份清单及其中的页或动画条目、顺序、尺寸、实际 URL 变体和帧时序。
- `objects`、`assets`：文件位置、媒体类型、实际存储尺寸，以及取得或派生记录。
- `discovery_snapshots`、`discovery_members`：作者目录、关注、收藏、推荐等某次分页返回及成员；保留范围与结束语义。

`work_versions`、`media_asset_versions`、`changes`、标签 FTS 是投影。发布、水位、构建进度和租约是运行性结构。控制任务及凭据不放入来源在线库。

站点专属标记与统计放在版本化的 `source_fields_json`，Pixiv 使用 `pixiv.x_restrict`、`pixiv.ai_type`、`pixiv.bookmark_count`、`pixiv.view_count`、`pixiv.like_count` 等键，数值保持整数或 NULL。常用字段建立固定表达式索引，查询字段只能由已注册 profile 绑定；不允许客户端提供任意 JSON 路径或 SQL。该结构是有界的规范化查询投影，完整未知字段仍以捕获原文为准。

事实约束除 SQL 之外还必须检查：

1. 重复实体 ID 可复用；重复不可变记录 ID 必须与原记录逐字段一致，不能用 `INSERT OR IGNORE` 掩盖内容冲突。
2. 媒体清单所属作品、详情观察、条目及资产必须一致。槽位与 ordinal 在清单内唯一；完整静态页 ordinal 连续为 `0..N-1`。
3. 完整清单的条目数与返回内容、可用的详情页数一致。响应失败、页数冲突或异常空列表不能产生完整清单。
4. Ugoira 的清单只有一个动画条目；帧 ordinal 连续，文件名唯一，时长非负。包内文件与帧表按有界检查验证，封面引用第一帧。
5. 不同格式和字段的 NULL 表示未知或缺失，不补为零、否或默认分级。
6. 所有关系引用指向已归档记录或同批记录。事实表、索引和重建使用同一引用规则。

## 归档布局与类型映射

新 `library.json` 声明 `format_version=2`、`site="pixiv"`、独立 `library_id`。每批继续位于 `segments/<batch_id>/`，包含 manifest、Parquet 记录、可选 `media.tar` 和 `collection_replay.json`。无记录的可选表文件可省略，manifest 说明实际文件、角色、schema、行数、字节和摘要。

Parquet 文件名对应事实表名。列默认与 SQL 事实表一致，排除 SQLite 的 `row_id`、`object_row`、`tag_id`、`commit_seq`、`first_seq` 等派生定位字段。具体例外固定如下：

| 文件                | 与在线字段的差异                                                                                                                                                |
| ------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `captures.parquet`  | 使用 `raw_body: binary` 保存 `requests.Response` 解传输编码后的实体字节，替代 `raw_zlib`；保留原文字节数和 SHA。HTTP 原始 Content-Encoding 放在非敏感请求记录中 |
| `objects.parquet`   | 使用批次内 `pack_file`、`member_name`、`offset`、`length` 替代在线 `pack_path`；发布器生成库内相对位置                                                          |
| `work_tags.parquet` | 使用字面 `tag` 替代整数 `tag_id`，字典 ID 在每个在线库重新建立                                                                                                  |
| 其余事实表文件      | 使用 SQL 中相应事实字段；不存在 Parquet 外键，但写入、发布和重建均验证同样的关系                                                                                |

字符串、JSON 文本和规范时间映射 Arrow `string`；整数映射 `int64`；布尔语义字段映射 `bool`；原文映射 `binary`。空值按 SQL 可空性保留。Parquet 使用 Zstd 和嵌入 schema，逐批固定 schema 标识；不能靠猜列名读取未来版本。

原文可由多条结构化记录引用，作者资料不按图片复制。在线按原文字节压缩保存，检查长度、SHA 和压缩流完整性。沿用原文接口 128 KiB 的返回上限，超限返回 `too_large`，原文仍在归档。结构化标签 FTS 使用内部标签 ID token，避免把中文或包含空格的标签拆成不同语义。

TAR 对象成员仍以文件哈希命名，保留可直接定位的偏移和长度。静态图片可以保存原件、派生物或两者；因此文件对象数量不等于页数。压缩包等非图片也可归档，但普通图片浏览只枚举验证为图片的对象。Ugoira 首帧封面是 `poster` 资产，不冒充原动画。

## 媒体表示与已有配方

`representation` 为 `original`、`derived`、`poster`。静态图使用现有 `image_policy.media_policy()` 规范化，配方 ID 复用现有 `profile_id()`；自定义配方保持 `studio-image-v1:<hash>`，不能另造一个同名但含义不同的编码配置。

新采集器 v1 使用 `existing="match_profile"`、`allow_sample=false`。原件长期保留由 `retain_original` 决定；`profile="original"` 要求保留原件，`metadata_only` 不创建媒体取得任务。Ugoira 默认归档网页提供的包与帧时序，生成版本为 `ugoira-poster-v1` 的首帧封面；不将网页包声称为已验证的上传原始帧集合。

`downloaded` 表示实际下载校验，`http_validated` 表示有效 HTTP 条件验证，`historical_reuse` 表示沿用历史已取得文件，`derived` 表示生成的表示。历史复用必须匹配作品、槽位、来源变体、URL、相关尺寸/帧清单及配方，且未超任务年龄上限；它不能更新为“本轮重新校验”。强验证缺失时，`revalidate` 模式重新取得字节。

## 当前版本选择与发布

版本区间为 `[valid_from, valid_until)`。读者在固定水位 S 下读取；每个事实过滤 `commit_seq <= S`，对象过滤 `first_seq <= S`，版本关系过滤 `valid_from <= S < valid_until` 或无上界。不能用“`valid_until IS NULL` 就是当前”替代快照判断。

投影规则固定为：

1. 每个作品选择最新有效完整详情观察，按规范 `observed_at`、观察 ID 排序，不按延迟到达的发布次序覆盖更新观察。
2. 媒体指针只选完整、身份一致的清单，按捕获观察时间和清单 ID 排序。新详情与旧清单页数冲突时保留最后已知清单，并标 `needs_refresh`。
3. 新完整清单替换当前成员关系，旧事实及对象保留。新页无对应资产时返回缺口，不用同槽位旧图冒充。
4. 新旧可见条件不可比较时标 `visibility_changed`。差分只能称“本次条件下未列出”，不产生站点删除事实；权限错误和失败响应本身不能替换有效清单。
5. 每个 `(media_id, representation, recipe_id)` 选择一个已成功取得的资产。优先采用新的实际验证，其次为历史复用；历史复用不能降低已有更强验证的依据。排序元组和证据等级由统一投影函数维护，增量发布与重建共用。

第五项排序具体为：验证等级（`downloaded/http_validated/derived` 为 2，`historical_reuse` 为 1）、`last_verified_at`（NULL 最低）、`acquired_at`、`asset_id`，均取最大。输出相同文件但验证收据不同，可以产生新资产记录并更新选择。

新 `ONLINE.json` 声明在线 3；来源版本为 `online-v3:<generation>:<served_seq>`，元数据版本为 `metadata-v3:<source_id>:<generation>:<served_seq>`。旧前缀继续按旧解析器处理。元数据与关系变化对旧、新关联对象均记录变更；图片字节未变不要求重建图像缓存。

## 查询与元数据读取

工作集图片键继续为 `source_id + SHA`；来源资产记录使用独立的 `record_id`。作品成员接口保留每个页位置，图片查询按 SHA 去重。页尺寸来自媒体条目，实际保存尺寸来自文件对象；作品标签、标题和统计来自作品观察。

`CurrentPost` 在新来源中表示当前作品观察与当前媒体清单的关联；`AnyObservation` 遍历历史媒体清单及其配对的详情观察。历史元数据检查可显示同作品的其他观察，但必须标明 `same_work`，不能在查询中把不同出处的字段拼成一个匹配。

例如标签和源宽度条件必须落在同一作品与媒体关系上：先筛选作品观察的标签，再连接该清单中的页宽度与该页资产，最后返回文件 SHA。条件不能分别在同一文件的不同作品出处上成立后被合并。

Pixiv 初版提供浏览、图片、元数据、原文、精确标签查询、逐页关联和存储尺寸能力。新增 `work_members`、`author_metadata`、`literal_tags` 能力；旧来源缺少这些字段时按 false 读取。暂不声明 Danbooru 专属排名或未经定义的跨站分级投影。Pixiv 原始分级、AI 类型和作品统计作为 `pixiv.*` 字段暴露。

## 兼容边界

归档 1、在线 2 的 schema、资产身份、当前绑定与历史兼容记录保持原规则。新程序通过版本分派选择实现；禁止仅替换一个全局 VERSION 常量。旧湖不自动搬迁、重打包或改写源 ID。

格式 2 必须拒绝不认识的必需功能，旧程序也因格式号不匹配而拒绝误读。新控制 schema 8 为追加迁移，不能修改已执行的旧迁移。重建使用固定归档前缀产生独立目录，启用和换代继续遵守现有位置与租约流程。
