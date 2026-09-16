import type { Schema } from "@studio/contracts";
export type Request = <T>(path: string, init?: RequestInit) => Promise<T>;
const provider = (id: string) => "/v1/llm/providers/" + encodeURIComponent(id);
const json = (body: unknown): RequestInit => ({
  method: "POST",
  body: JSON.stringify(body),
});

export class LlmProvidersClient {
  constructor(private readonly request: Request) {}
  list(signal?: AbortSignal) {
    return this.request<Schema["LlmProviders"]>("/v1/llm/providers", {
      signal: signal ?? null,
    });
  }
  save(value: Schema["SaveLlmProvider"]) {
    return this.request<Schema["LlmProviderView"]>(
      "/v1/llm/providers",
      json(value),
    );
  }
  remove(id: string, expectedRevision: number) {
    return this.request<Schema["OkResponse"]>(
      provider(id) + "/remove",
      json({ expected_revision: expectedRevision }),
    );
  }
}
export class LlmModelsClient {
  constructor(private readonly request: Request) {}
  list(providerId: string, signal?: AbortSignal) {
    return this.request<Schema["LlmModels"]>(provider(providerId) + "/models", {
      signal: signal ?? null,
    });
  }
  catalog(providerId: string, signal?: AbortSignal) {
    return this.request<Schema["LlmCatalogStatus"]>(
      provider(providerId) + "/catalog",
      { signal: signal ?? null },
    );
  }
  save(value: Schema["SaveLlmModel"]) {
    return this.request<Schema["LlmModel"]>("/v1/llm/models", json(value));
  }
  remove(id: string, expectedRevision: number) {
    return this.request<Schema["OkResponse"]>(
      "/v1/llm/models/" + encodeURIComponent(id) + "/remove",
      json({ expected_revision: expectedRevision }),
    );
  }
  parameters(id: string, signal?: AbortSignal) {
    return this.request<Schema["LlmParameters"]>(
      "/v1/llm/models/" + encodeURIComponent(id) + "/parameters",
      { signal: signal ?? null },
    );
  }
}
export class LlmPresetsClient {
  constructor(private readonly request: Request) {}
  list(signal?: AbortSignal) {
    return this.request<Schema["LlmPresets"]>("/v1/llm/presets", {
      signal: signal ?? null,
    });
  }
  save(value: Schema["SaveLlmPreset"]) {
    return this.request<Schema["LlmPreset"]>("/v1/llm/presets", json(value));
  }
  remove(id: string, expectedRevision: number) {
    return this.request<Schema["OkResponse"]>(
      "/v1/llm/presets/" + encodeURIComponent(id) + "/remove",
      json({ expected_revision: expectedRevision }),
    );
  }
}
