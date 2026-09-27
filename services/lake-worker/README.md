# Studio 数据湖更新服务

本模块由 Dataset Studio 仓库维护，负责 Danbooru、Yandere、Gelbooru 的 API 抓取、原文保留、图片处理、持久任务、调度、归档及在线发布。Rust 引擎负责应用生命周期和 HTTP 接口，React 工作台只使用公开 SDK。

运行源码由引擎的 `build.rs` 编入二进制，启动时按源码摘要展开到应用数据目录的 `lake-worker/<revision>`。每次更新使用独立源码版本，不改写正在运行的版本。不需要外部 Danbooru-Store 仓库、editable install、工作目录或 PYTHONPATH。Python 解释器及第三方依赖仍是运行环境，不嵌入 Rust 可执行文件。

## 环境和启动

在仓库根目录运行：

```powershell
pwsh -File tooling/setup-lake-worker.ps1 -Dev
```

默认创建 `.local/runtime/lake-worker`，安装本目录 `pyproject.toml` 的依赖。可用 `-Python <python.exe>` 选择 Python 3.11–3.13。然后在“设置 → 数据湖 API”填写该环境的 Python 和共享更新状态目录。已有配置继续使用原 Python 与状态目录，旧 `store_root` 字段不再参与运行。

开发诊断可直接运行模块入口；常规使用由 Studio 引擎托管，无需手动另开运行器：

```powershell
& '.local/runtime/lake-worker/Scripts/python.exe' -I 'services/lake-worker/worker.py' --root '<更新状态目录>' --mode serve
```

`worker.py` 固定 UTF-8 并从自身路径导入 `studio_lake`。`--mode rpc` 继续使用协议版本 1 的标准输入/输出 JSON，凭据不进入命令行。不要同时启动多个更新控制器。

## 代码和数据边界

- `src/studio_lake/updates`：三站适配、控制数据库、下载、速率和设备调度、固定输入、归档提交与重放。
- `image_policy.py`：自定义配方校验、标识和实际编码；`ingest.py` 保留原先两种策略的字节处理与旧归档恢复语义。
- `library.py`、`metadata.py`、`online*.py`、`index.py`：现有规范湖和在线投影契约。`daily_state.py` 等仅为读取、接续既有归档证据保留，Studio 不接管旧每日自动化入口。
- `upstream-origin.json`：迁入前的 Store 提交与文件摘要，用于追踪初始来源；以后在 Studio 内维护。

索引、状态与准备数据继续使用配置的 SSD 目录，图片与原始元数据归档使用原 HDD 目录。库身份、在线格式 2、项目格式 13 都不改变，无需重新转换或复制三湖。

控制数据库升级为 schema 4，防止旧 Store 更新运行器领取不认识的自定义配方。升级前检查旧运行器和在途任务锁；有活动执行时明确拒绝升级。旧任务、幂等键、游标、凭据、计划和固定输入保留，原图/WebP 固定策略继续可恢复。旧独立 `daily` 流程仍有自己的状态及锁，不读取 schema 4 控制库。

## 保存策略

`media.profile` 保留 `metadata_only`、`original`、`webp-2048-q95`，新增 `custom`。没有新增全局默认。自定义示例：

```json
{
  "profile": "custom",
  "existing": "match_profile",
  "allow_sample": false,
  "encoding": {
    "version": 1,
    "format": "webp",
    "max_edge": 3072,
    "quality": 90,
    "lossless": false,
    "method": 6,
    "animation": "preserve",
    "alpha": "preserve"
  }
}
```

共同参数：`max_edge=null` 保持原尺寸，否则为 1–32768 像素，等比例缩小且不放大。`animation=preserve` 对多帧图保留原文件，跳过缩放/转码；`first_frame` 明确仅取首帧。`alpha` 为保留、背景合成或拒绝透明图片；合成使用 `background=#RRGGBB`。JPEG 不能选择保留透明通道。

| 编码 | 专属参数 |
| --- | --- |
| WebP | `lossless`；有损 `quality=1–100`；`method=0–6` |
| JPEG | `quality=1–100`；`optimize`；`subsampling=444/420` |
| PNG | 无损编码；`compress_level=0–9`，没有有损质量滑条 |

不兼容的参数会被拒绝。无损编码不撤销用户选择的缩放、首帧提取或背景合成。自定义编码保留/转换颜色配置，归档中记录源图片信息、编码配方、Pillow/编码器版本及实际尺寸；API 原文保留不受编码策略影响。

规范化编码配方的 SHA-256 构成 `storage_profile`。改变尺寸、质量、动画或透明处理会改变配方身份；取得图片时的备用图许可与已有图片策略不改变配方身份。`keep` 仍可复用同一媒体身份的已有图片；`match_profile` 要求匹配完整配方，再遵守原图/备用图来源约束。旧资产不删除，物理字节继续按 SHA-256 去重。

界面的命名预设使用应用偏好及修订保护，跨项目共享。应用预设时复制参数，创建任务/计划时固定参数；修改或删除预设不会修改已创建任务/计划。

## 验证

`pnpm test:lake`、`node tooling/integration-lake-updates.mjs`、`pnpm test:lake-ui`。优先使用 `.local/runtime/lake-worker`，也可指定 `STUDIO_LAKE_TEST_PYTHON`，不再使用 `STUDIO_LAKE_TEST_STORE`。夹具和证据只写入 `.local/`；测试不访问真实站点，不创建正式计划。
