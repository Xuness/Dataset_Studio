import { StudioError } from "@studio/client";
import type { StudioClient } from "@studio/client";
import type { Schema } from "@studio/contracts";
import {
  defaultEncoding,
  fieldsForPolicy,
  imagePolicy,
} from "./imagePolicy.js";
import type { ImageFields } from "./imagePolicy.js";

export type CollectionJob = Schema["CollectionJob"];
export type CollectionDefinition = Schema["CollectionJobDefinition"];
export type CollectionAction = Schema["CollectionAction"];
export type WorkspaceLake = Schema["LakeWorkspaceLake"];
export const collectionStates: Record<Schema["CollectionJobState"], string> = {
  queued: "排队中",
  running: "正在采集",
  pausing: "正在暂停",
  paused: "已暂停",
  waiting_credentials: "等待登录凭据",
  waiting_retry: "等待重试",
  waiting_resources: "等待资源",
  waiting_budget: "本轮预算用完",
  publishing: "正在发布",
  needs_review: "需要检查",
  cancelling: "正在取消",
  cancelled: "已取消",
  completed: "已完成",
  completed_with_gaps: "完成 · 有缺口",
};
export const collectionActions: Record<CollectionAction, string> = {
  pause: "暂停",
  resume: "继续下一轮",
  retry_failed: "重试未获取项",
  cancel: "取消后续工作",
  replay_publication: "补发已归档数据",
};
export const taskKinds: Record<string, string> = {
  author_profile: "作者资料",
  author_directory: "作者作品目录",
  work_detail: "作品详情",
  media_manifest: "逐页与动画清单",
  relationship_page: "关系发现",
  media_download: "媒体取得",
};
export const taskStates: Record<string, string> = {
  queued: "待处理",
  running: "正在处理",
  done: "已完成",
  staged: "等待归档",
  archived: "已归档",
  retry_wait: "等待重试",
  waiting_credentials: "等待凭据",
  waiting_resources: "等待资源",
  unavailable: "本次未获取",
  excluded: "不在范围内",
  needs_review: "需要检查",
  cancelled: "已取消",
};
const reasons: Record<string, string> = {
  run_budget_exhausted: "本轮预算用完，继续会保留进度并重置本轮用量。",
  fresh_existing_snapshot: "沿用近期完整快照",
  scope_filter: "不符合所选范围",
  depth_limit: "达到扩展深度",
  COLLECTION_CREDENTIAL_REQUIRED: "需要导入或更新登录会话",
  COLLECTION_SCOPE_CHANGED: "会话身份或可见条件发生变化，请检查后新建快照",
  COLLECTION_NOT_ACCESSIBLE: "来源本次未提供该内容",
  COLLECTION_SOURCE_CHANGED: "来源内容与本次清单不一致，可新建复查任务",
  COLLECTION_REMOTE_UNAVAILABLE: "远端暂时不可用，将按冷却时间重试",
  COLLECTION_MEDIA_INVALID: "媒体未通过验证",
  COLLECTION_LIMIT: "达到响应、图片或内存上限",
  COLLECTION_INTEGRITY: "数据校验失败，请查看运行日志",
  COLLECTION_NORMALIZATION_FAILED: "来源数据结构尚未识别，原始响应已归档供检查",
  COLLECTION_RESPONSE_INVALID: "来源响应格式发生变化，原始 JSON 已归档供检查",
  COLLECTION_IO: "读写失败，请检查磁盘或路径",
  shared_resource_wait: "等待共享暂存、内存或磁盘空间",
  remote_retry: "等待远端冷却结束",
};
export const collectionReason = (value: string | null | undefined) =>
  value ? (reasons[value] ?? value) : "";
export const collectionRange = (d: CollectionDefinition) =>
  `${d.seeds.kind === "authors" ? "作者" : "作品"} ${d.seeds.ids.slice(0, 3).join("、")}${d.seeds.ids.length > 3 ? ` 等 ${d.seeds.ids.length} 个` : ""} · ${d.discovery.max_depth ? `${d.discovery.max_depth} 跳扩展` : "指定范围"}`;
