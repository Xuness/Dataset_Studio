import type { StudioClient } from "@studio/client";
import type { Schema } from "@studio/contracts";
export type PinterestJob = Schema["PinterestJob"];
export const pinterestStates: Record<string, string> = {
  queued: "排队中",
  running: "正在采集",
  pausing: "正在暂停",
  paused: "已暂停",
  cancelling: "正在取消",
  cancelled: "已取消",
  waiting_retry: "等待重试",
  waiting_resources: "等待资源",
  waiting_budget: "本轮预算用完",
  needs_review: "需要检查",
  completed: "本次范围已处理",
  completed_with_gaps: "完成 · 有缺口",
  done: "已归档",
  unavailable: "未获取",
  superseded: "已转为详情确认",
  active: "仍有后续页",
  exhausted: "源站本次返回结束",
};
export const pinterestActions: Record<string, string> = {
  pause: "暂停",
  resume: "恢复",
  cancel: "取消后续工作",
  retry: "重试未获取项",
  continue: "追加一轮预算并继续",
};
export const pinterestActive = (job: PinterestJob) =>
  [
    "queued",
    "running",
    "pausing",
    "cancelling",
    "waiting_retry",
    "waiting_resources",
  ].includes(job.state);
export const pinterestCount = (
  job: PinterestJob,
  states: string[],
  kind?: string,
) =>
  job.counts
    .filter((c) => states.includes(c.state) && (!kind || c.kind === kind))
    .reduce((n, c) => n + c.n, 0);
export const pinterestSeedLabels: Record<string, string> = {
  pin: "Pin",
  board: "图版",
  section: "分区",
  search_pins: "Pin 搜索",
  search_boards: "图版搜索",
  topic: "Ideas 主题",
};
export const pinterestTaskLabels: Record<string, string> = {
  pin_detail: "Pin 详情",
  pin_enrichment: "元数据补取",
  pin_admit: "Pin 准入",
  media_download: "原图获取",
  board_resolve: "图版链接解析",
  section_resolve: "分区链接解析",
  board_admit: "图版 / 分区准入",
  board_page: "图版成员",
  section_page: "分区成员",
  board_sections: "分区列表",
  board_more_ideas: "图版 More ideas",
  related_pins: "Pin 相关推荐",
  search_page: "搜索",
  topic_page: "Ideas 主题",
};
export const pinterestEntrypoints: Record<string, string> = {
  board_more_ideas: "图版 More ideas",
  related_pins: "Pin 相关推荐",
  pin_boards: "Pin 所在图版",
  topic_boards: "主题相关图版",
};
export const pinterestRange = (spec: Schema["PinterestDefinition"]) =>
  spec.seeds.length === 1
    ? `${pinterestSeedLabels[spec.seeds[0]!.kind] ?? "种子"} ${spec.seeds[0]!.id}`
    : spec.seeds.every((s) => s.kind === "pin")
      ? `${spec.seeds.length} 个指定 Pin`
      : `${spec.seeds.length} 个${pinterestSeedLabels[spec.seeds[0]?.kind ?? ""] ?? ""}种子`;
