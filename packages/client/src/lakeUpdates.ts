import type { Schema } from "@studio/contracts";

type Request = <T>(path: string, init?: RequestInit) => Promise<T>;
const base = "/v1/lake-updates";
const job = (id: string) => `${base}/jobs/${encodeURIComponent(id)}`;

export class LakeUpdateClient {
  constructor(private readonly request: Request) {}

  createInput(value: Schema["CreateLakeUpdateInput"]) {
    return this.request<Schema["LakeUpdateInput"]>(`${base}/inputs`, {
      method: "POST",
      body: JSON.stringify(value),
    });
  }
  input(id: string, signal?: AbortSignal) {
    return this.request<Schema["LakeUpdateInput"]>(
      `${base}/inputs/${encodeURIComponent(id)}`,
      { signal: signal ?? null },
    );
  }
  appendInput(id: string, value: Schema["AppendLakeUpdateInput"]) {
    return this.request<Schema["LakeUpdateInput"]>(
      `${base}/inputs/${encodeURIComponent(id)}/append`,
      { method: "POST", body: JSON.stringify(value) },
    );
  }
  sealInput(id: string) {
    return this.request<Schema["LakeUpdateInput"]>(
      `${base}/inputs/${encodeURIComponent(id)}/seal`,
      { method: "POST" },
    );
  }

  status(signal?: AbortSignal) {
    return this.request<Schema["LakeUpdateServiceStatus"]>(`${base}/status`, {
      signal: signal ?? null,
    });
  }
  configure(runtime: Schema["ConfigureLakeUpdates"]) {
    return this.request<Schema["OkResponse"]>(`${base}/runtime`, {
      method: "PUT",
      body: JSON.stringify(runtime),
    });
  }
  capabilities(signal?: AbortSignal) {
    return this.request<Schema["LakeUpdateCapabilities"]>(
      `${base}/capabilities`,
      { signal: signal ?? null },
    );
  }
  lakes(signal?: AbortSignal) {
    return this.request<Schema["UpdateLakes"]>(`${base}/lakes`, {
      signal: signal ?? null,
    });
  }
  register(target: Schema["RegisterUpdateLake"]) {
    return this.request<Schema["UpdateLake"]>(`${base}/lakes`, {
      method: "POST",
      body: JSON.stringify(target),
    });
  }
  setCredentials(value: Schema["SetLakeCredentials"]) {
    return this.request<Schema["LakeCredentialStatus"]>(`${base}/credentials`, {
      method: "PUT",
      body: JSON.stringify(value),
    });
  }
  clearCredentials(site: Schema["LakeUpdateSite"]) {
    return this.request<Schema["LakeCredentialStatus"]>(
      `${base}/credentials/${site}/clear`,
      { method: "POST" },
    );
  }
  probe(site: Schema["LakeUpdateSite"], signal?: AbortSignal) {
    return this.request<Schema["LakeApiProbe"]>(`${base}/probes/${site}`, {
      method: "POST",
      signal: signal ?? null,
    });
  }
  preview(definition: Schema["LakeUpdateDefinition"]) {
    return this.request<Schema["LakeUpdatePreview"]>(`${base}/preview`, {
      method: "POST",
      body: JSON.stringify(definition),
    });
  }
  create(
    definition: Schema["LakeUpdateDefinition"],
    requestKey = crypto.randomUUID(),
  ) {
    return this.request<Schema["LakeUpdateJob"]>(`${base}/jobs`, {
      method: "POST",
      body: JSON.stringify({ definition, request_key: requestKey }),
    });
  }
  jobs(options: { after?: string; limit?: number; signal?: AbortSignal } = {}) {
    const query = new URLSearchParams({ limit: String(options.limit ?? 50) });
    if (options.after) query.set("after", options.after);
    return this.request<Schema["LakeUpdateJobs"]>(`${base}/jobs?${query}`, {
      signal: options.signal ?? null,
    });
  }
  job(id: string, signal?: AbortSignal) {
    return this.request<Schema["LakeUpdateJob"]>(job(id), {
      signal: signal ?? null,
    });
  }
  action(id: string, action: Schema["LakeUpdateAction"]) {
    return this.request<Schema["LakeUpdateJob"]>(`${job(id)}/actions`, {
      method: "POST",
      body: JSON.stringify({ action }),
    });
  }
  items(
    id: string,
    options: { after?: number; limit?: number; signal?: AbortSignal } = {},
  ) {
    const query = new URLSearchParams({ limit: String(options.limit ?? 100) });
    if (options.after !== undefined) query.set("after", String(options.after));
    return this.request<Schema["LakeUpdateItems"]>(
      `${job(id)}/items?${query}`,
      { signal: options.signal ?? null },
    );
  }
  coverage(id: string, signal?: AbortSignal) {
    return this.request<Schema["LakeUpdateCoverage"]>(`${job(id)}/coverage`, {
      signal: signal ?? null,
    });
  }
  schedules(signal?: AbortSignal) {
    return this.request<Schema["LakeUpdateSchedules"]>(`${base}/schedules`, {
      signal: signal ?? null,
    });
  }
  saveSchedule(value: Schema["SaveLakeUpdateSchedule"]) {
    return this.request<Schema["LakeUpdateScheduleSaved"]>(
      `${base}/schedules`,
      { method: "POST", body: JSON.stringify(value) },
    );
  }
  removeSchedule(id: string, revision: number) {
    return this.request<Schema["OkResponse"]>(
      `${base}/schedules/${encodeURIComponent(id)}/remove`,
      { method: "POST", body: JSON.stringify({ revision }) },
    );
  }
}
