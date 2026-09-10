import type { ObjectKind, ScopeRef } from "@studio/contracts";
import type { ObjectTarget } from "@studio/client";
export const objectNames: Record<ObjectKind, string> = {
  project: "项目",
  source: "数据湖",
  workset: "工作集",
  artifact: "计算成果",
  query: "查询条件",
  job: "任务记录",
  query_result: "查询结果",
  selection: "当前选择",
  selection_history: "选择撤销历史",
};
export const objectStates: Record<string, string> = {
  attached: "已关联",
  detached: "未关联",
  ready: "可用",
  released: "已删除",
  deleted: "已删除",
  saved: "已保存",
  open: "已打开",
  queued: "排队中",
  waiting_input: "准备输入中",
  preparing: "准备中",
  running: "执行中",
  succeeded: "已完成",
  failed: "失败",
  cancelled: "已取消",
  interrupted: "已中断",
  publishing: "发布中",
  unavailable: "不可用",
  legacy: "待校验",
};
export function targetFromScope(
  scope: ScopeRef | null | undefined,
): ObjectTarget | null {
  const target = scope?.target;
  if (!target) return null;
  if (target.kind === "source") return { kind: "source", id: target.source_id };
  if (target.kind === "workset")
    return { kind: "workset", id: target.collection_id };
  if (target.kind === "query_result")
    return { kind: "query_result", id: target.result_id };
  return { kind: "selection", id: "selection" };
}
export function readableTime(value: string | null | undefined) {
  return value && Number.isFinite(Number(value))
    ? new Date(Number(value)).toLocaleString("zh-CN")
    : "未记录";
}
export function objectBytes(value: string | null | undefined) {
  if (value == null) return "未记录";
  const bytes = Number(value);
  return bytes >= 1024 ** 3
    ? (bytes / 1024 ** 3).toFixed(2) + " GiB"
    : bytes >= 1024 ** 2
      ? (bytes / 1024 ** 2).toFixed(1) + " MiB"
      : bytes >= 1024
        ? (bytes / 1024).toFixed(1) + " KiB"
        : bytes + " B";
}
