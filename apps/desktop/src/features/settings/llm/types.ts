import type { Schema } from "@studio/contracts";
export type ParameterFields = Record<
  string,
  { mode: "value" | "omit"; text: string }
>;
export type ConnectionDraft = {
  id: string | null;
  revision: number;
  config: Schema["LlmConnectionConfig"];
  apiKey: string;
  clearKey: boolean;
  headers: string;
};
export type ModelDraft = {
  id: string | null;
  revision: number;
  providerId: string;
  config: Schema["LlmModelConfig"];
  parameters: ParameterFields;
  capabilities: string;
};
export type LlmSettingsDraft = {
  providerId: string | null;
  connection: ConnectionDraft | null;
  model: ModelDraft | null;
};
export const emptyLlmDraft: LlmSettingsDraft = {
  providerId: null,
  connection: null,
  model: null,
};
export const protocols: Record<Schema["LlmProtocol"], string> = {
  openai_chat: "Chat Completions",
  openai_responses: "Responses",
  gemini: "Gemini 原生",
};
export const providers: Record<
  Schema["LlmProviderKind"],
  { label: string; url: string }
> = {
  openai_compatible: { label: "OpenAI 兼容", url: "https://api.openai.com/v1" },
  openai: { label: "OpenAI", url: "https://api.openai.com/v1" },
  openrouter: { label: "OpenRouter", url: "https://openrouter.ai/api/v1" },
  gemini: {
    label: "Gemini",
    url: "https://generativelanguage.googleapis.com/v1beta",
  },
};
export function connectionDraft(
  value?: Schema["LlmProviderView"],
): ConnectionDraft {
  return {
    id: value?.id ?? null,
    revision: value?.revision ?? 0,
    apiKey: "",
    clearKey: false,
    config: value?.config ?? {
      name: "新供应商",
      kind: "openai_compatible",
      base_url: providers.openai_compatible.url,
      enabled: true,
      headers: {},
      network: {
        connect_timeout_ms: 15000,
        request_timeout_ms: 180000,
        idle_timeout_ms: 60000,
        max_concurrency: 2,
        min_interval_ms: 0,
        rate_limit_retries: 0,
        proxy_url: null,
      },
    },
    headers: JSON.stringify(value?.config.headers ?? {}, null, 2),
  };
}
export function toFields(
  values: Record<string, unknown> = {},
): ParameterFields {
  return Object.fromEntries(
    Object.entries(values).map(([key, value]) => [
      key,
      {
        mode: value === null ? "omit" : "value",
        text:
          typeof value === "string" ? value : JSON.stringify(value, null, 2),
      },
    ]),
  );
}
export function fromFields(
  fields: ParameterFields,
  specs: Schema["LlmParameterSpec"][],
): Record<string, unknown> {
  const result: Record<string, unknown> = {};
  for (const [key, field] of Object.entries(fields)) {
    const spec = specs.find((s) => s.key === key);
    if (!spec)
      throw new Error("当前协议不支持参数：" + key + "。请移除或切回原协议。");
    if (field.mode === "omit") {
      result[key] = null;
      continue;
    }
    let value: unknown;
    try {
      value =
        spec.value_type === "string" ? field.text : JSON.parse(field.text);
    } catch {
      throw new Error(spec.label + "：请输入有效的 " + spec.value_type + " 值");
    }
    if (spec.value_type === "number" || spec.value_type === "integer") {
      if (
        typeof value !== "number" ||
        !Number.isFinite(value) ||
        (spec.value_type === "integer" && !Number.isSafeInteger(value)) ||
        (spec.minimum !== null &&
          spec.minimum !== undefined &&
          value < spec.minimum) ||
        (spec.maximum !== null &&
          spec.maximum !== undefined &&
          value > spec.maximum)
      )
        throw new Error(spec.label + " 超出允许范围");
    }
    result[key] = value;
  }
  return result;
}
export function modelDraft(
  provider: Schema["LlmProviderView"],
  model?: Schema["LlmModel"],
  remote?: Schema["LlmCatalogModel"],
): ModelDraft {
  return {
    id: model?.id ?? null,
    revision: model?.revision ?? 0,
    providerId: provider.id,
    config: model?.config ?? {
      name: remote?.name || remote?.id || "新模型",
      remote_model_id: remote?.id ?? "",
      protocol: provider.config.kind === "gemini" ? "gemini" : "openai_chat",
      enabled: true,
      parameters: {},
      capability_overrides: {},
    },
    parameters: toFields(model?.config.parameters),
    capabilities: JSON.stringify(
      model?.config.capability_overrides ?? {},
      null,
      2,
    ),
  };
}
