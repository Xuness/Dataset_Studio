import { parseTagInput, tagList } from "./tagInput.js";
export { tagList } from "./tagInput.js";
import type { QueryCondition } from "@studio/contracts";

export const ratingChoices = [
  { value: "g", label: "G · 一般", title: "General" },
  {
    value: "s",
    label: "S · 敏感",
    title: "Sensitive；历史快照中的 s 可能采用旧分级含义，按来源原值匹配",
  },
  { value: "q", label: "Q · 可疑", title: "Questionable" },
  { value: "e", label: "E · 露骨", title: "Explicit" },
] as const;

export function ratingLabel(value: string) {
  return ratingChoices.find((r) => r.value === value)?.label ?? value;
}

export function RatingPicker({
  values,
  onChange,
  label = "分级",
  allowAny = true,
}: {
  values: string[];
  onChange: (values: string[]) => void;
  label?: string;
  allowAny?: boolean;
}) {
  return (
    <div className="rating-picker" role="group" aria-label={label}>
      {allowAny && (
        <button
          type="button"
          aria-pressed={!values.length}
          onClick={() => onChange([])}
        >
          不限
        </button>
      )}
      {ratingChoices.map((rating) => (
        <button
          type="button"
          key={rating.value}
          title={rating.title}
          aria-label={label + " " + rating.value.toUpperCase()}
          aria-pressed={values.includes(rating.value)}
          onClick={() =>
            onChange(
              values.includes(rating.value)
                ? values.filter((v) => v !== rating.value)
                : [...values, rating.value],
            )
          }
        >
          {rating.label}
        </button>
      ))}
      {values
        .filter((v) => !ratingChoices.some((r) => r.value === v))
        .map((v) => (
          <button
            type="button"
            key={v}
            aria-pressed="true"
            onClick={() => onChange(values.filter((other) => other !== v))}
          >
            {v} ×
          </button>
        ))}
    </div>
  );
}

export type BrowseFilters = {
  ratings: string[];
  include: string;
  exclude: string;
  tagMode: "all" | "any";
};
export const emptyFilters: BrowseFilters = {
  ratings: [],
  include: "",
  exclude: "",
  tagMode: "all",
};
export function filterConditions(filters: BrowseFilters): QueryCondition[] {
  const clauses: QueryCondition[] = [];
  const add = (
    field: string,
    operator: QueryCondition["operator"],
    values: string[],
  ) => {
    if (values.length)
      clauses.push({
        field,
        operator,
        value: { type: "text_list", value: [...new Set(values)].sort() },
      });
  };
  add("rating", "in", filters.ratings);
  add(
    "tags",
    filters.tagMode === "all" ? "has_all_tags" : "has_any_tags",
    tagList(filters.include),
  );
  add("tags", "has_no_tags", tagList(filters.exclude));
  return clauses;
}
export function filterError(filters: BrowseFilters) {
  for (const text of [filters.include, filters.exclude]) {
    const parsed = parseTagInput(text);
    if (parsed.error) return parsed.error;
  }
  return "";
}
export function FiltersEditor({
  value,
  onChange,
}: {
  value: BrowseFilters;
  onChange: (value: BrowseFilters) => void;
}) {
  return (
    <div className="filters-editor">
      <div className="filter-ratings">
        <span className="filter-label">分级 · 可多选</span>
        <RatingPicker
          values={value.ratings}
          onChange={(ratings) => onChange({ ...value, ratings })}
        />
      </div>
      <div className="filter-tags">
        <label>
          <span>包含标签</span>
          <textarea
            rows={1}
            aria-label="包含标签"
            placeholder="普通空格分隔；特殊标签可用双引号和 JSON 转义"
            maxLength={16384}
            value={value.include}
            onChange={(e) => onChange({ ...value, include: e.target.value })}
          />
        </label>
        <select
          aria-label="包含标签的匹配方式"
          value={value.tagMode}
          onChange={(e) =>
            onChange({ ...value, tagMode: e.target.value as "all" | "any" })
          }
        >
          <option value="all">全部满足（AND）</option>
          <option value="any">任意满足（OR）</option>
        </select>
        <label>
          <span title="包含任一排除标签的图片都会被剔除；未记录标签按未知处理。">
            排除标签
          </span>
          <textarea
            rows={1}
            aria-label="排除标签"
            placeholder="例如 comic monochrome"
            maxLength={16384}
            value={value.exclude}
            onChange={(e) => onChange({ ...value, exclude: e.target.value })}
          />
        </label>
      </div>
    </div>
  );
}
