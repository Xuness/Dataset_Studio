import type { Schema } from "@studio/contracts";
import {
  defaultEncoding,
  imagePolicy,
  imagePolicyLabel,
} from "./imagePolicy.js";
import type { ImageFields } from "./imagePolicy.js";
export type Definition = Schema["LakeUpdateDefinition"];
export type UpdateJob = Schema["LakeUpdateJob"];
export type Lake = Schema["UpdateLake"];
export type Schedule = Schema["LakeUpdateSchedule"];
export const sites = {
  danbooru: "Danbooru",
  yandere: "Yandere",
  gelbooru: "Gelbooru",
};
export const lakeLabel = (lakes: readonly Lake[], id: string) => {
  const lake = lakes.find((l) => l.id === id);
  return lake ? sites[lake.site] : `数据湖 ${id.slice(0, 8)}`;
};
export const states: Record<Schema["LakeUpdateJobState"], string> = {
  queued: "排队中",
  running: "正在运行",
  paused: "已暂停",
  cancelled: "已取消",
  completed: "已完成",
  completed_with_exclusions: "完成 · 有未获取项",
  waiting_retry: "等待重试",
  waiting_space: "等待空间",
  waiting_credentials: "等待凭据",
  needs_review: "需要检查",
};
export const phases: Record<string, string> = {
  recovering: "恢复检查点",
  waiting_worker: "等待后台服务自动接续",
  metadata: "获取元数据",
  metadata_retry: "元数据扫描等待重试",
  publishing_metadata: "发布元数据",
  downloading: "下载图片",
  encoding: "处理图片",
  processing_image: "处理图片",
  publishing_media: "发布图片",
  completed: "完成",
  pipeline: "流水线运行",
  rate_wait: "等待请求额度",
  connecting: "连接图片服务器",
  verifying: "校验原件",
  waiting_encode: "等待编码",
  ready: "等待发布",
  waiting_resources: "等待队列或资源",
  paused: "已暂停",
  idle: "本轮已退出",
};
export const itemStates: Record<string, string> = {
  stored: "已保存",
  reused: "已复用",
  metadata_only: "仅元数据",
  metadata: "仅元数据",
  pending: "待下载",
  pending_metadata: "待刷新元数据",
  failed: "失败",
  needs_review: "需要检查",
  unavailable: "未获取",
  excluded: "不在范围内",
  skipped: "已跳过",
};
const itemReasons: Record<string, string> = {
  metadata_reused: "沿用已保存的元数据观察",
  metadata_requires_refresh: "已有图片可复用；缺少原始下载信息时刷新该帖子",
  refresh_cached_media_locator: "已保存的下载地址需重新获取",
  image_http_404: "图片地址暂未找到；可重试，不代表帖子已删除",
  image_color_profile_error: "图片色彩配置无法转换；原始下载已保留",
  image_source_incompatible: "源文件结构超出 PNG 兼容范围；重试时按当前规则重新处理",
  image_decode_error: "图片无法解码；重试时重新处理",
  image_policy_rejected: "被当前保存配方排除（如拒绝透明图片）",
  image_storage_error: "图片暂存失败；检查本地存储后重试",
  image_decode_or_storage_error: "图片解码或暂存失败；检查格式与本地存储",
  image_transport_failed: "图片传输中断；可从有效断点重试",
  unsupported_media: "视频或压缩包等当前不支持的媒体",
  source_deleted: "来源标记已删除",
  source_restricted: "来源标记访问受限",
  no_image_url: "元数据中没有可用的图片地址",
  not_returned_by_api: "本次 API 请求未返回该帖子",
  UPDATE_RESOURCE_LIMIT: "图片超过当前单文件、像素或内存预算",
};
export const itemReasonLabel = (reason: string | null | undefined) =>
  reason ? (itemReasons[reason] ?? reason) : "";
export const active = (job: UpdateJob) =>
  job.execution_active ||
  ["queued", "running", "waiting_retry", "waiting_space"].includes(job.state);
export const problemCount = (job: UpdateJob) =>
  ["failed", "needs_review", "unavailable"].reduce(
    (n, k) => n + (job.counts[k] ?? 0),
    0,
  );
