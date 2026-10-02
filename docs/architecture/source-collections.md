# 来源采集后端接入

日期：2026-10-03。已实现 Pixiv 的数据湖读写、采集和任务控制、Studio 后端与 SDK。没有新增 Pixiv 前端页面。架构与兼容性见 [0052](../decisions/0052-pixiv-collections-and-canonical-media.md)。

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

| 接口                                                | 用途                                                  |
| --------------------------------------------------- | ----------------------------------------------------- |
| `GET /v1/source-collections/status`、`capabilities` | 受控运行器状态、能力与限额                            |
| `GET/POST .../lakes`                                | 分页列湖、创建独立湖                                  |
| `GET .../accounts`、`PUT .../accounts/{id}`         | 分页列账号、导入或更新凭据                            |
| `POST .../accounts/{id}/probe`、`clear`             | 会话探测、清除凭据                                    |
| `POST .../jobs/preview`、`POST .../jobs`            | 只读规范化预览、幂等创建                              |
| `GET .../jobs`、`jobs/{id}`、`tasks`、`coverage`    | 队列、状态、细项和覆盖依据                            |
| `POST .../jobs/{id}/actions`                        | `pause/resume/retry_failed/cancel/replay_publication` |
| `GET/PUT .../pipeline`                              | Pixiv 准入、频率及积压上限，共享资源使用现有 pipeline |

SDK 入口为 `client.sourceCollections`。分页使用 `cursor/limit`，上限 200；游标绑定列表和筛选条件。命令使用 `expected_revision`，修改请求内容不能沿用相同 `request_key`。任务媒体计数明确区分已下载、历史复用、已归档、已发布和缺口。

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
