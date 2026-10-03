import type { Job } from "@studio/contracts";

export const jobStatusNames: Record<string, string> = {
  waiting_input: "正在确定输入",
  queued: "排队中",
  preparing: "准备中",
  running: "运行中",
  succeeded: "已完成",
  failed: "失败",
  cancelled: "已取消",
};

export function isJobActive(job: Pick<Job, "status"> | undefined) {
  return (
    !!job &&
    ["waiting_input", "queued", "preparing", "running"].includes(job.status)
  );
}

const stages: Record<
  string,
  { label: string; phase: number; unit?: string; detail: string }
> = {
  fixing_members: {
    label: "保存任务输入",
    phase: 0,
    unit: "项",
    detail: "正在保存本次输入范围。",
  },
  waiting_input: {
    label: "确定输入范围",
    phase: 0,
    detail: "正在固定本次输入成员。",
  },
  queued: {
    label: "等待执行",
    phase: 0,
    detail: "任务已提交，等待引擎安排执行。",
  },
  restoring_input: {
    label: "核验恢复材料",
    phase: 1,
    detail: "正在核验之前保存的固定输入。",
  },
  snapshot_index: {
    label: "整理元数据快照",
    phase: 1,
    detail: "正在核对成员并整理重复图片。",
  },
  input_checksum: {
    label: "校验输入快照",
    phase: 1,
    unit: "bytes",
    detail: "正在确认输入快照完整。",
  },
  waiting_resources: {
    label: "等待计算资源",
    phase: 2,
    detail: "正在等待可用的计算内存与执行额度。",
  },
  validating_input: {
    label: "复核固定输入",
    phase: 3,
    unit: "bytes",
    detail: "正在核对结果对应的输入快照。",
  },
  validating: {
    label: "校验评分与名次",
    phase: 3,
    unit: "项",
    detail: "正在逐项核对评分、名次和候选名额。",
  },
  output_checksum: {
    label: "校验成果文件",
    phase: 3,
    unit: "bytes",
    detail: "正在确认成果文件完整。",
  },
  scope_basis: {
    label: "固定输入范围",
    phase: 0,
    unit: "项",
    detail: "正在保存本次计算的输入与来源版本。",
  },
  metadata_snapshot: {
    label: "读取元数据快照",
    phase: 1,
    unit: "项",
    detail: "正在读取并整理元数据，随后开始评分。",
  },
  snapshot_reuse: {
    label: "复用固定元数据",
    phase: 1,
    unit: "bytes",
    detail: "正在核验同一来源版本的元数据，并为本次排名保存独立快照。",
  },
  eligibility: {
    label: "检查候选资格",
    phase: 2,
    unit: "项",
    detail: "正在检查分级、用途条件和元数据完整性。",
  },
  loading_rating: {
    label: "准备分级数据",
    phase: 2,
    unit: "项",
    detail: "G、S、Q、E 分别计算；进度对应当前分级。",
  },
  heat: {
    label: "计算热度位置",
    phase: 2,
    detail: "正在计算当前分级的热度分布。",
  },
  time: {
    label: "计算时间补救",
    phase: 2,
    unit: "轮",
    detail: "正在逐轮扩展比较群体。各轮耗时可能不同。",
  },
  time_window: {
    label: "计算时间邻域",
    phase: 2,
    unit: "项",
    detail: "进度对应当前分级、当前轮次的比较群体。",
  },
  artists: {
    label: "计算画师先验",
    phase: 2,
    unit: "项",
    detail: "正在汇总当前分级的画师作品。",
  },
  scores: {
    label: "计算分数",
    phase: 2,
    detail: "正在合并热度、补救与扣分项。",
  },
  ranks: {
    label: "排序与分配名额",
    phase: 2,
    unit: "步",
    detail: "正在整理主排名、补救排名及候选名额。",
  },
  v2_periods: {
    label: "准备年代参考",
    phase: 2,
    unit: "项",
    detail: "正在分配跨年羽化权重。",
  },
  v2_feather: {
    label: "计算羽化分布",
    phase: 2,
    unit: "组",
    detail: "正在统计相近帖龄与年代的加权分布。",
  },
  v2_weighted_ranks: {
    label: "计算年代相对位置",
    phase: 2,
    unit: "项",
    detail: "进度对应当前年代参考群体。",
  },
  v2_scores: {
    label: "计算 v2 分数",
    phase: 2,
    unit: "项",
    detail: "正在合并分级公式与年代相对表现。",
  },
  v2_selection: {
    label: "分配候选预算",
    phase: 2,
    unit: "项",
    detail: "正在执行补救保护与年代目标。",
  },
  v2_audit: {
    label: "分配随机审计",
    phase: 2,
    unit: "项",
    detail: "审计名额包含在保留预算内。",
  },
  v2_diagnostics: {
    label: "汇总年代诊断",
    phase: 2,
    unit: "项",
    detail: "正在记录前段分布、回退与类型保护。",
  },
  writing: {
    label: "保存评分明细",
    phase: 2,
    unit: "项",
    detail: "正在保存当前分级的评分与排名。",
  },
  indexing: {
    label: "建立榜单索引",
    phase: 3,
    detail: "评分已生成，正在建立可浏览的榜单索引。",
  },
  complete: {
    label: "校验排名成果",
    phase: 3,
    detail: "评分已生成，等待校验与保存完成。",
  },
  publishing: {
    label: "保存项目成果",
    phase: 3,
    detail: "正在核验完整性并保存成果；完成后即可查看榜单。",
  },
};

