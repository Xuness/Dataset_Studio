# 来源采集后端接入

日期：2026-10-03。Pixiv 已接入数据湖读写、持久采集、增量复查、周期计划和 Studio 的现有浏览、任务及设置界面。归档兼容性见 [0052](../decisions/0052-pixiv-collections-and-canonical-media.md)，公开完成状态与持续运行见 [0053](../decisions/0053-pixiv-public-operation-and-incremental-refresh.md)。

## 界面操作

1. 在数据湖工作台选择“登记数据湖 → Pixiv”，分别填写图片归档与在线索引目录；新湖勾选“创建新的 Pixiv 空湖”。已有湖需要原控制记录，不能从其他控制器直接接管正在管理的归档。
2. 在“新建更新”选择 Pixiv，填写作者或作品 ID／链接。默认公开访问，凭据可留空；可设置作品类型、分级、未知标记、关系扩展、保存配方和预算。
3. “增量复查”控制详情快照有效期和历史文件复用。周期在“本轮预算与周期”启用，随后在同一工作台的“定时计划”管理。
4. 完成后按作者、作品、原始标签等筛选。图片属性显示作者、作品和页序，提供“筛选同作者／同作品”。Ugoira 保存帧包与时序，以派生首帧进入图片浏览。
5. 登录会话在“设置 → 数据湖 API → Pixiv”导入 PHPSESSID 或 Cookie JSON，验证后才能用于登录采集。账号身份与显示条件分别记录。公开任务需要换登录范围时，新建复查并选择该会话。

数据湖位置可在同一设置页重新关联。迁移时按页面流程暂停写入并搬好目录，程序同步项目和采集器的位置；无需重建图片身份。

## 入口与所有权

使用[内置 lake-worker 环境](../../services/lake-worker/README.md)。Studio 配置现有更新服务后，同一个受控运行器处理新采集任务；关闭项目或离开页面不会取消任务。任务、湖和账号由共享控制状态目录管理，项目只保存来源引用。

独立命令调用相同 Python 服务：

```powershell
node tooling/source-collections.mjs lake_create --root '<control>' --input '<request.json>'
node tooling/source-collections.mjs account_save --root '<control>' --input '<account.json>'
node tooling/source-collections.mjs preview --root '<control>' --input '<preview.json>'
node tooling/source-collections.mjs create --root '<control>' --input '<job.json>'
node tooling/source-collections.mjs run --root '<control>' --input '<run.json>'
```

`--root`、媒体湖和在线库应为明确的绝对路径。媒体和在线目录必须独立；创建仅接受空目录，重复请求使用同一个 UUID `request_key`。`run.json` 形如 `{"id":"<job UUID>","time_slice":30}`，每次运行一个有界时间片；存在后台运行器时独立 `run` 拒绝竞争。其余命令读写控制状态，可与受控运行器协作。JSON 从文件或 stdin 输入，Cookie 不接受命令行明文。不要把含 Cookie 的输入文件加入仓库。

创建湖请求：

```json
{
  "request_key": "<fresh UUID>",
  "site": "pixiv",
  "media_root": "<absolute empty archive directory>",
  "index_root": "<absolute empty online directory>"
}
```

创建公开账号请求：

```json
{
  "request_key": "<fresh UUID>",
  "expected_revision": null,
  "account_id": "<fresh UUID>",
  "label": "Pixiv 公开验证",
  "mode": "anonymous"
}
```

会话账号使用 `mode=session` 并传入结构化 `cookies` 数组，每项含 `name/value/domain/path/secure/http_only/expires_unix`；只接受 Pixiv 域和 HTTPS Cookie。导入后调用 `account_probe`，参数为 `id/request_key/expected_revision`。账号身份、认证状态、目标内容可见性分别报告。浏览器登录助手尚未接入；无需保持浏览器窗口常驻。

## 有限快照任务

预览输入为 `{"definition": ...}`；创建输入增加 `request_key`。以下定义只取明确的作者，不向外扩展。替换示例身份后才能使用：

```json
{
  "version": 1,
  "collector": "pixiv_web_v1",
  "library_id": "<library UUID>",
  "account_id": "<account UUID>",
  "seeds": { "kind": "authors", "ids": ["10109777"] },
  "scope": {
    "work_types": ["illustration", "manga", "ugoira"],
    "ratings": ["all_ages"],
    "include_ai": true,
    "include_unknown_markers": false
  },
  "discovery": {
    "entrypoints": [],
    "max_depth": 0,
    "recommendation_seeds_per_author": 0
  },
  "media": {
    "image_policy": {
      "profile": "original",
      "existing": "match_profile",
      "allow_sample": false
    },
    "retain_original": true,
    "ugoira": "archive_with_poster",
    "reuse": { "mode": "revalidate", "max_age_hours": 0 }
  },
  "run_budget": {
    "api_requests": 500,
    "admitted_authors": 1,
    "download_bytes": 1073741824,
    "wall_seconds": 3600
  }
}
```

`seeds.kind=works` 可执行作品小样本；该模式不向外扩展。作者模式可显式启用 `following/bookmarks/recommendations`，深度 0–4，推荐种子从已知目录按确定性排名位置取样。预算是单轮准入上限，下载字节可能因已开始的并发文件而超过边界一个有界批次；不会重写任务范围。未知总量使用 `null`。

