export const rankingBrowserInitial = {
  snapshotId: "",
  rating: "g",
  after: "",
  past: [] as string[],
  page: 1,
  pageSize: 48,
  ordinal: null as number | null,
  image: false,
  thumbnailSize: 208,
  scrollTop: 0,
};
export function decodeRankingBrowser(
  value: unknown,
): typeof rankingBrowserInitial | null {
  if (!value || typeof value !== "object") return null;
  const v = value as typeof rankingBrowserInitial;
  if (
    typeof v.snapshotId !== "string" ||
    !["g", "s", "q", "e"].includes(v.rating) ||
    typeof v.after !== "string" ||
    v.after.length > 16384 ||
    !Array.isArray(v.past) ||
    v.past.length > 64 ||
    v.past.some((x) => typeof x !== "string" || x.length > 16384) ||
    !Number.isSafeInteger(v.page) ||
    v.page < 1 ||
    ![12, 48, 96].includes(v.pageSize) ||
    (v.ordinal !== null &&
      (!Number.isSafeInteger(v.ordinal) || v.ordinal < 0)) ||
    typeof v.image !== "boolean"
  )
    return null;
  return {
    ...v,
    thumbnailSize: Number.isFinite(v.thumbnailSize)
      ? Math.max(128, Math.min(320, v.thumbnailSize))
      : 208,
    scrollTop: Number.isFinite(v.scrollTop)
      ? Math.max(0, Math.min(100000, v.scrollTop))
      : 0,
  };
}
