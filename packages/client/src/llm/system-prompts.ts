import type { Schema } from "@studio/contracts";
import type { Request } from "./configuration.js";

const path = "/v1/llm/system-prompts";

export class LlmSystemPromptsClient {
  constructor(private readonly request: Request) {}
  list(signal?: AbortSignal) {
    return this.request<Schema["LlmSystemPrompts"]>(path, {
      signal: signal ?? null,
    });
  }
  get(id: string, signal?: AbortSignal) {
    return this.request<Schema["LlmSystemPrompt"]>(
      path + "/" + encodeURIComponent(id),
      { signal: signal ?? null },
    );
  }
  save(value: Schema["SaveLlmSystemPrompt"]) {
    return this.request<Schema["LlmSystemPrompt"]>(path, {
      method: "POST",
      body: JSON.stringify(value),
    });
  }
  remove(id: string, expectedRevision: number) {
    return this.request<Schema["OkResponse"]>(
      path + "/" + encodeURIComponent(id) + "/remove",
      {
        method: "POST",
        body: JSON.stringify({ expected_revision: expectedRevision }),
      },
    );
  }
}
