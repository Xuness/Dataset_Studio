import type { Asset } from "@studio/contracts";

export function assetTitle(asset: Asset) {
  const id = asset.summary?.post_ids[0];
  if (id)
    return (
      (asset.summary?.site_name ?? asset.source_name ?? "帖子") + " #" + id
    );
  if (asset.summary)
    return (
      (asset.summary.status === "unlinked" ? "未关联帖子 · " : "图像 · ") +
      asset.key.asset_id.slice(0, 12)
    );
  return /^[a-f0-9]{64}$/.test(asset.name)
    ? "图像 · " + asset.name.slice(0, 12)
    : asset.name;
}

export function assetSummaryNote(asset: Asset) {
  if (asset.summary?.status === "unavailable") return "帖子 ID 暂不可用";
  const count = Number(asset.summary?.post_count ?? 0);
  return count > 1 ? count.toLocaleString() + " 个关联帖子" : "";
}
