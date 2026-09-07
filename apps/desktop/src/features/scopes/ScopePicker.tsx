import { Field } from "@studio/ui";
import type { ScopeOption } from "./scopes.js";

export function ScopePicker({
  options,
  value,
  onChange,
  label = "输入范围",
}: {
  options: ScopeOption[];
  value: string;
  onChange: (value: string) => void;
  label?: string;
}) {
  return (
    <Field label={label}>
      <select
        value={value}
        onChange={(event) => onChange(event.target.value)}
        required
      >
        {!options.length && <option value="">暂无可用范围</option>}
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
            {option.count === null
              ? " · 数量待计算"
              : " · " + option.count.toLocaleString() + " 项"}
          </option>
        ))}
      </select>
    </Field>
  );
}
