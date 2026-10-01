# 0050：归档直接重建与生产者索引退役

日期：2026-10-01。承接 [0037](0037-versioned-online-lakes.md) 与 [0040](0040-owned-lake-worker-and-image-recipes.md)。

三湖的正式读取与更新使用同一在线 v2 协议。旧的整湖 DuckDB/目录 SQLite 是可重建的派生物，保留它们作为在线库初始化的唯一入口，会使已退出日常流程的完整 raw 副本继续占用 SSD。

## 归档与服务边界

- E 盘的图片包、来源 Parquet、观察、资产、提交日志与来源 schema 继续承担完整归档责任。Parquet 延续 Zstd，不改写既有不可变批次。
- F 盘各湖独立的在线 SQLite 保留版本、租约、索引和逐条压缩 raw；三湖共用 schema、编解码规则与更新服务。
- 项目捕获的成员、排名与按需 DuckDB 分析仍保留。退役整湖文件不等于移除 DuckDB 运行库。
- raw 统一校验原始 UTF-8 字节长度、SHA-256、压缩流结尾及解压预算。暂不切换在线压缩算法，不在没有收益测量的情况下迁移 payload 去重结构。

## 独立重建

`archive_rebuild` 读取固定的连续 journal 前缀，逐批校验封存元数据，再在独立目录构建在线库；不读取生产者数据库或图片 TAR 内容，也不重新编码图片。

各输入阶段的行位置、导入摘要和检查点与输出行在同一 SQLite 事务提交。中断后验证原有归档前缀和目录身份再续建。标签字典缓存有界；辅助查询索引在私有准备库中延后构建，身份约束始终保留。

完整验证包含 SQLite/FTS 检查、归档导入摘要、raw 全量无损回读、关联完整性和最终文件摘要。图片包检查存在性和长度，不宣称重做了全湖图像介质校验。

指定现用在线库作参考时，先登记显式校验租约，固定版本下界；比较稳定身份、完整观察、资产、对象定位、raw、schema 和当前帖子/对象关系。整数行号或首次在线发布序号不作为跨代身份。对照失败保留准备库与租约，`release` 可显式释放；成功对照自动释放。

准备库不能覆盖现有 `ONLINE.json`。普通构建不会启用结果；首次启用只允许没有其他在线位置的库，并可接续相同代次的指针写入。对已使用的湖，换代恢复仍需处理项目及租约引用，不能用重建命令绕过位置/版本协议。

### 早期投影的兼容关联

正式全量对照发现，Danbooru 有 266 条旧投影关联：当前 API 观察缺少 MD5，但现用库仍指向同帖已有的历史资产。观察、资产和 raw 本身完全一致，按现行严格关联规则重新推导会把这些历史关联变为空值。

这类已存在的选择保存在归档的 `source_manifests/legacy-online-bindings-v1.json` 中，包含库身份、捕获水位、参考代次及明确的观察/资产标识。它是历史投影的兼容信息，不是新增来源事实；正常增量发布的匹配规则不变。恢复仅在帖子从捕获水位以后没有被新观察或资产触及时回放，否则使用现行规则。文件摘要属于重建身份和退役验证条件，并随既有 `source_manifests` 搬盘协议保留。

`adopt-legacy-bindings --output ...` 只接受六项事实对照通过、仅当前帖子关联不同的已验证准备库。它逐条证明观察相同、MD5 缺失、现用资产来自同帖归档，并限制清单大小；其它差异拒绝交接。先核实完整验证的数据库摘要，再只修改私有准备库的帖子关联，完整重比全部帖子关系，记录变更前后摘要与原始验证凭据。未改动的 raw 和六项事实沿用已完成的全量验证，不重新声称解码过一遍。中断后的交接必须重新运行完整验证与对照才能恢复为已验证状态。

schema 对照使用已逐条核对的 raw 中实际出现的 schema 标识，仍比较全部对应 schema 字节，并限制标识集合预算；不再为了得到相同的标识集合额外关联扫描整张观察表。

