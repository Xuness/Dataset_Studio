# 契约三 后端任务与读取接口

版本：1。定稿日期：2026-10-03。范围：无前端也可完成创建新湖、管理网页凭据、运行采集、恢复及读取结果。所有路由和类型均为待实现契约，不是当前已可调用的接口。

## 服务与协议接入

新增 domain/application 的采集控制端口和作品成员读取端口。HTTP DTO 位于 `studio-protocol`，路由位于 engine，SDK 使用公共 transport；不把 SQLite、网页请求或 Tauri 传入核心层。

采集控制与旧 `LakeUpdateBackend` 共用一个运行器控制对象、Python 环境和状态目录。保留现有运行配置入口，不能配置两个竞争同一控制库的独立 daemon。Python 内增加 `collections` 控制与计划模块、`collectors/pixiv` 适配器，验证 CLI 调用同一应用服务。

现有 stdin/stdout 外壳保持 `protocol_version=1` 与 `{command, arguments}`，新增 `collection_*` 命令。握手追加 `features.collections=1`、支持的归档和在线版本；新 engine 必须验证能力，不能只看到协议 1 就发送新命令。旧命令、响应形状和错误语义保持兼容。

## 控制路由

统一前缀 `/v1/source-collections`。继承现有 engine 认证、请求上下文、2 MiB 控制消息上限和 `ApiError {code,message,request_id}`，不另造错误外壳。

| 方法与相对路径                   | 契约                                                                                                    |
| -------------------------------- | ------------------------------------------------------------------------------------------------------- |
| `GET /status`                    | 同一运行器的健康、采集活动、各等待原因计数和共享资源摘要；运行器未就绪时仍由 Rust 返回健康状态          |
| `GET /capabilities`              | collector 版本、工作类型、发现入口、参数限制、支持格式和能力；只声明已实现能力                          |
| `GET /lakes`                     | 采集能力注册的湖、身份、格式和就绪情况；不扫描归档计算计数                                              |
| `POST /lakes`                    | `CreateLake` → `CollectionLake`；幂等初始化空目录并登记，格式固定归档 2、在线 3                         |
| `GET /accounts`                  | `Account[]`，仅非敏感状态                                                                               |
| `PUT /accounts/{id}`             | `SaveAccount` → `Account`；新建时 expected_revision 为 null，更新必须匹配修订                           |
| `POST /accounts/{id}/probe`      | `{request_key,expected_revision}` → `Account` 与非敏感可见性检查结果；这是明确的网络操作                |
| `POST /accounts/{id}/clear`      | `{request_key,expected_revision}` → `Account`；清除秘密但保留被任务引用的账号身份                       |
| `POST /jobs/preview`             | `JobDefinition` → 规范化定义、已知种子数、未知总量及能力问题；不发网络请求、不创建任务                  |
| `POST /jobs`                     | `CreateJob` → `JobResult`；固定规范化定义，以 queued 或 waiting_credentials 开始                        |
| `GET /jobs`                      | 按湖、状态和时间游标读取有界任务页                                                                      |
| `GET /jobs/{id}`                 | `Job`；包括定义、实际状态、期望状态、修订、执行活动和分单位进度                                         |
| `GET /jobs/{id}/tasks`           | 按阶段、状态、原因读取 `Task` 页；不返回无界原文或秘密                                                  |
| `GET /jobs/{id}/coverage`        | 作者目录、作品清单和媒体目标的覆盖说明、可见条件及不确定原因                                            |
| `POST /jobs/{id}/actions`        | `JobAction` → `JobResult`；暂停、继续、失败项重试、取消或发布重放                                       |
| `GET /pipeline`、`PUT /pipeline` | 新 collector 的请求额度、下载并发和采集准入设置，PUT 使用 expected_revision；共用全局设备/解码/暂存上限 |

字段形状以 [contract-schema.json](contract-schema.json) 为准，正反例见 [examples.json](examples.json)。列表响应统一 `{items,next_cursor}`，默认 50、最大 200；原始响应和关系集合必须单独分页。

共用响应固定如下，复用的既有类型以当前 Rust DTO 为准：status 返回 configured、protocol_version、collection_contract_version、既有 LakeUpdateRuntimeHealth 类型的 runtime、按 JobState 计数的 counts 和最多 20 条活动摘要；capabilities 返回 collector、contract_version、工作类型/入口集合及参数 limits。preview 返回 definition、known_seed_count、known_work_count（可为 null）、known_media_count（可为 null）和 `{code,message,severity}` 问题列表。

