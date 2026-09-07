import { Plus, X } from "lucide-react";
import { Button } from "@studio/ui";
import type { FieldDefinition, QueryCondition } from "@studio/contracts";

export const operatorNames: Record<QueryCondition["operator"], string> = {
  eq: "等于",
  ne: "不等于",
  gte: "大于等于",
  lte: "小于等于",
  has_tag: "包含标签",
  is_missing: "未记录",
  is_present: "已记录",
};
export function initialCondition(field: FieldDefinition): QueryCondition {
  const operator = field.operators[0] ?? "eq";
  return {
    field: field.id,
    operator,
    value:
      field.field_type === "integer"
        ? { type: "integer", value: "0" }
        : field.field_type === "boolean"
          ? { type: "boolean", value: false }
          : { type: "text", value: "" },
  };
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
        const noValue =
          condition.operator === "is_missing" ||
          condition.operator === "is_present";
        return (
          <div className="query-condition" key={index}>
            <span className="condition-join">{index ? "并且" : "满足"}</span>
            <select
              aria-label={"条件字段 " + (index + 1)}
              value={condition.field}
              onChange={(event) => {
                const next = fields.find((f) => f.id === event.target.value);
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
              onChange={(event) => {
                const operator = event.target
                  .value as QueryCondition["operator"];
                update(index, {
                  ...condition,
                  operator,
                  value:
                    operator === "is_missing" || operator === "is_present"
                      ? null
                      : (condition.value ??
                        (field
                          ? (initialCondition(field).value ?? null)
                          : null)),
                });
              }}
            >
              {field?.operators.map((operator) => (
                <option key={operator} value={operator}>
                  {operatorNames[operator]}
                </option>
              ))}
            </select>
            {!noValue &&
              (field?.field_type === "boolean" ? (
                <select
                  aria-label={"条件值 " + (index + 1)}
                  value={condition.value?.value === true ? "true" : "false"}
                  onChange={(event) =>
                    update(index, {
                      ...condition,
                      value: {
                        type: "boolean",
                        value: event.target.value === "true",
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
                    field?.field_type === "tags"
                      ? "一个完整标签"
                      : field?.field_type === "integer"
                        ? "整数"
                        : "允许空文本"
                  }
                  onChange={(event) =>
                    update(index, {
                      ...condition,
                      value: {
                        type:
                          field?.field_type === "integer" ? "integer" : "text",
                        value: event.target.value,
                      },
                    })
                  }
                />
              ))}
            {noValue && <span className="condition-no-value">无需填写值</span>}
            <button
              type="button"
              className="icon-button"
              aria-label={"删除条件 " + (index + 1)}
              onClick={() => onChange(conditions.filter((_, i) => i !== index))}
            >
              <X size={13} />
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
        <Plus size={12} />
        添加条件
      </Button>
      {!conditions.length && (
        <span className="subtle">不设条件将查询所选数据湖的全部存储对象。</span>
      )}
    </div>
  );
}
