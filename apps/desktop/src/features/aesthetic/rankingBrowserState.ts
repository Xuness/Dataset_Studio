export const rankingRatings = ["g", "s", "q", "e"];
export const rankingBrowserInitial = {
  snapshotId: "",
  ratings: ["g"],
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
  const { rating, ...v } = value as typeof rankingBrowserInitial & {
    rating?: unknown;
  };
  // Drafts saved before multi-Rating browsing hold a single `rating`.
  const ratings: unknown = v.ratings ?? (rating === undefined ? [] : [rating]);
  if (
    typeof v.snapshotId !== "string" ||
    !Array.isArray(ratings) ||
    !ratings.length ||
    ratings.some((r) => !rankingRatings.includes(r)) ||
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
    ratings: rankingRatings.filter((r) => ratings.includes(r)),
    thumbnailSize: Number.isFinite(v.thumbnailSize)
      ? Math.max(128, Math.min(320, v.thumbnailSize))
      : 208,
    scrollTop: Number.isFinite(v.scrollTop)
      ? Math.max(0, Math.min(100000, v.scrollTop))
      : 0,
  };
}