probe 返回 account 和 visibility；visibility 包含 context_id、observed_at，以及 login、r18、r18g、ai_display 各项的已确认值或 unknown，另外返回 coverage_verified。pipeline 返回 revision、value 和只读的 shared_limits：value 固定包含 pixiv 的既有 LakeSitePipeline 字段、metadata_concurrency、pending_media_limit、publication_backlog_mib 和 time_slice_seconds；shared_limits 复用现有 LakePipelineConfig，不再建立第二份共享设置。PUT 只接受 expected_revision 和 value。

状态响应的采集计数只聚合采集任务。旧 `/v1/lake-updates/*` 继续过滤到支持的旧任务族和湖，避免返回 Pixiv 后使旧站点枚举解码失败。两族任务共享资源与位置准入；公共来源注册表独立声明四种来源。

## 幂等与修订

所有创建和状态变更使用 UUID `request_key`。服务端保存规范化请求指纹和结果身份：相同 key、相同语义返回原操作对应对象的最新状态并标 `replayed=true`；相同 key、不同语义返回 `COLLECTION_IDEMPOTENCY_CONFLICT`。重复请求先查收据，再检查 expected_revision，避免已成功动作因后续状态变化而变成失败。

请求指纹不进入公共响应。普通请求保存规范化 SHA-256；凭据请求对同样的摘要使用既有 DPAPI 保护，存为带类型前缀的受保护指纹，比较时解密校验，不保存明文摘要。指纹不能写日志、归档或公共错误。API 日志不记录凭据请求体。

`expected_revision` 防止旧页面或客户端覆盖新指令。进程自动状态变化也推进 revision。领取执行增加 execution_epoch；实际运行还须持有现有 OS 锁。暂停和取消请求成功不等于执行已退出，客户端查看 execution_active。

新湖初始化先持久化请求意图、生成固定 library_id，再创建目录与标记。重试只接续同一请求、同一身份和路径；非空且无匹配意图的目录拒绝。成功登记前完成格式及身份预检，不自动把已有目录改造成 Pixiv 湖。

## 任务范围

`JobDefinition.version=1`，collector 为 `pixiv_web_v1`。库身份、账号引用、种子、范围、发现规则、媒体策略和单轮预算都固定在定义中；全局资源额度另外管理。

- `seeds.kind="authors"`：1–1000 个作者 ID；每位获准进入本轮的作者保存目录快照，再补全其中符合目标类型的作品。
- `seeds.kind="works"`：1–1000 个明确作品 ID，用于验证和定点补全；v1 要求关闭作者扩展，避免定点输入隐含变成全作者任务。
- `scope.work_types`：插画、漫画、Ugoira；分级原值经站点适配器分类。AI 和未知标记是否纳入显式记录。默认示例纳入全部类型、分级、AI 和未知标记，实际取得范围仍受登录可见性限制。
- 保存策略使用现有图片配方及显式 retain_original，v1 禁止缩略图静默替代原图。metadata_only 仍保存详情和媒体清单，但不声称文件已取得。

作者目录和媒体清单在任务中分别冻结为明确观察。恢复继续同一目标；如果有证据表明远端媒体已变化，原目标保留 source_changed 缺口，后续新快照任务取得新版本。HTTP 404 本身不能证明作品删除或内容变化。

初版不提供无结束条件的 watch 任务。持续更新可由调度器创建后续快照任务，沿用同一核心及已存事实；定时策略不影响已创建任务定义。

## 作者扩展

支持公开收藏、公开关注和作品推荐，`max_depth` 为 0–4。作者种子深度为 0；从其关注账号、收藏作品作者或推荐作品作者引入另一个账号时增加 1，读取该作者自己的作品不增加深度。

深度为 0 时发现入口为空。推荐种子按冻结目录中的作品 ID 数值排序，等距选取指定数量，最多 32 个；数量大于目录时取全部。它是确定性覆盖策略，不将 ID 排序解释为准确创作年代。

同一作者或作品只维护一个任务内实体，发现路径全部保留。如果后来发现更短路径，降低 min_depth；按更浅深度重新传播已归档关系，使用 expanded_depth 避免重复工作，不立即重新发同一网络请求。