export async function pinterestAction(
  client: StudioClient,
  id: string,
  action: string,
) {
  const job = await client.pinterestCollections.job(id);
  return client.pinterestCollections.action(id, {
    action,
    expected_revision: job.revision,
  });
}
const reasons: Record<string, string> = {
  media_decision_fields_missing: "响应缺少判断媒体类型所需的字段",
  complex_story_not_supported: "故事需要合成或包含多个媒体，当前保留缺口",
  video_not_supported: "含有视频，当前不下载封面作为原图",
  story_video_not_supported: "故事包含视频，当前保留缺口",
  carousel_not_supported: "轮播图片尚未支持",
  animation_not_supported: "文件包含动画，当前保留缺口",
  image_format_not_supported: "暂不支持该文件格式",
  image_decode_failed: "下载文件无法完整解码",
  original_field_missing: "本次响应没有明确的原图字段",
  original_field_invalid: "原图地址或尺寸无效",
  media_decision_fields_invalid: "媒体类型判断字段无效",
  story_shape_unknown: "故事结构暂时无法识别",
  story_image_identity_unknown: "故事图片与 Pin 的身份关联尚不明确",
  PINTEREST_STORAGE_CHECK_REQUIRED: "归档需要检查，已保留当前进度",
  PINTEREST_NETWORK: "来源请求失败，稍后重试",
  PINTEREST_ACQUISITION_LOST: "暂存原图缺失或改变，需要明确重试",
  PINTEREST_ACQUISITION_CHANGED: "下载内容在校验前发生变化，需要检查",
  UPDATE_SPACE: "等待暂存空间或下载资源",
  UPDATE_RESOURCE_LIMIT: "图片超出当前资源预算",
  shared_url_dimensions_mismatch: "共用图片地址的尺寸与该 Pin 声明不一致",
  original_dimensions_mismatch: "原图实际尺寸与来源声明不一致",
  cdn_etag_md5_mismatch: "CDN 校验标记与下载内容不一致",
  pin_response_invalid: "来源响应无法识别",
  pin_identity_or_shape_invalid: "来源响应的 Pin 身份或结构不符",
  repeated_cursor: "源站重复返回已处理游标，保留响应待检查",
  pagination_signal_missing_or_invalid: "来源缺少有效分页信号，尚不能确认结束",
  pagination_signal_conflict: "来源分页信号互相矛盾",
  discovery_response_shape_invalid: "发现响应结构无法识别，原始响应已保留",
  discovery_visibility_changed:
    "访问地区、语言或会话条件改变，已停止沿用游标；请在确认访问条件后新建一轮扫描",
  unrecognized_discovery_items: "返回条目类型无法识别",
  source_identity_or_shape_invalid: "来源实体身份或结构不符",
  list_detail_media_difference: "列表与抽样详情的媒体不同，本扫描改为详情确认",
  list_manifest_requires_detail: "列表清单已转交详情确认",
  empty_result_reason_unknown: "本次为空结果，原因未知",
  empty_result_confirmed: "空页结束信号已复查，或已与来源计数核对",
  discovery_end_unconfirmed: "分页途中收到空页结束信号，正在保留原游标复查",
  discovery_count_shortfall:
    "已观察的不同 Pin 少于来源报告数量，保留原游标复查；重试后仍不足会保留缺口",
  discovery_backlog: "等待已有候选与下载积压处理",
  api_requests: "本轮来源请求预算用完",
  detail_requests: "本轮详情 / 补取预算用完",
  admitted_pins: "本轮 Pin 准入预算用完",
  admitted_boards: "本轮图版准入预算用完",
  download_bytes: "本轮下载预算用完",
  cdn_etag_changed_on_304: "来源返回了相互矛盾的文件校验标记",
  PINTEREST_STORED_OBJECT_CHANGED: "已归档文件缺失或字节校验失败，需要检查存储",
};
export const pinterestReason = (reason: string) =>
  reasons[reason] ??
  (reason.startsWith("entry_requests:")
    ? `${pinterestTaskLabels[reason.slice(15)] ?? "发现入口"}请求预算用完`
    : reason);

