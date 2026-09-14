import type { QuerySpec } from "@studio/contracts";

export function describe(spec: QuerySpec) {
  const names: Record<string, string> = {
    rating: "分级",
    tags: "标签",
    "stored.bytes": "文件大小",
    "stored.extension": "文件格式",
    "post.id": "帖子 ID",
    score: "评分",
  };
  const operators: Record<string, string> = {
    eq: "",
    in: "",
    has_tag: "包含",
    has_all_tags: "包含全部",
    has_any_tags: "包含任一",
    has_no_tags: "排除",
    is_missing: "未记录",
    is_present: "已记录",
    gte: "≥",
    lte: "≤",
    ne: "不等于",
  };
  if (!spec.conditions.length) return "浏览排序 · 全部成员";
  return spec.conditions
    .map((c) => {
      const raw =
        c.value?.type === "text_list"
          ? c.value.value.join(" / ")
          : c.value
            ? String(c.value.value)
            : "";
      return [
        (c.field.startsWith("project.") && c.field.endsWith(".rating")
          ? "评分分级"
          : names[c.field]) ?? c.field,
        operators[c.operator] ?? c.operator,
        c.field === "rating" || c.field.endsWith(".rating")
          ? raw.toUpperCase()
          : raw,
      ]
        .filter(Boolean)
        .join(" ");
    })
    .join(" · ");
}
export const usedAt = (value: string) =>
  Number(value)
    ? new Date(Number(value)).toLocaleString("zh-CN", {
        month: "2-digit",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
      })
    : "尚未使用";