超深度属于范围排除；超过本轮准入、网络或时间预算属于等待预算，两者不能混用。关系页的快照先归档，再产生候选，重复 ID 不重复派发；offset 重叠和去重不等于站点提供了事务快照。

## 预算与共享调度

单轮预算固定 API 请求数、作者准入数、下载字节和运行时间。它们是停止新准入的轮次预算，允许在途工作到达安全边界；文件、像素、内存、磁盘余量等硬限制沿用共享资源机制。继续任务开启新的同额轮次，不修改原范围。

新来源起始额度取元数据并发 1、每秒 1 次请求、媒体并发 2；它们是可调整的初始配置，不是站点吞吐保证。共享冷却至少按站点、账号和主机类别协调，响应 Retry-After 优先。普通暂时错误有限退避，最多 8 次自动尝试，之后 needs_review；登录失效不当作普通重试。

旧更新与新采集共享 active_lakes、设备、编码、暂存、解码内存及同湖发布额度。调度每湖只授予一个任务执行所有权，按安全时间片轮转可执行任务；初始时间片 30 秒，停止新准入后等待在途操作到可靠边界。时间片不是强杀线程的期限。

Pixiv 来源额度保存在新的采集设置中，旧 pipeline_v1 的三站响应形状不变。共享总额度只有一份，不能给新采集器再分配一份等额的独立预算。

## 状态与操作

| 状态组                                             | 进入与退出规则                                                                             |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| queued / running                                   | 有可执行任务时取得锁和新 epoch；子阶段独立运行                                             |
| pausing / paused                                   | desired_state=pause 后停止新准入，实际退出才成为 paused                                    |
| waiting_credentials                                | 会话缺失、失效或需要交互；有效同账号恢复后才继续                                           |
| waiting_retry / waiting_resources / waiting_budget | 保存明确原因与检查点；分别遵循重试时间、资源可用性或新的执行轮次                           |
| publishing                                         | 取得任务已收敛，但仍有必须发布的批次；不能提前称完成                                       |
| needs_review                                       | 重试耗尽、完整性或契约冲突；无自动高速重试                                                 |
| cancelling / cancelled                             | 停止新准入并结束执行，回收独占暂存；已归档事实保留                                         |
| completed / completed_with_gaps                    | 所有范围内任务收敛，控制确认与所需发布完成；有不可获取项、未验证可见性或范围缺口则使用后者 |

resume 只在旧执行退出后接受；retry_failed 只重排选中的失败/未获取项或默认全部可重试缺口，不重新下载成功项。重试仍使用固定媒体目标；需要新快照时创建新任务。replay_publication 只补发布和确认，不发 Pixiv 请求。

cancelled 任务不通过 resume 重新激活。需要继续其范围时创建新的任务并按保存策略复用已有结果。暂停、取消和完成状态不会被迟到的后台回调改回 running。

`progress` 的作者、作品、媒体和文件对象分别计数。媒体 archived/published 表示该条目所要求的全部表示已达到对应阶段；原件成功但必需封面失败仍是媒体缺口。相同文件被两页引用可以是两个媒体成功、一个对象新增。

只有发现结束、目录和清单目标都确定后，相应计划总量才非 null。无总量时不返回整体百分比。目录遍历结束与可见范围完整性分别表达；无法确认可见条件时，不输出无条件的作者完整覆盖。

## 凭据与会话

账号 ID 是本地身份，viewer_key 是可入归档的非敏感引用。凭据存入现有 Windows DPAPI 保护机制，公共接口只返回保存状态、修订、已确认用户 ID 和检查时间。

Cookie 导入使用有界结构化列表，校验名称、值、域、路径、过期和 Secure 属性；总请求不超过 64 KiB。只接受 Pixiv 域范围，不把会话材料发送到 pximg 或外链。HTTP 客户端按账号会话复用连接，并集中处理 Cookie 更新；旧修订的响应不得覆盖用户新导入的会话。

probe 必须验证服务端认可的用户身份，不能仅解析 Cookie 字符串或请求一个公开作品。账号首次成功校验后绑定来源用户 ID；不同用户的 Cookie 不可静默复用同一本地账号。更换身份需新账号引用和新任务。

显示条件逐项标为已确认、未确认或受限。会话续期但身份与已知可见条件一致可继续；已知条件变化时暂停并报告 scope_changed。已有事实仍可读取，失败或权限变化不能产生删除结论。认证页面和挑战材料不进入原始作品档案。