export type PinterestDraft = {
  version: 2;
  lakeId: string;
  pins: string;
  requests: number;
  seedKind: string;
  detailRequests: number;
  pinBudget: number;
  boardBudget: number;
  metadataPolicy: "none" | "sample" | "all";
  sampleSize: number;
  entrypoints: string[];
  depth: number;
  includeSections: boolean;
  entryRequests: number;
  maxPending: number;
  reuseMode: "none" | "revalidate" | "historical";
  reuseHours: number;
  periodic: boolean;
  intervalHours: number;
  scheduleEnabled: boolean;
  downloadMiB: number;
  minutes: number;
  submission: {
    key: string;
    spec: Schema["PinterestDefinition"];
    id?: string;
    schedule?: {
      id: string;
      everySeconds: number;
      firstRunAt: string;
      enabled: boolean;
    };
  } | null;
};
export const initialPinterestDraft: PinterestDraft = {
  version: 2,
  lakeId: "",
  pins: "",
  requests: 100,
  seedKind: "pin",
  detailRequests: 100,
  pinBudget: 100,
  boardBudget: 20,
  metadataPolicy: "sample",
  sampleSize: 3,
  entrypoints: [],
  depth: 1,
  includeSections: true,
  entryRequests: 100,
  maxPending: 128,
  reuseMode: "none",
  reuseHours: 24,
  periodic: false,
  intervalHours: 24,
  scheduleEnabled: true,
  downloadMiB: 1024,
  minutes: 60,
  submission: null,
};
export function decodePinterestDraft(value: unknown): PinterestDraft | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Omit<PinterestDraft, "version"> & { version: number };
  if (
    ![1, 2].includes(v.version) ||
    typeof v.lakeId !== "string" ||
    typeof v.pins !== "string" ||
    ![v.requests, v.downloadMiB, v.minutes].every(
      (n) => typeof n === "number" && Number.isFinite(n),
    )
  )
    return null;
  if (
    v.submission !== null &&
    (!v.submission ||
      typeof v.submission.key !== "string" ||
      !v.submission.spec ||
      v.submission.spec.collector !== "pinterest_web_v1" ||
      !Array.isArray(v.submission.spec.seeds))
  )
    return null;
  const result: PinterestDraft = { ...initialPinterestDraft, ...v, version: 2 };
  if (
    ![result.includeSections, result.periodic, result.scheduleEnabled].every(
      (b) => typeof b === "boolean",
    )
  )
    return null;
  if (v.version === 1) {
    result.metadataPolicy = "none";
    result.detailRequests = v.requests;
    result.pinBudget = Math.min(v.requests, 500);
    result.boardBudget = 0;
  }
  if (
    !Object.hasOwn(pinterestSeedLabels, result.seedKind) ||
    ![
      result.detailRequests,
      result.pinBudget,
      result.boardBudget,
      result.sampleSize,
      result.depth,
      result.entryRequests,
      result.maxPending,
      result.reuseHours,
      result.intervalHours,
    ].every(Number.isFinite) ||
    !Array.isArray(result.entrypoints) ||
    result.entrypoints.some((s) => !Object.hasOwn(pinterestEntrypoints, s)) ||
    !["none", "sample", "all"].includes(result.metadataPolicy) ||
    !["none", "revalidate", "historical"].includes(result.reuseMode)
  )
    return null;
  return result;
}
export function pinterestDefinition(
  d: PinterestDraft,
  libraryId: string,
): Schema["PinterestDefinition"] {
  const seeds = [
    ...new Set(
      d.pins
        .trim()
        .split(d.seedKind.startsWith("search_") ? /[\r\n]+/ : /[\s,，]+/)
        .filter(Boolean),
    ),
  ];
  if (!seeds.length || seeds.length > 500)
    throw new Error("请输入 1–500 个种子，搜索查询每行一条。");
  if (
    ![d.requests, d.pinBudget, d.downloadMiB * 1048576, d.minutes * 60].every(
      (n) => Number.isSafeInteger(n) && n > 0,
    )
  )
    throw new Error("请求数、下载预算和运行时间必须为正数。");
  if (
    ![d.detailRequests, d.boardBudget, d.entryRequests].every(
      (n) => Number.isSafeInteger(n) && n >= 0,
    )
  )
    throw new Error("详情、图版和入口预算必须为非负整数。");
  return {
    version: 1,
    collector: "pinterest_web_v1",
    library_id: libraryId,
    seeds: seeds.map((id) => ({ kind: d.seedKind, id })),
    access: { mode: "anonymous", language: "zh-TW" },
    scope: { media_types: ["image"], ai_policy: "record_only" },
    discovery: {
      entrypoints: d.entrypoints,
      max_depth: d.entrypoints.length ? d.depth : 0,
      include_sections: d.includeSections,
      max_pending_downloads: d.maxPending,
      entry_requests: Object.fromEntries(
        [
          "board_page",
          "section_page",
          "board_sections",
          "board_more_ideas",
          "related_pins",
          "search_page",
          "topic_page",
        ].map((k) => [k, d.entryRequests]),
      ),
    },
    metadata: {
      detail_enrichment: d.metadataPolicy,
      sample_size: d.sampleSize,
    },
    media: {
      image_policy: {
        profile: "original",
        existing: "match_profile",
        allow_sample: false,
      },
      retain_original: true,
      reuse: {
        mode: d.reuseMode === "historical" ? "historical" : "revalidate",
        max_age_hours: d.reuseMode === "none" ? 0 : d.reuseHours,
      },
    },
    run_budget: {
      api_requests: d.requests,
      detail_requests: d.detailRequests,
      admitted_pins: d.pinBudget,
      admitted_boards: d.boardBudget,
      download_bytes: d.downloadMiB * 1048576,
      wall_seconds: d.minutes * 60,
    },
  };
}
