import type { Schema } from "@studio/contracts";
import type { PageOptions } from "./queries.js";
type Request = <T>(path: string, init?: RequestInit) => Promise<T>;
const project = (id: string) => "/v1/projects/" + encodeURIComponent(id);
const query = (options: PageOptions) => {
  const params = new URLSearchParams();
  if (options.cursor) params.set("cursor", options.cursor);
  if (options.limit !== undefined) params.set("limit", String(options.limit));
  return params;
};
export class ToolClient {
  constructor(private readonly request: Request) {}
  operators(signal?: AbortSignal) {
    return this.request<Schema["Operators"]>("/v1/operators", {
      signal: signal ?? null,
    });
  }
  submit(pid: string, body: Schema["ToolSubmission"]) {
    return this.request<Schema["Job"]>(project(pid) + "/tools/jobs", {
      method: "POST",
      body: JSON.stringify(body),
    });
  }
  validateScope(pid: string, scope: Schema["ScopeRef"], signal?: AbortSignal) {
    return this.request<Schema["OkResponse"]>(
      project(pid) + "/tools/validate-scope",
      {
        method: "POST",
        body: JSON.stringify({ scope }),
        ...(signal ? { signal } : {}),
      },
    );
  }
  run(pid: string, jid: string) {
    return this.request<Schema["JobRun"]>(
      project(pid) + "/jobs/" + encodeURIComponent(jid) + "/run",
    );
  }
  retry(pid: string, jid: string) {
    return this.request<Schema["Job"]>(
      project(pid) + "/jobs/" + encodeURIComponent(jid) + "/retry",
      { method: "POST" },
    );
  }
  artifacts(pid: string, options: PageOptions = {}) {
    return this.request<Schema["Artifacts"]>(
      project(pid) + "/artifacts?" + query(options),
      { signal: options.signal ?? null },
    );
  }
  artifact(pid: string, aid: string, signal?: AbortSignal) {
    return this.request<Schema["Artifact"]>(
      project(pid) + "/artifacts/" + encodeURIComponent(aid),
      { signal: signal ?? null },
    );
  }
  rows(pid: string, aid: string, options: PageOptions = {}) {
    return this.request<Schema["ArtifactPage"]>(
      project(pid) +
        "/artifacts/" +
        encodeURIComponent(aid) +
        "/rows?" +
        query(options),
      { signal: options.signal ?? null },
    );
  }
  verify(pid: string, aid: string) {
    return this.request<Schema["Artifact"]>(
      project(pid) + "/artifacts/" + encodeURIComponent(aid) + "/verify",
      { method: "POST" },
    );
  }
  release(pid: string, aid: string) {
    return this.request<Schema["Artifact"]>(
      project(pid) + "/artifacts/" + encodeURIComponent(aid) + "/release",
      { method: "POST" },
    );
  }
}
export class DraftClient {
  constructor(private readonly request: Request) {}
  get(pid: string, module: string, instance = "default", signal?: AbortSignal) {
    return this.request<Schema["MaybeDraft"]>(
      project(pid) +
        "/drafts/" +
        encodeURIComponent(module) +
        "/" +
        encodeURIComponent(instance),
      { signal: signal ?? null },
    );
  }
  save(
    pid: string,
    module: string,
    instance: string,
    body: Schema["SaveDraft"],
  ) {
    return this.request<Schema["Draft"]>(
      project(pid) +
        "/drafts/" +
        encodeURIComponent(module) +
        "/" +
        encodeURIComponent(instance),
      { method: "PUT", body: JSON.stringify(body) },
    );
  }
  preference(key: string, signal?: AbortSignal) {
    return this.request<Schema["MaybePreference"]>(
      "/v1/preferences/" + encodeURIComponent(key),
      { signal: signal ?? null },
    );
  }
  savePreference(key: string, body: Schema["SaveDraft"]) {
    return this.request<Schema["Preference"]>(
      "/v1/preferences/" + encodeURIComponent(key),
      { method: "PUT", body: JSON.stringify(body) },
    );
  }
}