## 来源读取与项目接入

公共来源注册表新增 `kind="pixiv"`，按 library.json 和 ONLINE.json 识别身份与版本。复用 `/v1/source-probes` 和项目 source attach；目录名字不作为站点或库身份。

已有图片、元数据、观察及 raw 路由保持路径不变。v3 资产来源记录追加可选的 work_id、media_id、manifest_id、ordinal、kind 和 representation；旧来源缺省为空。媒体原始观察指向该页/动画清单，作品历史观察明确标记 same_work。

新增项目来源下的读取入口：

| 相对 `/v1/projects/{project_id}/sources/{source_id}` | 返回                                                                |
| ---------------------------------------------------- | ------------------------------------------------------------------- |
| `GET /works/{work_id}`                               | 当前已知作品观察、最后完整媒体清单及刷新状态                        |
| `GET /works/{work_id}/media`                         | `WorkMediaPage`，固定版本、逐页位置及表示绑定；支持有界历史清单读取 |
| `GET /authors/{author_id}`                           | 作者观察与原文引用                                                  |
| `GET /authors/{author_id}/works`                     | 最近目录观察中的有界作品成员及获取状态；不将收藏作品混入作者投稿    |

所有读取经过 SourceService/SourceRead、取消、预算和版本租约。记录 ID 必须属于所请求的库、文件和作品；不能把传入 ID 直接当 SQLite 或文件路径。

作品成员每页的 bindings 缺省返回原件及该清单采集配方对应的派生物/封面，也可指定一个配方读取，最多 8 条。其他历史表示通过已有来源记录分页接口读取，不将无界历史数组塞进一页。游标绑定清单、配方筛选和版本。

普通图片浏览与预览只公开可解码图像对象，Ugoira 默认浏览首帧封面。压缩包保留在归档及作品媒体信息中；此版本不把 ZIP 送进旧图像预览接口。原始包可由后台归档读取/导出路径取得，完整动画播放稍后设计。

## 分页与错误

来源读取首批固定版本，后续 cursor 绑定库身份、generation、水位、筛选条件、排序和最后键；过期返回 SOURCE_CHANGED，不能静默跳到最新版本。默认 50、最大 200，响应还受 2 MiB 控制大小约束。单个详情超限明确报错，不返回伪完整的截断内容。

任务与活动列表是实时读模型，采用 keyset 游标并绑定筛选条件，不承诺跨多次请求冻结所有状态变化。新增任务可设上界避免翻页被不断插入的头部拖动；状态变化后客户端刷新列表。

| 错误                                                   | HTTP 与处理                                |
| ------------------------------------------------------ | ------------------------------------------ |
| INVALID_INPUT / COLLECTION_POLICY_UNSUPPORTED          | 400，参数或能力不符合契约                  |
| NOT_FOUND                                              | 404，本地对象不存在；不代表 Pixiv 作品删除 |
| REVISION_CONFLICT / COLLECTION_IDEMPOTENCY_CONFLICT    | 409，重新读取或使用正确请求身份            |
| COLLECTION_EXECUTION_ACTIVE / COLLECTION_SCOPE_CHANGED | 409，旧执行尚未退出或范围条件已变化        |
| COLLECTION_PROTOCOL / COLLECTION_FORMAT_UNSUPPORTED    | 409，运行器或格式能力不匹配，拒绝部分执行  |
| COLLECTION_UNAVAILABLE / COLLECTION_REMOTE_UNAVAILABLE | 503，运行器或探测暂不可用；保留原状态      |
| METADATA_LIMIT / COLLECTION_LIMIT                      | 413，有界请求或响应超过限制                |

任务内的远端失败写入任务原因，不把远端 HTTP 状态直接变成本地控制 API 的权限状态。错误信息有界且脱敏，保留 request_id 关联本地诊断。

## 验证入口

独立 CLI 提供与 API 等价的初始化、凭据导入、预览、创建、状态、动作、作品成员读取和归档重放命令。凭据从交互输入或指定文件读取，不进入 argv；测试运行以隔离状态根和湖目录为必填条件。

第一轮功能验收通过后端创建小型新湖、抓取指定作者、发布、注册为项目来源，并经现有图片/元数据接口及新作品成员接口回读；不依赖新增页面。