export const processedCount = (job: UpdateJob) =>
  [
    "stored",
    "reused",
    "metadata",
    "metadata_only",
    "excluded",
    "skipped",
  ].reduce((n, state) => n + (job.counts[state] ?? 0), 0);
export const pendingCount = (job: UpdateJob) =>
  (job.counts.pending ?? 0) + (job.counts.pending_metadata ?? 0);
export const dateLabel = (value: string | null | undefined) =>
  value ? new Date(value).toLocaleString() : "—";
export const bytesLabel = (value: number | null | undefined) =>
  value == null
    ? "—"
    : value < 1048576
      ? (value / 1024).toFixed(1) + " KiB"
      : (value / 1048576).toFixed(1) + " MiB";
export function rangeLabel(spec: Definition) {
  const r = spec.range;
  switch (r.kind) {
    case "tags": {
      const all = r.query.all ?? [],
        any = r.query.any ?? [],
        none = r.query.none ?? [];
      const labels = [
        all.length ? `全部：${all.join(" + ")}` : "",
        any.length ? `任一：${any.join(" / ")}` : "",
        none.length ? `排除：${none.join(" / ")}` : "",
      ]
        .filter(Boolean)
        .join(" · ");
      const bounds =
        r.end_id != null
          ? ` · ID ${r.start_id ?? 1}–${r.end_id - 1}`
          : r.start_id && r.start_id > 1
            ? ` · ID ≥ ${r.start_id}`
            : "";
      return `${r.source === "local" ? "已有元数据" : "标签采集"} · ${labels || "全部条目"}${r.post_ids ? ` · 已选 ${r.post_ids.length} 条` : ""}${r.missing_media ? " · 尚无图片" : ""}${bounds}`;
    }
    case "new":
      return r.after_id == null
        ? "补充新帖 · 接续已验证基线"
        : `补充新帖 · ID > ${r.after_id}`;
    case "ids":
      return `指定 ${r.ids.length} 个帖子`;
    case "id_range":
      return `帖子 ${r.start}–${r.end - 1}`;
    case "local":
      return r.missing_media ? "补齐本地缺图" : "刷新已有记录";
    case "created":
    case "updated": {
      let start = r.start,
        end = r.end;
      try {
        const format = new Intl.DateTimeFormat("zh-CN", {
          timeZone: r.timezone,
          dateStyle: "short",
          timeStyle: "short",
        });
        start = format.format(new Date(r.start));
        end = format.format(new Date(r.end));
      } catch {
        /* Older timezone databases can still display the exact UTC bounds. */
      }
      return `${r.kind === "created" ? "创建日期" : "最后修改时间"} · ${start} 至 ${end}（不含末端）· ${r.timezone}`;
    }
    case "changes":
      return `变更序号 > ${r.after}`;
    case "input":
      return "固定成员范围";
  }
}
export const policyLabel = (spec: Definition) => imagePolicyLabel(spec.media);
export function actions(job: UpdateJob): Schema["LakeUpdateAction"][] {
  if (job.execution_active)
    return ["paused", "cancelled"].includes(job.state)
      ? []
      : ["pause", "cancel"];
  if (job.state === "cancelled") return [];
  if (job.state === "completed") return [];
  if (job.state === "completed_with_exclusions") return ["retry"];
  if (["queued", "running"].includes(job.state)) return ["pause", "cancel"];
  return ["resume", "retry", "replay", "cancel"];
}
export const actionLabels = {
  pause: "暂停",
  resume: "继续",
  retry: "重试未获取项",
  replay: "使用已保存响应重新解析",
  cancel: "取消后续工作",
};
export type FormDraft = ImageFields & {
  lakes: string[];
  kind:
    | "tags"
    | "new"
    | "local"
    | "missing"
    | "created"
    | "updated"
    | "ids"
    | "id_range"
    | "changes"
    | "input";
  inputIds: Record<string, string>;
  tagAll: string;
  tagAny: string;
  tagNone: string;
  tagSource: "remote" | "local";
  tagFreshness: string;
  tagRefreshAll: boolean;
  tagMissingMedia: boolean;
  perLake: Record<
    string,
    {
      ids?: string;
      after?: string;
      metadataIds?: string;
      catalogVersion?: string;
    }
  >;
  startId: string;
  endId: string;
  from: string;
  until: string;
  timezone: string;
  observedBefore: string;
  pageBudget: string;
  itemBudget: string;
  execution: "now" | "once" | "interval";
  firstRun: string;
  intervalHours: string;
  submissions: { key: string; spec: Definition; jobId?: string }[];
};
export const initialDraft: FormDraft = {
  lakes: [],
  kind: "tags",
  tagAll: "",
  tagAny: "",
  tagNone: "",
  tagSource: "remote",
  tagFreshness: "24",
  tagRefreshAll: false,
  tagMissingMedia: false,
  profile: "",
  encoding: defaultEncoding,
  existing: "keep",
  allowSample: false,
  perLake: {},
  inputIds: {},
  startId: "",
  endId: "",
  from: "",
  until: "",
  timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
  observedBefore: "",
  pageBudget: "1000",
  itemBudget: "100000",
  execution: "now",
  firstRun: "",
  intervalHours: "24",
  submissions: [],
};
export function decodeDraft(value: unknown): FormDraft | null {
  if (!value || typeof value !== "object") return null;
  const v = value as FormDraft;
  if (
    (v.tagSource !== undefined && !["remote", "local"].includes(v.tagSource)) ||
    (v.tagRefreshAll !== undefined && typeof v.tagRefreshAll !== "boolean") ||
    (v.tagMissingMedia !== undefined &&
      typeof v.tagMissingMedia !== "boolean") ||
    [v.tagAll, v.tagAny, v.tagNone, v.tagFreshness].some(
      (field) => field !== undefined && typeof field !== "string",
    )
  )
    return null;
  if (
    !Array.isArray(v.lakes) ||
    !v.lakes.every((id) => typeof id === "string") ||
    ![
      "tags",
      "new",
      "local",
      "missing",
      "created",
      "updated",
      "ids",
      "id_range",
      "changes",
      "input",
    ].includes(v.kind)
  )
    return null;
  if (
    !["", "metadata_only", "original", "webp-2048-q95", "custom"].includes(
      v.profile,
    ) ||
    !["now", "once", "interval"].includes(v.execution)
  )
    return null;
  return { ...initialDraft, ...v, encoding: v.encoding ?? defaultEncoding };
}
function integer(
  value: string,
  name: string,
  min = 1,
  max = Number.MAX_SAFE_INTEGER - 1,
) {
  const n = Number(value);
  if (!value.trim() || !Number.isSafeInteger(n) || n < min || n > max)
    throw new Error(`${name}需为 ${min} 至 ${max} 的整数`);
  return n;
}
/** Calendar boundaries are resolved in the chosen IANA zone, including DST. */
export function dateBoundary(
  value: string,
  zone: string,
  nextDay = false,
): string {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) throw new Error("请选择完整日期");
  const [y, m, d] = value.split("-").map(Number) as [number, number, number];
  if (new Date(Date.UTC(y, m - 1, d)).toISOString().slice(0, 10) !== value)
    throw new Error("日期无效");
  const desired = Date.UTC(y, m - 1, d + (nextDay ? 1 : 0));
  const formatter = new Intl.DateTimeFormat("en-CA", {
    timeZone: zone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
  });
  let result = desired;
  for (let i = 0; i < 4; i++) {
    const p = Object.fromEntries(
      formatter.formatToParts(result).map((p) => [p.type, p.value]),
    );
    const actual = Date.UTC(
      Number(p.year),
      Number(p.month) - 1,
      Number(p.day),
      Number(p.hour),
      Number(p.minute),
      Number(p.second),
    );
    if (actual === desired) return new Date(result).toISOString();
    result += desired - actual;
  }
  throw new Error("该时区的日期边界不存在，请调整日期或时区");
}
export function definitions(d: FormDraft): Definition[] {
  if (!d.lakes.length) throw new Error("请选择数据湖");
  if (!d.profile) throw new Error("请明确选择图片保存策略");
  if (d.kind === "missing" && d.profile === "metadata_only")
    throw new Error("补齐缺图需要选择图片保存策略");
  const bounded = !["new", "ids", "input"].includes(d.kind);
  const bounds = {
    start_id: bounded && d.startId ? integer(d.startId, "起始 ID") : null,
    end_id: bounded && d.endId ? integer(d.endId, "结束 ID") + 1 : null,
  };
  if (bounds.start_id && bounds.end_id && bounds.end_id <= bounds.start_id)
    throw new Error("结束 ID 不能早于起始 ID");
  return d.lakes.map((library_id) => {
    const local = d.perLake[library_id] ?? {};
    let range: Definition["range"];
    switch (d.kind) {
      case "tags": {
        const split = (value: string) => [
          ...new Set(value.split(/[\s,，]+/).filter(Boolean)),
        ];
        const query = {
          all: split(d.tagAll),
          any: split(d.tagAny),
          none: split(d.tagNone),
        };
        if (d.tagSource !== "local" && !query.all.length && !query.any.length)
          throw new Error(
            "请至少填写一个需要包含的 Tag；画师或角色使用源站标签",
          );
        const postIds =
          d.tagSource === "local" && local.metadataIds?.trim()
            ? split(local.metadataIds).map((id) => integer(id, "帖子 ID"))
            : undefined;
        if (postIds && postIds.length > 10000)
          throw new Error("一次最多选择 10000 个帖子");
        range = {
          kind: "tags",
          query,
          ...bounds,
          source: d.tagSource,
          ...(d.tagSource === "local"
            ? {
                missing_media: d.tagMissingMedia,
                ...(postIds ? { post_ids: postIds } : {}),
                ...(local.catalogVersion && d.execution === "now"
                  ? { version: local.catalogVersion }
                  : {}),
              }
            : {
                refresh: {
                  mode: d.tagRefreshAll ? "all" : "missing_or_stale",
                  max_age_hours: integer(
                    d.tagFreshness,
                    "元数据新鲜度",
                    1,
                    8760,
                  ),
                },
              }),
        };
        break;
      }
      case "input": {
        const input_id = d.inputIds[library_id];
        if (!input_id)
          throw new Error("目标湖缺少已封存的固定输入，请重新选择准备结果");
        range = { kind: "input", input_id };
        break;
      }
      case "new":
        range = {
          kind: "new",
          after_id: local.after ? integer(local.after, "新帖起点", 0) : null,
        };
        break;
      case "ids": {
        const ids = [
          ...new Set(
            (local.ids ?? "")
              .split(/[\s,，]+/)
              .filter(Boolean)
              .map((v) => integer(v, "帖子 ID")),
          ),
        ];
        if (!ids.length || ids.length > 10000)
          throw new Error("每个湖需填写 1–10000 个帖子 ID");
        range = { kind: "ids", ids };
        break;
      }
      case "id_range":
        range = {
          kind: "id_range",
          start: integer(d.startId, "起始 ID"),
          end: integer(d.endId, "结束 ID") + 1,
        };
        break;
      case "local":
      case "missing":
        range = {
          kind: "local",
          ...bounds,
          missing_media: d.kind === "missing",
          observed_before: d.observedBefore
            ? new Date(d.observedBefore).toISOString()
            : null,
        };
        break;
      case "changes":
        range = {
          kind: "changes",
          ...bounds,
          after: integer(local.after ?? "", "变更序号", 0),
        };
        break;
      case "created":
      case "updated": {
        const start = dateBoundary(d.from, d.timezone),
          end = dateBoundary(d.until, d.timezone, true);
        if (start >= end) throw new Error("结束日期不能早于开始日期");
        range = { kind: d.kind, ...bounds, start, end, timezone: d.timezone };
        break;
      }
    }
    return {
      library_id,
      range,
      media: imagePolicy(d),
      page_budget: integer(d.pageBudget, "页数预算", 1, 100000),
      item_budget: integer(d.itemBudget, "记录预算", 1, 10000000),
    };
  });
}