## 旧任务交接

已结束 API 捕获但停在规划阶段的旧日任务，需要在 Studio 以相同帖子集合、保存 profile 和保留已有图片策略完成接续。只有所有帖子都有 stored/reused/unavailable 终态，才记录交接。

交接前备份提交日志，在同一事务保存 `legacy_task_handoffs` 凭据并将旧任务标记为 `handed_off`。不伪造旧验收报告，不推进旧验收水位；历史记录继续可读。旧入口不再恢复或验收该任务。

## 退役事务与文件回收

维护入口先取得位置准入、日任务、归档 writer、生产者索引和在线发布锁，要求独立归档重建与现用在线对照成功且覆盖生产者水位，并复核输入文件身份。未知文件、链接、共享硬链接、未交接任务或代次变化会中止操作。

在删除第一份文件之前持久化 `PRODUCER-RETIRED.json`，旧 `CURRENT.json` 改为版本 0 的退役标记。只逐项回收清单中的生产者数据库及其辅助文件，每项记账；中断后从凭据接续。Studio 在线代次、业务数据和项目引用保持原值。

旧 Store 与内置兼容 Index 看到退役标记后拒绝自动重建。独立离线 Store 工作可使用另一个缓存目录。已退休文件意外重新出现时维护入口报告冲突。

位置迁移把退役标记、旧指针标记和 retirement 凭据目录纳入校验清单；缺失或未完成的退役状态不能随搬盘丢弃。

准备库的 `cleanup` 先归档小型验证凭据，再删除本工具生成且尚未启用的数据库；不递归删除目录，不将测试时的峰值占用算成最终节省。

## 热字段与 raw

结构化查询字段继续使用普通 SQL 列和索引。排名遇到明确有效的资产尺寸时不解压整条 raw；缺失或含糊的尺寸沿用原来的回退语义，并使用与详情相同的完整性校验。

后续若测量支持 raw payload 去重，应按原始字节身份共享压缩内容，同时分别保留每次观察、来源格式与 schema。此阶段仅输出有界哈希抽样统计，不引入新的三湖格式分支。

## 维护入口

仓库根目录使用 `node tooling/lake-storage.mjs`，Python 选择沿用 `tooling/lake-worker-runtime.mjs`。示例中的准备目录必须与主库和现有在线目录相互独立；复用同一准备目录只会接续其固定水位，新快照应使用新目录。

```powershell
$mediaRoot = 'E:\AI\AI_Dataset\Danbooru'
$indexRoot = 'F:\Dataset\Danbooru'
$preparedRoot = 'F:\Dataset\.local\rebuild-example\Danbooru'
node tooling/lake-storage.mjs build --media $mediaRoot --output $preparedRoot --site danbooru --reference-index $indexRoot
node tooling/lake-storage.mjs verify --output $preparedRoot
node tooling/lake-storage.mjs compare --output $preparedRoot
# 仅在完整对照证明是上述历史无 MD5 关联差异时：
# node tooling/lake-storage.mjs adopt-legacy-bindings --output $preparedRoot
node tooling/lake-storage.mjs retire --media $mediaRoot --index $indexRoot --proof $preparedRoot
# 审核上述清单、旧任务交接及验证结果后，显式执行：
node tooling/lake-storage.mjs retire --media $mediaRoot --index $indexRoot --proof $preparedRoot --apply
node tooling/lake-storage.mjs cleanup --output $preparedRoot --evidence '.local/reports/rebuild-example/Danbooru'
```

旧任务交接入口为 `handoff --media ... --index ... --state-root ... --run <旧 run_id> --job <Studio job_id>`，默认只预览；`--apply` 备份日志并持久化交接。不自动创建网络抓取任务，也不改变其他暂停任务。

`activate` 仅用于不存在其他在线位置的首次启用；正式湖的退役操作保持已有在线文件和代次。未完成对照需要取消时，先用 `release --output ...` 释放校验租约；不要直接删除控制文件来遗忘租约。
