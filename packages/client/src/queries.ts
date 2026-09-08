import type { Schema } from "@studio/contracts";

type Transport = <T>(path: string, init?: RequestInit) => Promise<T>;
export type PageOptions = {
  order?: Schema["QueryOrder"];
  cursor?: string;
  limit?: number;
  signal?: AbortSignal;
  priority?: "interactive" | "background" | "prefetch";
};
const projectPath = (id: string) => "/v1/projects/" + encodeURIComponent(id);
function pageQuery(options: PageOptions) {
  const query = new URLSearchParams();
  if (options.cursor) query.set("cursor", options.cursor);
  if (options.limit !== undefined) query.set("limit", String(options.limit));
  if (options.order) query.set("order", options.order);
  return query;
}
export class QueryClient {
  constructor(private readonly request: Transport) {}
  fields(projectId: string, sourceId: string, signal?: AbortSignal) {
    return this.request<Schema["FieldDirectory"]>(
      projectPath(projectId) +
        "/sources/" +
        encodeURIComponent(sourceId) +
        "/fields",
      signal ? { signal } : {},
    );
  }
  definitions(projectId: string, options: PageOptions = {}) {
    return this.request<Schema["QueryDefinitions"]>(
      projectPath(projectId) + "/queries?" + pageQuery(options),
      options.signal ? { signal: options.signal } : {},
    );
  }
  definition(projectId: string, id: string) {
    return this.request<Schema["QueryDefinition"]>(
      projectPath(projectId) + "/queries/" + encodeURIComponent(id),
    );
  }
  save(projectId: string, body: Schema["SaveQuery"], id?: string) {
    return this.request<Schema["QueryDefinition"]>(
      projectPath(projectId) +
        "/queries" +
        (id ? "/" + encodeURIComponent(id) : ""),
      { method: id ? "PATCH" : "POST", body: JSON.stringify(body) },
    );
  }
  build(projectId: string, id: string, expectedRevision: number) {
    return this.request<Schema["QueryResult"]>(
      projectPath(projectId) +
        "/queries/" +
        encodeURIComponent(id) +
        "/results",
      {
        method: "POST",
        body: JSON.stringify({ expected_revision: expectedRevision }),
      },
    );
  }
  run(projectId: string, spec: Schema["QuerySpec"]) {
    return this.request<Schema["QueryResult"]>(
      projectPath(projectId) + "/query-results",
      {
        method: "POST",
        body: JSON.stringify({ spec }),
      },
    );
  }
  async latestSpec(projectId: string, spec: Schema["QuerySpec"]) {
    const target = spec.input_scope?.target;
    if (!target || target.kind !== "source") return spec;
    const sources = await this.request<Schema["Sources"]>(
      projectPath(projectId) + "/sources",
    );
    const source = sources.items.find(
      (s) => s.id === target.source_id && s.available,
    );
    if (!source?.revision)
      throw new Error("查询来源暂不可用，请刷新数据湖后重试");
    return {
      ...spec,
      input_scope: {
        project_id: projectId,
        target: { ...target, revision: source.revision },
      },
    };
  }
  async runLatest(projectId: string, spec: Schema["QuerySpec"]) {
    return this.run(projectId, await this.latestSpec(projectId, spec));
  }
  lease(projectId: string, id: string, leaseId: string, release = false) {
    return this.request<Schema["OkResponse"]>(
      projectPath(projectId) +
        "/query-results/" +
        encodeURIComponent(id) +
        "/leases/" +
        encodeURIComponent(leaseId) +
        (release ? "/release" : ""),
      { method: "POST" },
    );
  }
  results(projectId: string, options: PageOptions = {}) {
    return this.request<Schema["QueryResults"]>(
      projectPath(projectId) + "/query-results?" + pageQuery(options),
      options.signal ? { signal: options.signal } : {},
    );
  }
  result(projectId: string, id: string, signal?: AbortSignal) {
    return this.request<Schema["QueryResult"]>(
      projectPath(projectId) + "/query-results/" + encodeURIComponent(id),
      signal ? { signal } : {},
    );
  }
  validity(projectId: string, id: string, signal?: AbortSignal) {
    return this.request<Schema["ResultValidity"]>(
      projectPath(projectId) +
        "/query-results/" +
        encodeURIComponent(id) +
        "/validity",
      signal ? { signal } : {},
    );
  }
  assets(projectId: string, id: string, options: PageOptions = {}) {
    return this.request<Schema["ResultAssets"]>(
      projectPath(projectId) +
        "/query-results/" +
        encodeURIComponent(id) +
        "/assets?" +
        pageQuery(options),
      {
        ...(options.signal ? { signal: options.signal } : {}),
        headers: {
          "x-studio-read-priority": options.priority ?? "interactive",
        },
      },
    );
  }
  cancel(projectId: string, id: string) {
    return this.request<Schema["QueryResult"]>(
      projectPath(projectId) +
        "/query-results/" +
        encodeURIComponent(id) +
        "/cancel",
      { method: "POST" },
    );
  }
  release(projectId: string, id: string) {
    return this.request<Schema["QueryResult"]>(
      projectPath(projectId) +
        "/query-results/" +
        encodeURIComponent(id) +
        "/release",
      { method: "POST" },
    );
  }
  capture(projectId: string, scope: Schema["ScopeRef"]) {
    return this.request<Schema["QueryResult"]>(
      projectPath(projectId) + "/scopes/capture",
      { method: "POST", body: JSON.stringify({ scope }) },
    );
  }
}
