import type { Schema } from "@studio/contracts";

type Request = <T>(path: string, init?: RequestInit) => Promise<T>;
const base = "/v1/visual";

/** Shared visual feature jobs and materials; no dependency on a project being open. */
export class VisualClient {
  constructor(private readonly request: Request) {}
  runtime() {
    return this.request<Schema["VisualRuntimeStatus"]>(`${base}/runtime`);
  }
  configure(value: Schema["VisualRuntimeConfig"]) {
    return this.request<Schema["VisualRuntimeStatus"]>(`${base}/runtime`, {
      method: "PUT",
      body: JSON.stringify(value),
    });
  }
  models() {
    return this.request<Schema["VisualModelDescription"]>(`${base}/models`);
  }
  recipes() {
    return this.request<Schema["VisualRecipes"]>(`${base}/recipes`);
  }
  sources() {
    return this.request<Schema["VisualSources"]>(`${base}/sources`);
  }
  runs(after = 0, limit = 50) {
    return this.request<Schema["VisualRuns"]>(
      `${base}/runs?after=${after}&limit=${limit}`,
    );
  }
  create(value: Schema["VisualRunRequest"]) {
    return this.request<Schema["VisualRun"]>(`${base}/runs`, {
      method: "POST",
      body: JSON.stringify(value),
    });
  }
  run(id: string) {
    return this.request<Schema["VisualRun"]>(
      `${base}/runs/${encodeURIComponent(id)}`,
    );
  }
  action(id: string, action: "pause" | "resume" | "cancel") {
    return this.request<Schema["VisualRun"]>(
      `${base}/runs/${encodeURIComponent(id)}/actions`,
      { method: "POST", body: JSON.stringify({ action }) },
    );
  }
  failures(id: string, after = 0, limit = 50) {
    return this.request<Schema["VisualFailures"]>(
      `${base}/runs/${encodeURIComponent(id)}/failures?after=${after}&limit=${limit}`,
    );
  }
  coverage() {
    return this.request<Schema["VisualCoverageResponse"]>(`${base}/coverage`);
  }
  search(value: Schema["VisualSearchRequest"], signal?: AbortSignal) {
    return this.request<Schema["VisualSearchResult"]>(`${base}/search`, {
      method: "POST",
      body: JSON.stringify(value),
      signal: signal ?? null,
    });
  }
  duplicates(value: Schema["VisualDuplicatesRequest"]) {
    return this.request<Schema["VisualDuplicates"]>(`${base}/duplicates`, {
      method: "POST",
      body: JSON.stringify(value),
    });
  }
  buildIndex(value: Schema["VisualIndexRequest"]) {
    return this.request<Schema["VisualIndexResult"]>(`${base}/indexes`, {
      method: "POST",
      body: JSON.stringify(value),
    });
  }
}
