# Booru 视觉特征后端

首轮接入日期：2026-10-11。支持 Danbooru、Yandere、Gelbooru 的在线数据湖；通过引擎 API、SDK 和命令行使用。前端页面与项目工作集引用尚未接入。

## 运行与模型

视觉服务有独立 Python 环境。Rust 引擎内嵌对应版本的服务源码并托管常驻进程；模型、依赖、特征和日志不进入 Git。安装入口：

```powershell
pnpm setup:visual --dev
```

需要固定解释器时追加 `--python=解释器完整路径`。依赖索引可用 `--index-url=...` 指定；CUDA PyTorch 仍从官方 cu128 索引安装。CPU 环境使用 `--cpu`，同时将运行配置的 `device` 设为 `cpu`。当前固定 PyTorch 2.11.0、Transformers 5.19.0、LanceDB 0.40.0、PyLance 9.0.0 等版本，具体约束见 [pyproject.toml](../../services/visual-worker/pyproject.toml) 与 [constraints.txt](../../services/visual-worker/constraints.txt)。

本机模型目录为 `Models/Embedding Model/`。传入 `Models` 时自动识别这一级目录：

| 模型                  | 本地目录                        | 固定输入与输出                                           |
| --------------------- | ------------------------------- | -------------------------------------------------------- |
| SigLIP 2 NaFlex       | `siglip2-so400m-patch16-naflex` | 原生处理器、784 patch、1152 维                           |
| EmbeddingGemma 2      | `embeddinggemma-2`              | 纯图片、280 soft token、masked mean pooling、完整 768 维 |
| Kaloscope 3.0-preview | `Kaloscope3.0-preview`          | 短边 512、中心裁剪、CLS＋mean 主干经风格投影得到 256 维  |

