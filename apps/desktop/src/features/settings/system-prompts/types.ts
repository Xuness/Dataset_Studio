import type { Schema } from "@studio/contracts";

export type SystemPromptDraft = {
  selectedId: string | null;
  search: string;
  editor: Schema["SaveLlmSystemPrompt"] | null;
};
export const emptySystemPromptDraft: SystemPromptDraft = {
  selectedId: null,
  search: "",
  editor: null,
};
export function promptEditor(
  value?: Schema["LlmSystemPrompt"],
  copy = false,
): Schema["SaveLlmSystemPrompt"] {
  return {
    id: copy ? null : (value?.id ?? null),
    expected_revision: copy ? 0 : (value?.revision ?? 0),
    config: value
      ? {
          ...value.config,
          name: value.config.name + (copy ? "（副本）" : ""),
        }
      : { name: "", description: "", text: "" },
  };
}