export const rankingPhases = [
  "固定输入",
  "读取元数据",
  "评分与排序",
  "校验与保存",
];
const formatCount = (value: number) => value.toLocaleString("zh-CN");
const formatBytes = (value: number) =>
  value >= 1024 ** 3
    ? `${(value / 1024 ** 3).toFixed(1)} GiB`
    : value >= 1024 ** 2
      ? `${(value / 1024 ** 2).toFixed(1)} MiB`
      : `${formatCount(value)} 字节`;
export const jobPhaseLabel = (name: string, rating?: string | null) =>
  `${stages[name]?.label ?? name}${rating ? ` · ${rating.toUpperCase()} 分级` : ""}`;
export function jobDuration(milliseconds: number) {
  if (!Number.isFinite(milliseconds)) return "未知";
  const seconds = Math.max(0, Math.floor(milliseconds / 1000));
  if (seconds < 60) return `${seconds} 秒`;
  if (seconds < 3600)
    return `${Math.floor(seconds / 60)} 分 ${seconds % 60} 秒`;
  return `${Math.floor(seconds / 3600)} 小时 ${Math.floor((seconds % 3600) / 60)} 分`;
}
export function jobPresentation(job: Job) {
  const ranking = ["danbooru.metarecall", "danbooru.metarecall_v2"].includes(
    job.operator,
  );
  const active = isJobActive(job);
  const succeeded = job.status === "succeeded";
  const stage = job.stage;
  // Queued/retried jobs can still contain the previous attempt's stage.
  const usableStage =
    stage &&
    (stage.name === "fixing_members" ||
      !["queued", "waiting_input"].includes(job.status)) &&
    !(
      job.status === "preparing" &&
      !stage.telemetry &&
      (stages[stage.name]?.phase ?? 0) > 1
    );
  const definition = usableStage ? stages[stage.name] : undefined;
  const completed = usableStage ? stage.completed : job.completed;
  const total = usableStage ? stage.total : job.total;
  const measurable =
    succeeded ||
    (total > 0 &&
      (stage?.name === "fixing_members" ||
        (ranking
          ? !!definition?.unit
          : job.input_members_frozen && job.status !== "queued")));
  const percent = succeeded
    ? 100
    : measurable
      ? Math.min(100, Math.max(0, (completed / total) * 100))
      : null;
  const title = succeeded
    ? ranking
      ? "本次排名已完成"
      : "任务已完成"
    : job.status === "failed"
      ? ranking
        ? "排名失败"
        : "任务失败"
      : job.status === "cancelled"
        ? ranking
          ? "排名已取消"
          : "任务已取消"
        : definition
          ? jobPhaseLabel(stage!.name, stage?.rating)
          : (jobStatusNames[job.status] ?? job.status);
  const detail = succeeded
    ? "成果已保存到项目，可以查看或继续处理。"
    : job.status === "failed"
      ? "本次执行未完成。查看错误后可重试固定输入。"
      : job.status === "cancelled"
        ? "本次执行已停止，可按固定输入重新执行。"
        : job.status === "waiting_input"
          ? "任务已提交，正在确定输入成员与数量。"
          : job.status === "queued"
            ? "任务已提交，等待引擎安排执行。"
            : (definition?.detail ?? "任务已开始，正在等待下一次进度更新。");
  return {
    active,
    succeeded,
    ranking,
    title,
    detail,
    percent,
    status: jobStatusNames[job.status] ?? job.status,
    phase: succeeded ? 4 : (definition?.phase ?? 0),
    progressLabel: ranking && !succeeded ? "当前步骤" : "处理进度",
    count: succeeded
      ? `${formatCount(job.total)} 项输入已处理`
      : measurable
        ? definition?.unit === "bytes"
          ? `${formatBytes(completed)} / ${formatBytes(total)}`
          : `${formatCount(completed)} / ${formatCount(total)} ${definition?.unit ?? "项"}`
        : job.status === "queued"
          ? job.input_members_frozen
            ? `已固定 ${formatCount(job.total)} 项，等待执行`
            : "等待确定输入"
          : job.status === "waiting_input"
            ? stage?.completed
              ? `已读取 ${formatCount(stage.completed)} 项，正在确定总量`
              : "输入数量尚未确定"
            : active
              ? "正在处理，进度持续更新"
              : "本次执行已停止",
  };
}

export function jobSubmittedAt(job: Pick<Job, "created_at">) {
  const value = Number(job.created_at);
  return Number.isFinite(value) && value > 0
    ? new Date(value).toLocaleString("zh-CN", {
        month: "2-digit",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
      })
    : "时间未知";
}