Kaloscope 从 `v1-artist-classifier/model.safetensors` 读取冻结的主干和原有投影头，严格匹配权重键；分类头不参与特征计算。该发布文件的 provenance 指向原始第 46,000 步编码器。架构依赖取自模型卡指定的公开 [comfyui-kaloscope 提交](https://github.com/spawner1145/comfyui-kaloscope/tree/8315d80ad58347ea119ffe7e415af2e6a42a6463/kaloscope_dinov3)，安装时保留其许可文件。无需执行原始 `best.pt` 中的 pickle 内容。

CUDA 默认使用 BF16；Kaloscope 保留 FP32 参数和原生位置／归一化缓冲，在前向使用 BF16 autocast。CPU 使用 FP32。EmbeddingGemma 2 禁止使用 FP16。三个适配器均输出有限、L2 归一化的完整 FP32 向量，存储与索引不做维度截断或量化。

配方 ID 由模型文件摘要、预处理、维度、前向精度、运行设备类别和相关依赖版本生成。配置中的路径不参与配方身份；文件变化生成新配方。旧向量仍可检索，旧配方补算需要匹配原运行环境。

## 配置与命令行

需要先启动新版引擎。无界面启动可使用已构建的 `target/debug/studio-engine.exe serve --data-dir .local/dev`。命令行默认连接 `.local/dev/engine.json`，可用 `--data-dir=...` 指定其他应用目录。

```powershell
pnpm visual configure --materials-root="F:\Dataset\Visual Features"
pnpm visual sources
pnpm visual models
pnpm visual create --sources="来源ID1,来源ID2,来源ID3" --models=all --limit=128 --batch=32
pnpm visual run 任务ID
pnpm visual action 任务ID pause
pnpm visual action 任务ID resume
pnpm visual action 任务ID cancel
pnpm visual coverage
pnpm visual failures 任务ID
```

`--models` 也可指定 `siglip2_naflex,embeddinggemma2,kaloscope3` 的子集；`none` 只计算 pHash。`--limit` 是每个湖的对象上限，按固定版本目录的对象行顺序取前 N 项，不能将其视为随机或代表性样本。省略该参数会处理所选湖固定版本的全部对象；后端安装和配置不会自动启动全湖任务。

已有 SHA 和所选配方全部完成、来源检索投影也已同步的条目，会在目录规划阶段剔除，计入 `reused_bindings`。新关联保留；缺失的配方与失败项可在下一次任务中补算。

### 固定图片范围与模型批量

任务支持三种输入：默认按来源目录枚举；`assets` 指定最多 4096 个 `AssetKey`；`scope_run_id` 引用一次已完成发现的任务范围。后两种不能与 `max_objects_per_source` 混用，引用任务时来源集合保持一致。范围包含已成功复用的条目，后续任务直接引用原成员清单，按页规划缺失特征。首版旧任务没有保存完整成员范围，不能用作范围引用；重新创建一次相同目录任务即可形成范围，已有特征继续复用。

读取批量与推理批量独立。`batch_size` 控制每次读取的目标项数，`micro_batch_sizes` 按模型名称配置 1–64 张推理小批量，未指定的模型仍使用 4。实际推理还受解码像素预算与 OOM 拆分约束。参数通过 `pnpm visual create --input=任务.json` 提交，例如：

```json
{
  "source_ids": ["来源ID"],
  "recipe_ids": ["配方ID"],
  "max_objects_per_source": null,
  "batch_size": 128,
  "scope_run_id": "已有任务ID",
  "micro_batch_sizes": {
    "siglip2_naflex": 8,
    "embeddinggemma2": 8,
    "kaloscope3": 8
  }
}
```

`GET /runs/{id}/metrics` 汇总已确认批次的读盘、SHA 校验、解码、pHash、模型加载／预处理／前向、Lance 写入与整理耗时，以及图片数、批次数和内存峰值。重复确认回执不重复累加。RSS 是工作进程生命周期的工作集高水位；CUDA 指标是每批重置的 PyTorch 分配／保留峰值，聚合时取最大值。它们不代表整台主机或其他程序的占用。旧回执没有这些细分指标。

### 特征快照与分批导出

`POST /snapshots` 用名称和 `recipe_ids` 固定各配方的已发布 Lance 版本，并创建保留 tag。`search` 的 `snapshot_id` 可选择旧快照；省略时读取当前发布版本。`POST /features` 返回指定配方和可选 `run_id` 范围内的成功条目，最多 100 项；通过 `after` 和 `next_cursor` 继续读取。游标表示已检查的对象身份，旧快照或部分失败可能产生空页，只要游标非空就需要继续。

`POST /exports` 使用相同范围与游标，要求指定 `snapshot_id`，单次最多检查 4096 个对象。每页写成独立 Arrow IPC 文件，保留完整 FP32 向量、对象身份、来源关联和配方／版本元数据，返回文件路径、行数、字节数和 SHA-256。导出文件位于材料目录的 `exports/`；向量不经 HTTP JSON 传输。快照、条目与导出分别可通过 `pnpm visual snapshot|features|export --input=请求.json` 调用。

## 读取、存储与恢复

- 来源适配器保留每个湖的发布版本，以对象行键分页枚举，在共享 SQLite 中建立按包和偏移排序的清单。不会修改湖的对象／媒体数据，也不反复对湖执行物理排序或大 OFFSET 查询。版本保留沿用现有的来源 lease。
- 每个读取批次最多 512 项、合计 64 MiB 压缩字节；默认目标 256 项。取得引擎的后台读取额度后，Python 只读打开批准范围并核对长度、SHA。字节缓冲准备好即释放媒体额度，模型前向不会占着归档读盘通道。
- 静态图片按 EXIF 方向校正、透明度合成白底、RGB 转换处理。单图上限 32,000,000 像素；累计解码图片按 64,000,000 像素分组，各模型再以小批次推理。三个头共享解码图。动画、视频、超限和解码失败明确记录，首版不自动取首帧。
- SQLite 保存全局 SHA 身份、来源关联、读取清单、任务、稀疏覆盖、失败和发布版本。覆盖计数在批事务中维护；查询覆盖无需遍历全量向量。`discovered` 只代表视觉任务已枚举的对象，完整湖规模单独保存在任务的来源快照中。
- 每个模型配方有独立 Lance 表，保存 `object_id`、`batch_id`、`source_ids` 和完整 FP32 向量。模型小批输出汇总后提交，避免每张图片形成一个数据文件。约每 32 次表修改执行独立的碎片合并和索引增量维护；不调用包含旧版本清理的组合 `optimize()`。
- 每批先保存意图，随后提交 Lance 数据与持久回执，最后发布 SQLite 覆盖和版本。中断后按稳定批次 ID 恢复，已落盘向量按对象 ID 复用；重复确认不会增加行数或计数。新写入先处理遗留未确认批次。
- 材料目录由一个引擎持有写锁。提取和索引维护串行写入；CPU 检索独立执行，只读取 SQLite 已发布的 Lance 版本。暂停和取消在批次边界收敛。引擎退出会终止视觉子进程；重启后未完成任务处于暂停状态，可显式恢复。
- 当前发布过的版本以 tag 保留，不自动回收旧版本或源文件。后续项目引用与材料回收需单独接入；保留空间应包含历史碎片、索引和构建临时空间。

原始 FP32 向量本身约为每张图 `4 × (1152 + 768 + 256)` 字节。IVF_FLAT 另存完整向量；旧版本和整理过程还会增加占用。不能只按一份原始向量估算磁盘需求。

## 查询接口

| 接口                                                   | 用途                                       |
| ------------------------------------------------------ | ------------------------------------------ |
| `GET/PUT /v1/visual/runtime`                           | 环境配置与进程状态                         |
| `GET /v1/visual/models`、`/recipes`、`/sources`        | 当前模型描述、已登记配方、三个 Booru 来源  |
| `POST/GET /v1/visual/runs`                             | 创建任务／有界分页列出任务                 |
| `GET /v1/visual/runs/{id}`、`/failures`                | 进度／按对象分页读取本次任务失败           |
| `GET /v1/visual/runs/{id}/metrics`                     | 已确认批次的性能和内存统计                 |
| `POST/GET /v1/visual/snapshots`、`GET /snapshots/{id}` | 固定、分页列出和读取特征快照               |
| `POST /v1/visual/features`、`/exports`                 | 成功条目分页与固定快照 Arrow 分批导出      |
| `POST /v1/visual/runs/{id}/actions`                    | `pause`、`resume`、`cancel`                |
| `GET /v1/visual/coverage`                              | 各来源与配方的已发现、成功、失败、待算计数 |
| `POST /v1/visual/search`                               | 已入库图片引用或给定向量的相似检索         |
| `POST /v1/visual/duplicates`                           | 跨来源 SHA 分组或 pHash 候选               |
| `POST /v1/visual/indexes`                              | 手动构建／重建 IVF_FLAT；至少 256 条向量   |

SDK 入口为 `StudioClient.visual`。请求和响应来自生成的 OpenAPI 契约；[VisualClient](../../packages/client/src/visual.ts) 与前端页面无耦合。

检索返回最多 100 项、来源 `AssetKey`、配方 ID、实际方法、存储版本和 `cosine_distance`。未建索引时执行精确扫描；`exact: true` 可绕过索引，`nprobes` 控制 IVF 分区探测范围。相似度不代表质量或画风概率。

来源过滤使用 Lance 的标签列表预过滤，再由 SQLite 解析来源身份。新发现的跨湖关联尚未投影完时，对受影响配方的来源过滤请求返回 `RESULT_NOT_READY`，避免静默漏掉候选；全来源查询仍可读取已发布向量。当前接口没有项目成员范围、文本／图文查询和工作集保存功能。

pHash 使用 64 位 DCT 哈希、8 个不重叠字节索引，支持半径 0–7。这个范围内至少一个字节保持不变，候选再经完整汉明距离核验。拥挤桶按游标继续扫描：即使返回空页，只要 `next_cursor` 非空就仍有候选待检查。pHash 关系不自动合并来源记录。

## 验证入口

```powershell
pnpm test:rust -p studio-visual-store
pnpm test:rust -p studio-sources visual_projection
pnpm test:visual
```

真实数据验证脚本为 [verify-visual-backend.py](../../tooling/verify-visual-backend.py)。它要求显式提供原注册库、模型目录、解释器、材料目录和输出目录；只读取原注册库，在自己的引擎目录复制来源注册信息，经正式 API 执行有界任务、暂停恢复、检索、索引和重复任务检查。它不会启动前端或自动扩展为全湖任务。

首次接入验证记录见 [2026-10-11 视觉后端验证](../verification/2026-10-11-visual-backend.md)。
