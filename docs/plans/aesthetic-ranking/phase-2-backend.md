# 第二阶段：离线排序与实验后端接入

日期：2026-09-18。第一阶段提交基线：`918add1054af016c5c2699c604e9fc4171e2559c`。

本阶段已完成从接受证据到统计快照、实验对照、保护复核、筛选预览、工作集派生及下一阶段输入的后端链路。没有新增前端页面。设计与数值语义见 [ADR 0026](../../decisions/0026-aesthetic-offline-analysis.md)。这是一组可以开展校准实验的实现，不是已经选定并验证了真实美学质量的生产算法。

## 模块入口

| 职责 | 源码 |
| --- | --- |
| 领域数据与冻结输入 | [domain/aesthetic_analysis.rs](../../../crates/studio-domain/src/aesthetic_analysis.rs) |
| 纯估计、分半重放、筛选与对照 | [application/aesthetic_analysis](../../../crates/studio-application/src/aesthetic_analysis/mod.rs) |
| 评审库投影、实验与复核 | [storage/aesthetic/analysis.rs](../../../crates/studio-storage/src/aesthetic/analysis.rs) |
| 工作集短事务构建和发布 | [analysis_project.rs](../../../crates/studio-storage/src/aesthetic/analysis_project.rs) |
| 离线执行器 | [engine/aesthetic/analysis.rs](../../../crates/studio-engine/src/aesthetic/analysis.rs) |
| HTTP | [api/aesthetic_analysis.rs](../../../crates/studio-engine/src/api/aesthetic_analysis.rs) |
| DTO 与 SDK | [protocol](../../../crates/studio-protocol/src/aesthetic_analysis.rs)、[client](../../../packages/client/src/aesthetic-analysis.ts) |

所有前端网络调用都应走 `client.aesthetic.analysis`。结果图片继续使用现有公开媒体接口和返回的 `AssetKey`；前端不加载原始响应做拟合，也不枚举百万个 ID 再提交后端。

## API 清单

公共前缀为 `/v1/projects/{project_id}/aesthetic/analysis`，继承项目租约和现有引擎认证。

| 方法 / 路径 | SDK | 作用 |
| --- | --- | --- |
| POST `/jobs` | `create` | 创建 `fit / compare / derive` 离线任务；幂等键绑定完整配置 |
| GET `/jobs` | `jobs` | 任务分页，可按 experiment_id 限定 |
| GET `/jobs/{id}` | `job` | 状态、进度、冻结输入、摘要和错误 |
| POST `/jobs/{id}/control` | `control` | `cancel / resume`；恢复沿用冻结证据 |
| GET `/snapshots/{id}` | `snapshot` | 只返回已发布拟合快照的元数据 |
| GET `/snapshots/{id}/rows` | `rows` | 排名键集分页，按 Rating 可选过滤 |
| GET `/snapshots/{id}/candidates/{ordinal}` | `candidate` | 单图的冻结统计 |
| POST `/snapshots/{id}/select` | `select` | 有界筛选预览；保护状态和复核水位随游标固定 |
| GET `/jobs/{id}/comparison` | `comparison` | 对照逐图结果分页 |
| POST / GET `/experiments` | `createExperiment / experiments` | 不可变实验定义与列表 |
| GET `/experiments/{id}` | `experiment` | 变体和创建时固定的各阶段证据 |
| POST `/experiments/{id}/run` | `runExperiment` | 幂等创建/启动各变体的离线任务 |
| POST `/reviews` | `review` | 追加保护复核决定 |
| GET `/snapshots/{id}/reviews` | `reviews` | 复核日志分页 |

原有评审的 `create / control / candidates / batches / attempts / backup` 保持不变。正式形状以自动生成的 [OpenAPI](../../../packages/contracts/openapi.json) 和 [TypeScript Schema](../../../packages/contracts/src/schema.d.ts) 为准。

## 最小接入示例

以下 TypeScript 只执行离线计算；`stageId` 来自已有评审阶段。浏览器提供 `crypto.randomUUID()`。

```ts
const analysis = client.aesthetic.analysis;
const fit = await analysis.create(projectId, {
  idempotency_key: crypto.randomUUID(),
  name: "离线校准",
  spec: {
    kind: "fit",
    config: {
      stage_id: stageId,
      estimator: {
        kind: "davidson_v1",
        iterations: 128,
        regularization: 0.1,
        tie_strength: 1,
      },
      stability_seed: 17,
    },
    experiment_id: null,
    variant: null,
  },
});
// 后续轮询 analysis.job(projectId, fit.id)，待 state === "completed"。
// 前端挂载时轮询、卸载时取消读取即可；执行不依赖页面存活。
```

完成后可分页预览，再按同一复核水位派生：

```ts
const filter = { ratings: ["g"], top_percent: 25, include_protected: true };
const preview = await analysis.select(projectId, fit.id, {
  filter, after: null, limit: 64,
});
// 有 next_cursor 时继续请求，必须携带相同 filter。
const derived = await analysis.create(projectId, {
  idempotency_key: crypto.randomUUID(),
  name: "G 类 Top25 与保护池",
  spec: {
    kind: "derive", snapshot_id: fit.id, filter,
    review_watermark: preview.review_watermark,
  },
});
// 完成后 result.kind === "derive"，result.collection_id 可直接用于新评审阶段。
```

