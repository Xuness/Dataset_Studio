import type { Schema } from "@studio/contracts";
type Request = <T>(path: string, init?: RequestInit) => Promise<T>;
type Page = {
  cursor?: string | undefined;
  limit?: number | undefined;
  signal?: AbortSignal | undefined;
};
const base = "/v1/pinterest-collections";
const jobPath = (id: string) => `${base}/jobs/${encodeURIComponent(id)}`;
function query(values: Record<string, string | number | undefined>) {
  const result = new URLSearchParams();
  for (const [key, value] of Object.entries(values))
    if (value !== undefined) result.set(key, String(value));
  return result.toString();
}
export class PinterestCollectionClient {
  constructor(private readonly request: Request) {}
  status(signal?: AbortSignal) {
    return this.request<Schema["PinterestStatus"]>(`${base}/status`, {
      signal: signal ?? null,
    });
  }
  capabilities(signal?: AbortSignal) {
    return this.request<Schema["PinterestCapabilities"]>(
      `${base}/capabilities`,
      { signal: signal ?? null },
    );
  }
  lakes(options: Page = {}) {
    const { signal, ...values } = options;
    return this.request<Schema["CollectionLakes"]>(
      `${base}/lakes?${query(values)}`,
      { signal: signal ?? null },
    );
  }
  createLake(value: Schema["CreateCollectionLake"]) {
    return this.request<Schema["CollectionLake"]>(`${base}/lakes`, {
      method: "POST",
      body: JSON.stringify(value),
    });
  }
  registerLake(value: Schema["CreateCollectionLake"]) {
    return this.request<Schema["CollectionLake"]>(`${base}/lakes/register`, {
      method: "POST",
      body: JSON.stringify(value),
    });
  }
  preview(definition: Schema["PinterestDefinition"]) {
    return this.request<Schema["PinterestPreview"]>(`${base}/jobs/preview`, {
      method: "POST",
      body: JSON.stringify(definition),
    });
  }
  create(
    definition: Schema["PinterestDefinition"],
    requestKey: string = crypto.randomUUID(),
  ) {
    return this.request<Schema["PinterestJob"]>(`${base}/jobs`, {
      method: "POST",
      body: JSON.stringify({ request_key: requestKey, definition }),
    });
  }
  jobs(
    options: Page & {
      library_id?: string | undefined;
      state?: string | undefined;
    } = {},
  ) {
    const { signal, ...values } = options;
    return this.request<Schema["PinterestJobs"]>(
      `${base}/jobs?${query(values)}`,
      { signal: signal ?? null },
    );
  }
  job(id: string, signal?: AbortSignal) {
    return this.request<Schema["PinterestJob"]>(jobPath(id), {
      signal: signal ?? null,
    });
  }
  items(id: string, options: Page & { state?: string | undefined } = {}) {
    const { signal, ...values } = options;
    return this.request<Schema["PinterestItems"]>(
      `${jobPath(id)}/items?${query(values)}`,
      { signal: signal ?? null },
    );
  }
  action(id: string, value: Schema["PinterestJobAction"]) {
    return this.request<Schema["PinterestJob"]>(`${jobPath(id)}/actions`, {
      method: "POST",
      body: JSON.stringify(value),
    });
  }
  streams(id: string, options: Page & { state?: string | undefined } = {}) {
    const { signal, ...values } = options;
    return this.request<Schema["PinterestStreams"]>(
      `${jobPath(id)}/streams?${query(values)}`,
      { signal: signal ?? null },
    );
  }
  schedules(options: Page & { library_id?: string | undefined } = {}) {
    const { signal, ...values } = options;
    return this.request<Schema["PinterestSchedules"]>(
      `${base}/schedules?${query(values)}`,
      { signal: signal ?? null },
    );
  }
  saveSchedule(value: Schema["SavePinterestSchedule"]) {
    return this.request<Schema["PinterestSchedule"]>(
      `${base}/schedules/${encodeURIComponent(value.id)}`,
      { method: "PUT", body: JSON.stringify(value) },
    );
  }
  removeSchedule(id: string, value: Schema["CollectionRevisionCommand"]) {
    return this.request<Schema["CollectionScheduleRemoved"]>(
      `${base}/schedules/${encodeURIComponent(id)}/remove`,
      { method: "POST", body: JSON.stringify(value) },
    );
  }
}