`metadata_only` 不下载媒体；常规图像复用现有配方。Ugoira 使用 `metadata_only` 或 `archive_with_poster`。`historical_if_same_locator` 加 `max_age_hours` 允许复用相同作品位置、URL、尺寸、帧表及配方的历史文件，保留此前核验时间，不把它计为新下载或 HTTP 核验。

## 公开 API 和 SDK

| 接口                                                                            | 用途                                                  |
| ------------------------------------------------------------------------------- | ----------------------------------------------------- |
| `GET /v1/source-collections/status`、`capabilities`                             | 受控运行器状态、能力与限额                            |
| `GET/POST .../lakes`                                                            | 分页列湖、创建独立湖                                  |
| `POST .../lakes/register`                                                       | 核验并登记由本控制器管理的已有 Pixiv 湖               |
| `GET .../accounts`、`PUT .../accounts/{id}`                                     | 分页列账号、导入或更新凭据                            |
| `POST .../accounts/{id}/probe`、`clear`                                         | 会话探测、清除凭据                                    |
| `POST .../jobs/preview`、`POST .../jobs`                                        | 只读规范化预览、幂等创建                              |
| `GET .../jobs`、`jobs/{id}`、`tasks`、`coverage`                                | 队列、状态、细项和覆盖依据                            |
| `POST .../jobs/{id}/actions`                                                    | `pause/resume/retry_failed/cancel/replay_publication` |
| `GET/PUT .../pipeline`                                                          | Pixiv 准入、频率及积压上限，共享资源使用现有 pipeline |
| `GET .../schedules`、`PUT .../schedules/{id}`、`POST .../schedules/{id}/remove` | 分页读取、修订保存、移除周期计划                      |
| `GET .../workspace/lakes`、`workspace/jobs`、`workspace/schedules`              | 现有工作台的联合有界列表                              |

SDK 入口为 `client.sourceCollections`。分页使用 `cursor/limit`，常规上限 200，联合任务列表上限 100 并受响应字节预算限制；游标绑定列表和筛选条件。命令使用 `expected_revision`，修改请求内容不能沿用相同 `request_key`。任务计数分别报告范围排除、快照沿用、历史文件复用、新下载、归档、发布和实际缺口。`completed` 与登录可见性是否验证独立。

可选快照策略例：`"refresh": {"mode":"missing_or_stale","max_age_hours":168}`。目录仍重新请求；同访问条件的完整近期快照可沿用原观察时间。`media.reuse` 控制已重新获得清单后是否复用相同地址的历史文件，两者互不冒充新的验证证据。

来源附加继续使用 `client.sourceAccess.attach(projectId, {kind:"auto", name, media_root, index_root})`。逐作品读取入口：

```typescript
await client.sourceAccess.work(projectId, sourceId, "149628477");
const first = await client.sourceAccess.workMedia(
  projectId,
  sourceId,
  "149628477",
  { limit: 50 },
);
await client.sourceAccess.workMedia(projectId, sourceId, "149628477", {
  version: first.version,
  cursor: first.next_cursor,
  limit: 50,
});
await client.sourceAccess.author(projectId, sourceId, "10109777");
await client.sourceAccess.authorWorks(projectId, sourceId, "10109777", {
  limit: 50,
});
```

对应路由为项目来源下的 `works/{work_id}`、`works/{work_id}/media`、`authors/{author_id}`、`authors/{author_id}/works`。分页续读携带固定版本。现有单图预览、元数据、来源资产、历史观察、raw 和查询接口保持可用。原始 caption 是来源 HTML，消费者不得直接当可信 HTML 执行。

## 维护与验证

独立重建不依赖控制库或原在线缓存：

```powershell
node tooling/lake-storage.mjs build --site pixiv --media '<archive>' --output '<empty preparation>'
node tooling/lake-storage.mjs verify --output '<preparation>'
```

构建时可传 `--reference-index '<online>'` 固定参考水位，再执行 `compare`。`release` 释放比较租约；`cleanup --evidence '<separate evidence>'` 回收未启用的准备库。`activate` 仅用于没有冲突指针的明确恢复位置，不能替换现有项目正在引用的代次。搬盘使用已有位置迁移协议。

离线回归：`pnpm check`、`pnpm test:integration`；单独 HTTP/SDK 回归为 `node tooling/integration-collections.mjs`。真实公开小样本入口为 `node tooling/validate-pixiv-public.mjs --author 10109777 --output '<isolated output>'`，会访问 Pixiv、保存作者目录，并有限下载两个作品。此入口应显式运行，不包含在自动测试中。它不验证登录、R-18 可见性、关系大规模扩展或持续吞吐。

界面回归为 `node tooling/smoke-pixiv-ui.mjs`：合成归档、真实界面与引擎，验证浏览、筛选、新湖／任务／计划、凭据保护和搬盘入口，不访问 Pixiv。旧三湖界面回归继续使用 `node tooling/smoke-lake-updates-ui.mjs`。

对已运行引擎进行显式公开作者验收：

```powershell
node tooling/validate-pixiv-production.mjs --connection '<engine.json>' --media-root '<new archive>' --index-root '<new online>' --project '<existing project id>' --author 10109777 --include-unknown --output '<report directory>'
```

该命令会创建独立 Pixiv 湖，附加到指定项目，抓取当次作者目录和所选范围的原图，再执行一次增量复查。任务键和结果保存在报告目录，同参数重跑接续原请求；不得并发写同一个报告目录。`--include-unknown` 纳入未知标记并保存相应固定定义。它会产生真实网络请求及归档数据，不在自动测试中，也不自动启用后续周期计划。
