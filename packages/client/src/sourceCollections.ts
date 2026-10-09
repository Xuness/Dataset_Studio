import type { Schema } from "@studio/contracts";
type Request = <T>(path: string, init?: RequestInit) => Promise<T>;
const base = "/v1/source-collections";
const job = (id: string) => `${base}/jobs/${encodeURIComponent(id)}`;
type PageOptions = {
  cursor?: string | undefined;
  limit?: number | undefined;
  signal?: AbortSignal | undefined;
};
function query(values: Record<string, string | number | boolean | undefined>) {
  const result = new URLSearchParams();
  for (const [key, value] of Object.entries(values))
    if (value !== undefined) result.set(key, String(value));
  return result.toString();
}
export class SourceCollectionClient {
  constructor(private readonly request: Request) {}
  status(signal?: AbortSignal) {
    return this.request<Schema["CollectionServiceStatus"]>(`${base}/status`, {
      signal: signal ?? null,
    });
  }
  capabilities(signal?: AbortSignal) {
    return this.request<Schema["CollectionCapabilities"]>(
      `${base}/capabilities`,
      { signal: signal ?? null },
    );
  }
  lakes(options: PageOptions = {}) {
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
  accounts(options: PageOptions = {}) {
    const { signal, ...values } = options;
    return this.request<Schema["CollectionAccounts"]>(
      `${base}/accounts?${query(values)}`,
      { signal: signal ?? null },
    );
  }
  saveAccount(value: Schema["SaveCollectionAccount"]) {
    return this.request<Schema["CollectionAccount"]>(
      `${base}/accounts/${encodeURIComponent(value.account_id)}`,
      { method: "PUT", body: JSON.stringify(value) },
    );
  }
  probeAccount(id: string, value: Schema["CollectionRevisionCommand"]) {
    return this.request<Schema["CollectionAccountProbe"]>(
      `${base}/accounts/${encodeURIComponent(id)}/probe`,
      { method: "POST", body: JSON.stringify(value) },
    );
  }
  authenticateAccount(value: Schema["SaveCollectionAccount"]) {
    return this.request<Schema["CollectionAccountProbe"]>(
      `${base}/accounts/${encodeURIComponent(value.account_id)}/authenticate`,
      { method: "POST", body: JSON.stringify(value) },
    );
  }
  clearAccount(id: string, value: Schema["CollectionRevisionCommand"]) {
    return this.request<Schema["CollectionAccount"]>(
      `${base}/accounts/${encodeURIComponent(id)}/clear`,
      { method: "POST", body: JSON.stringify(value) },
    );
  }
  preview(definition: Schema["CollectionJobDefinition"]) {
    return this.request<Schema["CollectionPreview"]>(`${base}/jobs/preview`, {
      method: "POST",
      body: JSON.stringify(definition),
    });
  }
  create(
    definition: Schema["CollectionJobDefinition"],
    requestKey: string = crypto.randomUUID(),
  ) {
    return this.request<Schema["CollectionJobResult"]>(`${base}/jobs`, {
      method: "POST",
      body: JSON.stringify({ request_key: requestKey, definition }),
    });
  }
  jobs(
    options: PageOptions & {
      library_id?: string | undefined;
      state?: string | undefined;
    } = {},
  ) {
    const { signal, ...values } = options;
    return this.request<Schema["CollectionJobs"]>(
      `${base}/jobs?${query(values)}`,
      { signal: signal ?? null },
    );
  }
  job(id: string, signal?: AbortSignal) {
    return this.request<Schema["CollectionJob"]>(job(id), {
      signal: signal ?? null,
    });
  }
  tasks(
    id: string,
    options: PageOptions & {
      kind?: string | undefined;
      state?: string | undefined;
      reason?: string | undefined;
    } = {},
  ) {
    const { signal, ...values } = options;
    return this.request<Schema["CollectionTasks"]>(
      `${job(id)}/tasks?${query(values)}`,
      { signal: signal ?? null },
    );
  }
  coverage(id: string, signal?: AbortSignal) {
    return this.request<Schema["CollectionCoverage"]>(`${job(id)}/coverage`, {
      signal: signal ?? null,
    });
  }
  action(id: string, value: Schema["CollectionJobAction"]) {
    return this.request<Schema["CollectionJobResult"]>(`${job(id)}/actions`, {
      method: "POST",
      body: JSON.stringify(value),
    });
  }
  pipeline(signal?: AbortSignal) {
    return this.request<Schema["CollectionPipelineSettings"]>(
      `${base}/pipeline`,
      { signal: signal ?? null },
    );
  }
  savePipeline(value: Schema["SaveCollectionPipeline"]) {
    return this.request<Schema["CollectionPipelineSettings"]>(
      `${base}/pipeline`,
      { method: "PUT", body: JSON.stringify(value) },
    );
  }
  workspaceLakes(options: PageOptions & { include_pinterest?: boolean } = {}) {
    const { signal, ...values } = options;
    return this.request<Schema["LakeWorkspaceLakes"]>(
      `${base}/workspace/lakes?${query(values)}`,
      { signal: signal ?? null },
    );
  }
  workspaceJobs(
    options: PageOptions & {
      library_id?: string | undefined;
      state?: string | undefined;
      include_pinterest?: boolean;
    } = {},
  ) {
    const { signal, ...values } = options;
    return this.request<Schema["LakeWorkspaceJobs"]>(
      `${base}/workspace/jobs?${query(values)}`,
      { signal: signal ?? null },
    );
  }
  workspaceSchedules(
    options: PageOptions & {
      library_id?: string | undefined;
      include_pinterest?: boolean | undefined;
    } = {},
  ) {
    const { signal, ...values } = options;
    return this.request<Schema["LakeWorkspaceSchedules"]>(
      `${base}/workspace/schedules?${query(values)}`,
      { signal: signal ?? null },
    );
  }
  schedules(options: PageOptions & { library_id?: string | undefined } = {}) {
    const { signal, ...values } = options;
    return this.request<Schema["CollectionSchedules"]>(
      `${base}/schedules?${query(values)}`,
      { signal: signal ?? null },
    );
  }
  saveSchedule(value: Schema["SaveCollectionSchedule"]) {
    return this.request<Schema["CollectionSchedule"]>(
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
