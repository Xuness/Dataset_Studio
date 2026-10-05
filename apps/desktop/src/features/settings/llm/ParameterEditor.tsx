import type { Schema } from "@studio/contracts";
import type { ParameterFields } from "./types.js";
const groups: Record<string, string> = {
  sampling: "采样",
  generation: "输出",
  reasoning: "推理",
  tools: "工具调用",
  routing: "上游路由",
  caching: "提示词缓存",
  advanced: "高级参数",
};
export function ParameterEditor({
  specs,
  value,
  onChange,
  disabled,
}: {
  specs: Schema["LlmParameterSpec"][];
  value: ParameterFields;
  onChange: (value: ParameterFields) => void;
  disabled: boolean;
}) {
  const update = (key: string, field: ParameterFields[string] | undefined) => {
    const next = { ...value };
    if (field) next[key] = field;
    else delete next[key];
    onChange(next);
  };
  return (
    <div className="llm-parameters">
      <p className="settings-note">
        未设置的参数交给上游默认值。“不发送”可清除继承值。模型能力未知时仍可设置，调用前会返回提示。
      </p>
      {Object.entries(groups).map(([group, label]) => (
        <details
          key={group}
          open={group === "sampling" || group === "generation"}
        >
          <summary>{label}</summary>
          {specs
            .filter((s) => s.group === group)
            .map((spec) => {
              const field = value[spec.key],
                mode = field?.mode ?? "inherit";
              const text = field?.text ?? "";
              return (
                <div className="llm-parameter" key={spec.key}>
                  <label htmlFor={"llm-value-" + spec.key}>
                    {spec.label}
                    <small>
                      {spec.key} ·{" "}
                      {spec.support === "supported"
                        ? "已声明支持"
                        : spec.support === "unsupported"
                          ? "不支持"
                          : "能力未知"}
                    </small>
                  </label>
                  <select
                    aria-label={spec.label + "设置方式"}
                    value={mode}
                    disabled={disabled}
                    onChange={(e) => {
                      if (e.target.value === "inherit")
                        update(spec.key, undefined);
                      else
                        update(spec.key, {
                          mode: e.target.value as "value" | "omit",
                          text:
                            field?.text ||
                            (spec.choices[0] ??
                              {
                                boolean: "false",
                                array: "[]",
                                object: "{}",
                                integer: String(spec.minimum ?? 1),
                                number: String(spec.minimum ?? 1),
                                string: "",
                              }[spec.value_type] ??
                              ""),
                        });
                    }}
                  >
                    <option value="inherit">未设置</option>
                    <option value="omit">不发送</option>
                    <option value="value">设置值</option>
                  </select>
                  {mode === "value" &&
                    (spec.choices.length ? (
                      <select
                        id={"llm-value-" + spec.key}
                        value={text}
                        disabled={disabled}
                        onChange={(e) =>
                          update(spec.key, {
                            mode: "value",
                            text: e.target.value,
                          })
                        }
                      >
                        {spec.choices.map((choice) => (
                          <option key={choice}>{choice}</option>
                        ))}
                      </select>
                    ) : spec.value_type === "boolean" ? (
                      <select
                        id={"llm-value-" + spec.key}
                        value={text}
                        disabled={disabled}
                        onChange={(e) =>
                          update(spec.key, {
                            mode: "value",
                            text: e.target.value,
                          })
                        }
                      >
                        <option value="false">关闭</option>
                        <option value="true">开启</option>
                      </select>
                    ) : ["object", "array"].includes(spec.value_type) ? (
                      <textarea
                        id={"llm-value-" + spec.key}
                        rows={3}
                        value={text}
                        disabled={disabled}
                        onChange={(e) =>
                          update(spec.key, {
                            mode: "value",
                            text: e.target.value,
                          })
                        }
                      />
                    ) : (
                      <input
                        id={"llm-value-" + spec.key}
                        type={
                          ["integer", "number"].includes(spec.value_type)
                            ? "number"
                            : "text"
                        }
                        step={spec.value_type === "integer" ? 1 : "any"}
                        min={spec.minimum ?? undefined}
                        max={spec.maximum ?? undefined}
                        value={text}
                        disabled={disabled}
                        onChange={(e) =>
                          update(spec.key, {
                            mode: "value",
                            text: e.target.value,
                          })
                        }
                      />
                    ))}
                  <p>{spec.description}</p>
                </div>
              );
            })}
        </details>
      ))}
      {Object.keys(value)
        .filter((key) => !specs.some((s) => s.key === key))
        .map((key) => (
          <p className="settings-validation" key={key}>
            当前协议未定义 {key}{" "}
            <button
              type="button"
              disabled={disabled}
              onClick={() => update(key, undefined)}
            >
              移除该参数
            </button>
          </p>
        ))}
    </div>
  );
}
