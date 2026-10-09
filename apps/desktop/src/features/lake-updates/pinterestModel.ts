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
  completed: "指定 Pin 已完成",
  completed_with_gaps: "完成 · 有缺口",
  done: "已归档",
  unavailable: "未获取",
};
export const pinterestActions: Record<string, string> = {
  pause: "暂停",
  resume: "恢复",
  cancel: "取消后续工作",
  retry: "重试未获取项",
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
export const pinterestRange = (spec: Schema["PinterestDefinition"]) =>
  spec.seeds.length === 1
    ? `Pin ${spec.seeds[0]!.id}`
    : `${spec.seeds.length} 个指定 Pin`;
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
};
export const pinterestReason = (reason: string) => reasons[reason] ?? reason;

export type PinterestDraft = {
  version: 1;
  lakeId: string;
  pins: string;
  requests: number;
  downloadMiB: number;
  minutes: number;
  submission: {
    key: string;
    spec: Schema["PinterestDefinition"];
    id?: string;
  } | null;
};
export const initialPinterestDraft: PinterestDraft = {
  version: 1,
  lakeId: "",
  pins: "",
  requests: 100,
  downloadMiB: 1024,
  minutes: 60,
  submission: null,
};
export function decodePinterestDraft(value: unknown): PinterestDraft | null {
  if (!value || typeof value !== "object") return null;
  const v = value as PinterestDraft;
  if (
    v.version !== 1 ||
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
  return v;
}
export function pinterestDefinition(
  d: PinterestDraft,
  libraryId: string,
): Schema["PinterestDefinition"] {
  const seeds = [
    ...new Set(
      d.pins
        .trim()
        .split(/[\s,，]+/)
        .filter(Boolean),
    ),
  ];
  if (!seeds.length || seeds.length > 500)
    throw new Error("请输入 1–500 个 Pin ID 或链接。");
  if (
    ![d.requests, d.downloadMiB * 1048576, d.minutes * 60].every(
      (n) => Number.isSafeInteger(n) && n > 0,
    )
  )
    throw new Error("请求数、下载预算和运行时间必须为正数。");
  return {
    version: 1,
    collector: "pinterest_web_v1",
    library_id: libraryId,
    seeds: seeds.map((id) => ({ kind: "pin", id })),
    access: { mode: "anonymous", language: "zh-TW" },
    scope: { media_types: ["image"], ai_policy: "record_only" },
    discovery: { entrypoints: [], max_depth: 0 },
    metadata: { detail_enrichment: "none" },
    media: {
      image_policy: {
        profile: "original",
        existing: "match_profile",
        allow_sample: false,
      },
      retain_original: true,
      reuse: { mode: "revalidate", max_age_hours: 0 },
    },
    run_budget: {
      api_requests: d.requests,
      detail_requests: d.requests,
      admitted_pins: Math.min(d.requests, 500),
      admitted_boards: 0,
      download_bytes: d.downloadMiB * 1048576,
      wall_seconds: d.minutes * 60,
    },
  };
}