export const collectionActive = (j: CollectionJob) =>
  j.execution_active ||
  [
    "queued",
    "running",
    "pausing",
    "cancelling",
    "publishing",
    "waiting_retry",
    "waiting_resources",
  ].includes(j.state);
export function availableCollectionActions(
  j: CollectionJob,
): CollectionAction[] {
  if (j.execution_active)
    return ["pausing", "cancelling"].includes(j.state)
      ? []
      : ["pause", "cancel"];
  if (["completed", "cancelled"].includes(j.state)) return [];
  if (j.state === "completed_with_gaps") return ["retry_failed", "cancel"];
  if (["queued", "running"].includes(j.state)) return ["pause", "cancel"];
  return ["resume", "retry_failed", "replay_publication", "cancel"];
}
export async function collectionAction(
  client: StudioClient,
  id: string,
  action: CollectionAction,
  taskIds?: string[],
) {
  for (let attempt = 0; ; attempt++) {
    const current = await client.sourceCollections.job(id);
    try {
      return await client.sourceCollections.action(id, {
        action,
        expected_revision: current.revision,
        request_key: crypto.randomUUID(),
        ...(taskIds?.length ? { task_ids: taskIds } : {}),
      });
    } catch (error) {
      if (
        !(error instanceof StudioError) ||
        error.code !== "REVISION_CONFLICT" ||
        attempt >= 2
      )
        throw error;
    }
  }
}
export async function publicAccount(
  client: StudioClient,
  accounts: Schema["CollectionAccount"][],
) {
  const existing = accounts.find(
    (a) => a.mode === "anonymous" && a.state === "valid",
  );
  if (existing) return existing;
  return client.sourceCollections.saveAccount({
    account_id: crypto.randomUUID(),
    request_key: crypto.randomUUID(),
    expected_revision: null,
    mode: "anonymous",
    label: "公开访问",
  });
}
export type CollectionDraft = ImageFields & {
  lakeId: string;
  accountId: string;
  kind: "authors" | "works";
  seeds: string;
  workTypes: string[];
  ratings: string[];
  includeAi: boolean;
  includeUnknown: boolean;
  entrypoints: string[];
  depth: number;
  recommendationSeeds: number;
  retainOriginal: boolean;
  ugoira: string;
  refreshMode: string;
  refreshHours: number;
  reuseMode: string;
  reuseHours: number;
  apiRequests: number;
  authors: number;
  downloadGiB: number;
  wallHours: number;
  periodic: boolean;
  intervalHours: number;
  submission: {
    key: string;
    scheduleId: string;
    spec: CollectionDefinition;
    periodic: boolean;
    everySeconds: number;
    firstRunAt: string;
    id?: string;
  } | null;
};
export const initialCollectionDraft: CollectionDraft = {
  lakeId: "",
  accountId: "",
  kind: "authors",
  seeds: "",
  workTypes: ["illustration", "manga", "ugoira"],
  ratings: ["all_ages"],
  includeAi: true,
  includeUnknown: true,
  entrypoints: [],
  depth: 0,
  recommendationSeeds: 2,
  profile: "original",
  encoding: defaultEncoding,
  existing: "match_profile",
  allowSample: false,
  retainOriginal: true,
  ugoira: "archive_with_poster",
  refreshMode: "missing_or_stale",
  refreshHours: 168,
  reuseMode: "historical_if_same_locator",
  reuseHours: 168,
  apiRequests: 5000,
  authors: 50,
  downloadGiB: 20,
  wallHours: 6,
  periodic: false,
  intervalHours: 24,
  submission: null,
};
export function decodeCollectionDraft(value: unknown): CollectionDraft | null {
  if (!value || typeof value !== "object") return null;
  const v = value as CollectionDraft;
  if (
    !["authors", "works"].includes(v.kind) ||
    typeof v.seeds !== "string" ||
    typeof v.lakeId !== "string" ||
    typeof v.accountId !== "string"
  )
    return null;
  if (
    ![v.workTypes, v.ratings, v.entrypoints].every(
      (a) => Array.isArray(a) && a.every((i) => typeof i === "string"),
    )
  )
    return null;
  return { ...initialCollectionDraft, ...v };
}
export function draftForCollection(
  spec: CollectionDefinition,
): CollectionDraft {
  return {
    ...initialCollectionDraft,
    ...fieldsForPolicy(spec.media.image_policy),
    lakeId: spec.library_id,
    accountId: spec.account_id,
    kind: spec.seeds.kind,
    seeds: spec.seeds.ids.join("\n"),
    workTypes: spec.scope.work_types,
    ratings: spec.scope.ratings,
    includeAi: spec.scope.include_ai,
    includeUnknown: spec.scope.include_unknown_markers,
    entrypoints: spec.discovery.entrypoints,
    depth: spec.discovery.max_depth,
    recommendationSeeds: spec.discovery.recommendation_seeds_per_author || 2,
    retainOriginal: spec.media.retain_original,
    ugoira: spec.media.ugoira,
    refreshMode: spec.refresh?.mode ?? "all",
    refreshHours: spec.refresh?.max_age_hours ?? 168,
    reuseMode: spec.media.reuse.mode,
    reuseHours: spec.media.reuse.max_age_hours || 168,
    apiRequests: spec.run_budget.api_requests,
    authors: spec.run_budget.admitted_authors,
    downloadGiB: spec.run_budget.download_bytes / 1024 ** 3,
    wallHours: spec.run_budget.wall_seconds / 3600,
  };
}
export function collectionDefinition(
  d: CollectionDraft,
  lakeId: string,
  accountId: string,
): CollectionDefinition {
  const ids = [
    ...new Set(
      d.seeds
        .split(/[\s,，]+/)
        .filter(Boolean)
        .map((v) => {
          const match = v.match(
            d.kind === "authors"
              ? /^(?:https:\/\/www\.pixiv\.net\/(?:[a-z]{2}\/)?users\/)?([1-9][0-9]{0,19})(?:\/?(?:[?#].*)?)?$/
              : /^(?:https:\/\/www\.pixiv\.net\/(?:[a-z]{2}\/)?artworks\/)?([1-9][0-9]{0,19})(?:\/?(?:[?#].*)?)?$/,
          );
          if (!match)
            throw new Error(
              "请输入对应的作者或作品 ID，也可粘贴 Pixiv 链接；用换行或逗号分隔。",
            );
          return match[1]!;
        }),
    ),
  ];
  if (!ids.length || ids.length > 1000)
    throw new Error("每次填写 1–1000 个作者或作品。");
  const policy = imagePolicy({
    ...d,
    existing: "match_profile",
    allowSample: false,
  });
  const metadata = policy.profile === "metadata_only";
  const depth = d.kind === "authors" ? d.depth : 0;
  return {
    version: 1,
    collector: "pixiv_web_v1",
    library_id: lakeId,
    account_id: accountId,
    seeds: { kind: d.kind, ids },
    scope: {
      work_types: d.workTypes,
      ratings: d.ratings,
      include_ai: d.includeAi,
      include_unknown_markers: d.includeUnknown,
    },
    discovery: {
      entrypoints: depth ? d.entrypoints : [],
      max_depth: depth,
      recommendation_seeds_per_author:
        depth && d.entrypoints.includes("recommendations")
          ? d.recommendationSeeds
          : 0,
    },
    media: {
      image_policy: policy,
      retain_original:
        !metadata && (policy.profile === "original" || d.retainOriginal),
      ugoira: metadata ? "metadata_only" : d.ugoira,
      reuse: {
        mode: d.reuseMode,
        max_age_hours: d.reuseMode === "revalidate" ? 0 : d.reuseHours,
      },
    },
    refresh: { mode: d.refreshMode, max_age_hours: d.refreshHours },
    run_budget: {
      api_requests: d.apiRequests,
      admitted_authors: d.authors,
      download_bytes: Math.round(d.downloadGiB * 1024 ** 3),
      wall_seconds: Math.round(d.wallHours * 3600),
    },
  };
}
