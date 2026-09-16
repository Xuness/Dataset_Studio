import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button, ErrorDetails, Field } from "@studio/ui";
import type { StudioClient } from "@studio/client";
import type { Schema } from "@studio/contracts";
import { ParameterEditor } from "./ParameterEditor.js";
import { fromFields, toFields, protocols, type ModelDraft } from "./types.js";

export function ModelEditor({
  value,
  setValue,
  provider,
  client,
  busy,
  action,
  close,
  saved,
}: {
  value: ModelDraft;
  setValue: (v: ModelDraft) => void;
  provider: Schema["LlmProviderView"];
  client: StudioClient;
  busy: boolean;
  action: (run: () => Promise<unknown>, notice: string) => Promise<void>;
  close: () => void;
  saved: () => void;
}) {
  const [presetId, setPresetId] = useState(""),
    [presetName, setPresetName] = useState("");
  const specs = useQuery({
    queryKey: [
      "settings",
      "llm",
      "parameters",
      client.connection.instance_id,
      provider.id,
      provider.revision,
      value.config.protocol,
    ],
    queryFn: ({ signal }) =>
      client.llm.parameters(
        value.config.protocol,
        provider.config.kind,
        signal,
      ),
  });
  const presets = useQuery({
    queryKey: ["settings", "llm", "presets", client.connection.instance_id],
    queryFn: ({ signal }) => client.llm.presets.list(signal),
  });
  const candidates =
    presets.data?.items.filter(
      (p) => p.config.protocol === value.config.protocol,
    ) ?? [];
  const selected = candidates.find((p) => p.id === presetId);
  const allowed: Schema["LlmProtocol"][] =
    provider.config.kind === "gemini"
      ? ["gemini"]
      : provider.config.kind === "openrouter"
        ? ["openai_chat"]
        : ["openai_chat", "openai_responses"];
  const set = (patch: Partial<Schema["LlmModelConfig"]>) =>
    setValue({ ...value, config: { ...value.config, ...patch } });
  return (
    <form
      className="llm-editor"
      onSubmit={(e) => {
        e.preventDefault();
        void action(async () => {
          const capabilities: unknown = JSON.parse(value.capabilities);
          if (
            !capabilities ||
            typeof capabilities !== "object" ||
            Array.isArray(capabilities) ||
            Object.values(capabilities).some(
              (v) => !["supported", "unsupported", "unknown"].includes(v),
            )
          )
            throw new Error(
              "能力覆盖须为能力名称到 supported、unsupported 或 unknown 的 JSON 对象",
            );
          await client.llm.models.save({
            id: value.id,
            provider_id: value.providerId,
            expected_revision: value.revision,
            config: {
              ...value.config,
              parameters: fromFields(value.parameters, specs.data?.items ?? []),
              capability_overrides: capabilities as NonNullable<
                Schema["LlmModelConfig"]["capability_overrides"]
              >,
            },
          });
          saved();
        }, "模型配置已保存。");
      }}
    >
      <h4>{value.id ? "编辑模型配置" : "添加模型配置"}</h4>
      {specs.error && <ErrorDetails error={specs.error} />}
      <fieldset disabled={busy || !specs.data}>
        <div className="llm-form-grid">
          <Field label="显示名称">
            <input
              aria-label="模型显示名称"
              required
              value={value.config.name}
              onChange={(e) => set({ name: e.target.value })}
            />
          </Field>
          <Field label="调用协议">
            <select
              aria-label="调用协议"
              value={value.config.protocol}
              onChange={(e) =>
                set({ protocol: e.target.value as Schema["LlmProtocol"] })
              }
            >
              {allowed.map((p) => (
                <option key={p} value={p}>
                  {protocols[p]}
                </option>
              ))}
            </select>
          </Field>
        </div>
        <Field label="远端模型 ID">
          <input
            aria-label="远端模型 ID"
            required
            value={value.config.remote_model_id}
            onChange={(e) => set({ remote_model_id: e.target.value })}
          />
        </Field>
        <label className="llm-check">
          <input
            type="checkbox"
            checked={value.config.enabled}
            onChange={(e) => set({ enabled: e.target.checked })}
          />
          启用此模型
        </label>
        <ParameterEditor
          specs={specs.data?.items ?? []}
          value={value.parameters}
          onChange={(parameters) => setValue({ ...value, parameters })}
          disabled={busy}
        />
        <details>
          <summary>模型能力覆盖</summary>
          <Field label="能力覆盖（JSON）">
            <textarea
              rows={4}
              value={value.capabilities}
              onChange={(e) =>
                setValue({ ...value, capabilities: e.target.value })
              }
            />
          </Field>
          <p className="settings-note">
            可声明参数名、input_image 或 tools 的支持状态。示例：
            {`{"temperature":"unsupported","input_image":"supported"}`}
            。空对象使用目录信息。
          </p>
        </details>
        <details>
          <summary>命名参数预设</summary>
          {presets.error && <ErrorDetails error={presets.error} />}
          <div className="llm-form-grid">
            <Field label="已有预设">
              <select
                aria-label="已有参数预设"
                value={selected?.id ?? ""}
                onChange={(e) => {
                  setPresetId(e.target.value);
                  setPresetName(
                    candidates.find((p) => p.id === e.target.value)?.config
                      .name ?? "",
                  );
                }}
              >
                <option value="">新建预设</option>
                {candidates.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.config.name}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="预设名称">
              <input
                aria-label="参数预设名称"
                value={presetName}
                onChange={(e) => setPresetName(e.target.value)}
              />
            </Field>
          </div>
          <div className="settings-toolbar">
            <Button
              type="button"
              disabled={!selected}
              onClick={() => {
                if (selected)
                  setValue({
                    ...value,
                    parameters: {
                      ...value.parameters,
                      ...toFields(selected.config.parameters),
                    },
                  });
              }}
            >
              应用到编辑中的参数
            </Button>
            <Button
              type="button"
              disabled={!presetName.trim()}
              onClick={() =>
                void action(async () => {
                  const p = await client.llm.presets.save({
                    id: selected?.id ?? null,
                    expected_revision: selected?.revision ?? 0,
                    config: {
                      name: presetName,
                      protocol: value.config.protocol,
                      parameters: fromFields(
                        value.parameters,
                        specs.data?.items ?? [],
                      ),
                    },
                  });
                  setPresetId(p.id);
                }, "参数预设已保存。")
              }
            >
              {selected ? "更新参数预设" : "保存参数预设"}
            </Button>
            <Button
              type="button"
              disabled={!selected}
              onClick={() =>
                void action(async () => {
                  if (selected)
                    await client.llm.presets.remove(
                      selected.id,
                      selected.revision,
                    );
                  setPresetId("");
                  setPresetName("");
                }, "参数预设已删除。")
              }
            >
              删除参数预设
            </Button>
          </div>
        </details>
        <div className="settings-page-actions">
          <Button type="button" onClick={close}>
            取消编辑模型
          </Button>
          <Button type="submit">保存模型配置</Button>
        </div>
      </fieldset>
    </form>
  );
}
