import type { StudioClient } from "@studio/client";
import type { Schema, Source } from "@studio/contracts";
import type { LlmSettingsDraft } from "./llm/types.js";

export type SettingsPageProps = {
  llmDraft: LlmSettingsDraft;
  setLlmDraft: (value: LlmSettingsDraft) => void;
  client: StudioClient;
  project: { id: string; name: string } | null;
  sources: Source[];
  activeResultId: string | null;
  data: Schema["SettingsStatus"];
  busy: boolean;
  action: (run: () => Promise<unknown>, notice: string) => Promise<void>;
  cacheDraft: Schema["CacheSettings"] | null;
  setCacheDraft: (value: Schema["CacheSettings"] | null) => void;
  memoryDraft: string | null;
  setMemoryDraft: (value: string | null) => void;
  undoDraft: string | null;
  setUndoDraft: (value: string | null) => void;
};
export const sizeLabel = (bytes: string | number) => {
  const value = Number(bytes);
  return value >= 1073741824
    ? (value / 1073741824).toFixed(2) + " GiB"
    : (value / 1048576).toFixed(1) + " MiB";
};