`rows()` 中的 `protected` 表示快照证据内的原始顶级提名；`select()` 的 `effective_protected` 还应用指定复核水位内的人工决定。用户输入的名字、说明、操作者和理由在后端校验。未知估计器、非法 Top 百分比、游标条件不一致、幂等键冲突都有明确错误。

## 前端必须保持的语义

- `position` 是分页位置，不能标成全局名次。每个 Rating 独立；不连通时使用分量名次并说明覆盖情况。
- score 越大越靠前；percentile 越小越靠前，0 为最高端。名次区间来自计算分数并列，Top 截断保留并列整个组。
- `split_percentile_delta=null` 表示未运行、拟合未收敛或拆分后没有足够连接；不能显示为“非常稳定”。没有校准过的置信区间字段。
- `converged=false` 仍可查看实验结果，需要展示迭代限制。不同模型分数不能直接合并；对照结果 `comparable=false` 时应显示原因。
- `needs_review`、残差、对手草图都是诊断；顶级提名与保护决定不是最终质量标签。
- 筛选预览可能因扫描预算返回空页和 `next_cursor`。不要将空页当作结束，也不要在前端自动无限拉取全库；继续按钮/有界预取即可。
- 不要对普通请求失败自动重建付费评审。离线 resume 安全边界与付费批次的显式可能收费重试是两套操作。

## 存储与执行边界

项目库 v11、评审库 v2，各自先备份再升级。每次离线计算最多 100 万候选，全引擎一次只运行一个计算任务；16 个准入位置包含等待任务。新快照写入和工作集成员写入都分批提交，未完成结果不可见。恢复遵守原证据/复核水位，取消不发布半成品。

工作集发布后保留父快照、筛选规则和水位，可从项目对象详情查看谱系。新阶段的曝光、模型、Prompt 仍由阶段配置决定；不会把上轮累计曝光或分数悄悄当作下一阶段评审证据。

历史快照和失败任务的可恢复部分不按临时缓存策略回收。当前没有历史快照释放接口、自动多轮计划编排器、基于诊断的主动付费组批器，也没有强模型/人工分数融合。已有原图传输策略与约 28 Mbps 上传准入预算未改变。

## 验证入口

```powershell
pnpm contracts
pnpm check
pnpm test:integration
# 单独运行新增的后端链路：
node tooling/integration-aesthetic-analysis.mjs
# 独立纯统计规模探针；不写生产库、不读图片、不联网：
node tooling/cargo-run.mjs build -p studio-application --example aesthetic_replay --release
.\target\release\examples\aesthetic_replay.exe 10000 davidson_v1 128
.\target\release\examples\aesthetic_replay.exe 1000000 borda_v1 1
.\target\release\examples\aesthetic_replay.exe 1000000 davidson_v1 128
```

新增单元测试覆盖三结果梯度、并列、分量隔离、不可评判、提名不加分、分半信息不足、比较可比性、冻结水位、重放幂等、恢复、复核语义、v1 账本备份迁移以及工作集的隐藏/续写/发布/删除墓碑。新增集成脚本贯通公共 SDK、localhost mock 证据采集、两类估计器、实验、对照、筛选、派生、下一阶段冻结、复核水位、取消和重启。

本阶段最终验收和容量探针结果归档到 `.local/reports/aesthetic-phase-2-20260918/`，代码和测试工具可独立复跑。该探针只证明相应纯计算路径的规模行为，不能当作百万图片 API、SQLite 全量投影写入、真实审美质量或长期运行的容量承诺。

2026-09-18 验收：`pnpm check` 通过，包含 168 项 Rust 测试及已有 SDK 检查；19 组集成脚本全部通过。最终 Rating 索引读取调整后，又通过完整 check 和新增离线集成的 9 项检查；现有前端构建通过，未新增界面设计。没有调用商业模型或上传真实用户图片。

同机 Release 纯计算探针使用 4 次合成曝光、16 图一批，均包含主拟合和两次整批分半重放：

| 候选数 | 观测批数 | 估计器 | 一次测量耗时 | 采样进程峰值工作集 |
| ---: | ---: | --- | ---: | ---: |
| 10,000 | 2,500 | Davidson | 0.54 秒 | 7.4 MiB |
| 1,000,000 | 250,000 | Borda | 2.30 秒 | 230.3 MiB |
| 1,000,000 | 250,000 | Davidson | 75.96 秒 | 230.2 MiB |

两次 Davidson 主拟合均在 20 次迭代内达到数值收敛条件。主比较图覆盖全部候选且连通，但分半结果不满足完整可比较条件，`split_comparable=0`，未给出虚假的零波动。进程工作集每 50 ms 采样 Windows 峰值计数；这些是单次开发环境观测，期间有其它开发检查，不是专门清空缓存、独占机器的基准。探针按页生成证据并丢弃已输出行，不包括 SQLite 证据解码/快照落库、图片 IO、HTTP 或真实视觉判断。
