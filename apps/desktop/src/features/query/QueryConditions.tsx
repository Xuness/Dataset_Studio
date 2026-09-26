import { useEffect, useRef, useState } from "react";
import { Plus, X } from "lucide-react";
import {
  Button,
  RatingPicker,
  ratingChoices,
  tagList,
  validSourceTag,
  formatTagList,
} from "@studio/ui";
import type { FieldDefinition, QueryCondition } from "@studio/contracts";

export const operatorNames: Record<QueryCondition["operator"], string> = {
  eq: "等于",
  ne: "不等于",
  gte: "大于等于",
  lte: "小于等于",
  has_tag: "包含单个标签",
  in: "任一分级（OR）",
  has_all_tags: "包含全部（AND）",
  has_any_tags: "包含任一（OR）",
  has_no_tags: "排除任一",
  is_missing: "未记录",
  is_present: "已记录",
};
const listOperators = new Set<QueryCondition["operator"]>([
  "in",
  "has_all_tags",
  "has_any_tags",
  "has_no_tags",
]);
export function initialCondition(field: FieldDefinition): QueryCondition {
  const operator = field.operators[0] ?? "eq";
  return {
    field: field.id,
    operator,
    value: listOperators.has(operator)
      ? { type: "text_list", value: [] }
      : field.field_type === "integer"
        ? { type: "integer", value: "0" }
        : field.field_type === "boolean"
          ? { type: "boolean", value: false }
          : { type: "text", value: "" },
  };
}
export function conditionIssue(
  condition: QueryCondition,
  field?: FieldDefinition,
) {
  if (!field || !field.operators.includes(condition.operator))
    return "此字段或操作在当前来源不可用。";
  if (["is_missing", "is_present"].includes(condition.operator)) return "";
  const value = condition.value;
  if (listOperators.has(condition.operator)) {
    if (value?.type !== "text_list" || !value.value.length)
      return condition.field === "rating"
        ? "请选择至少一个分级。"
        : "请填写至少一个标签。";
    if (value.value.length > 64) return "每组最多 64 个值。";
    if (
      value.value.some((v) =>
        field.field_type === "tags" ? !validSourceTag(v) : v.length > 256,
      )
    )
      return "标签必须完整，单个不超过 256 个字符。";
  } else if (field.field_type === "integer") {
    if (value?.type !== "integer" || !/^(?:0|-?[1-9]\d*)$/.test(value.value))
      return "请输入整数，不使用小数、指数或多余的前导零。";
    try {
      const n = BigInt(value.value);
      if (n < -(1n << 63n) || n >= 1n << 63n) return "整数超出支持范围。";
    } catch {
      return "请输入有效整数。";
    }
  } else if (
    condition.operator === "has_tag" &&
    (value?.type !== "text" || !validSourceTag(value.value))
  )
    return "此操作只接受一个标签；多个标签请选全部或任一。";
  return "";
}
function TagValues({
  values,
  onChange,
  label,
}: {
  values: string[];
  onChange: (values: string[]) => void;
  label: string;
}) {
  const [text, setText] = useState(formatTagList(values));
  const focused = useRef(false);
  useEffect(() => {
    if (!focused.current) setText(formatTagList(values));
  }, [values]);
  return (
    <textarea
      rows={1}
      aria-label={label}
      value={text}
      maxLength={16384}
      placeholder="普通空格分隔；双引号内可写 \t、\n 等 JSON 转义"
      onFocus={() => {
        focused.current = true;
      }}
      onBlur={() => {
        focused.current = false;
        setText(formatTagList(values));
      }}
      onChange={(e) => {
        setText(e.target.value);
        onChange(tagList(e.target.value));
      }}
    />
  );
}
export function QueryConditions({
  fields,
  conditions,
  onChange,
}: {
  fields: FieldDefinition[];
  conditions: QueryCondition[];
  onChange: (conditions: QueryCondition[]) => void;
}) {
  function update(index: number, condition: QueryCondition) {
    onChange(conditions.map((old, i) => (i === index ? condition : old)));
  }
  return (
    <div className="query-conditions">
      {conditions.map((condition, index) => {
        const field = fields.find((f) => f.id === condition.field);
        const noValue = ["is_missing", "is_present"].includes(
          condition.operator,
        );
        const issue = conditionIssue(condition, field);
        return (
          <div className="query-condition" key={index}>
            <span className="condition-join">{index ? "并且" : "满足"}</span>
            <select
              aria-label={"条件字段 " + (index + 1)}
              value={condition.field}
              onChange={(e) => {
                const next = fields.find((f) => f.id === e.target.value);
                if (next) update(index, initialCondition(next));
              }}
            >
              {!field && (
                <option value={condition.field}>
                  字段已不可用 · {condition.field}
                </option>
              )}
              {fields.map((f) => (
                <option key={f.id} value={f.id}>
                  {f.name}
                  {f.unit === "pixel"
                    ? "（px）"
                    : f.unit === "byte"
                      ? "（字节）"
                      : ""}
                </option>
              ))}
            </select>
            <select
              aria-label={"条件操作 " + (index + 1)}
              value={condition.operator}
              onChange={(e) => {
                const operator = e.target.value as QueryCondition["operator"];
                const old = condition.value;
                const value = ["is_missing", "is_present"].includes(operator)
                  ? null
                  : listOperators.has(operator)
                    ? {
                        type: "text_list" as const,
                        value:
                          old?.type === "text_list"
                            ? old.value
                            : old?.type === "text"
                              ? condition.field === "rating"
                                ? [old.value].filter(Boolean)
                                : tagList(old.value)
                              : [],
                      }
                    : old?.type === "text_list"
                      ? { type: "text" as const, value: old.value[0] ?? "" }
                      : (old ?? (field ? initialCondition(field).value : null));
                update(index, { ...condition, operator, value: value ?? null });
              }}
            >
              {field?.operators.map((op) => (
                <option key={op} value={op}>
                  {operatorNames[op]}
                </option>
              ))}
            </select>
            <div className="condition-value">
              {noValue ? (
                <span className="condition-no-value">无需填写值</span>
              ) : condition.field === "rating" ? (
                condition.operator === "in" ? (
                  <RatingPicker
                    label={"条件分级 " + (index + 1)}
                    allowAny={false}
                    values={
                      condition.value?.type === "text_list"
                        ? condition.value.value
                        : []
                    }
                    onChange={(value) =>
                      update(index, {
                        ...condition,
                        value: { type: "text_list", value },
                      })
                    }
                  />
                ) : (
                  <select
                    aria-label={"条件值 " + (index + 1)}
                    value={
                      condition.value?.type === "text"
                        ? condition.value.value
                        : ""
                    }
                    onChange={(e) =>
                      update(index, {
                        ...condition,
                        value: { type: "text", value: e.target.value },
                      })
                    }
                  >
                    <option value="">空分级</option>
                    {ratingChoices.map((r) => (
                      <option key={r.value} value={r.value}>
                        {r.label}
                      </option>
                    ))}
                    {condition.value?.type === "text" &&
                      condition.value.value &&
                      !ratingChoices.some(
                        (r) => r.value === condition.value?.value,
                      ) && (
                        <option value={condition.value.value}>
                          {condition.value.value}（来源原值）
                        </option>
                      )}
                  </select>
                )
              ) : listOperators.has(condition.operator) ? (
                <TagValues
                  key={condition.field + condition.operator}
                  label={"条件值 " + (index + 1)}
                  values={
                    condition.value?.type === "text_list"
                      ? condition.value.value
                      : []
                  }
                  onChange={(value) =>
                    update(index, {
                      ...condition,
                      value: { type: "text_list", value },
                    })
                  }
                />
              ) : field?.field_type === "tags" ? (
                <TagValues
                  label={"条件值 " + (index + 1)}
                  values={
                    condition.value?.type === "text" && condition.value.value
                      ? [condition.value.value]
                      : []
                  }
                  onChange={(tags) =>
                    update(index, {
                      ...condition,
                      value: {
                        type: "text",
                        value: tags.length === 1 ? tags[0]! : "",
                      },
                    })
                  }
                />
              ) : field?.field_type === "boolean" ? (
                <select
                  aria-label={"条件值 " + (index + 1)}
                  value={condition.value?.value === true ? "true" : "false"}
                  onChange={(e) =>
                    update(index, {
                      ...condition,
                      value: {
                        type: "boolean",
                        value: e.target.value === "true",
                      },
                    })
                  }
                >
                  <option value="false">否</option>
                  <option value="true">是</option>
                </select>
              ) : (
                <input
                  aria-label={"条件值 " + (index + 1)}
                  value={String(condition.value?.value ?? "")}
                  inputMode={
                    field?.field_type === "integer" ? "numeric" : "text"
                  }
                  maxLength={256}
                  placeholder={
                    field?.field_type === "integer" ? "整数" : "允许空文本"
                  }
                  onChange={(e) =>
                    update(index, {
                      ...condition,
                      value: {
                        type:
                          field?.field_type === "integer" ? "integer" : "text",
                        value: e.target.value,
                      },
                    })
                  }
                />
              )}
              {issue && <small className="condition-error">{issue}</small>}
            </div>
            <button
              type="button"
              className="icon-button"
              aria-label={"删除条件 " + (index + 1)}
              onClick={() => onChange(conditions.filter((_, i) => i !== index))}
            >
              <X size={14} />
            </button>
          </div>
        );
      })}
      <Button
        type="button"
        disabled={!fields.length || conditions.length >= 12}
        onClick={() => {
          if (fields[0]) onChange([...conditions, initialCondition(fields[0])]);
        }}
      >
        <Plus size={13} />
        添加条件
      </Button>
      {!conditions.length && (
        <span className="subtle">不设条件将查询所选数据湖的全部存储对象。</span>
      )}
    </div>
  );
}
