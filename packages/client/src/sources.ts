import type { Schema, Source } from "@studio/contracts";
type Transport = <T>(path: string, init?: RequestInit) => Promise<T>;
const project = (id: string) => "/v1/projects/" + encodeURIComponent(id);
export type SourceRelationOptions = {
  version?: string;
  cursor?: string;
  limit?: number;
  manifest_id?: string;
  recipe_id?: string;
  signal?: AbortSignal;
};
function relationPath(
  projectId: string,
  sourceId: string,
  path: string,
  options: SourceRelationOptions,
) {
  const query = new URLSearchParams();
  for (const key of [
    "version",
    "cursor",
    "limit",
    "manifest_id",
    "recipe_id",
  ] as const)
    if (options[key] !== undefined) query.set(key, String(options[key]));
  return `${project(projectId)}/sources/${encodeURIComponent(sourceId)}/${path}?${query}`;
}

export function sourceSupports(
  source: Source,
  capability: keyof Schema["SourceCapabilities"],
) {
  return source.descriptor?.capabilities[capability] === true;
}
export class SourceClient {
  constructor(private readonly request: Transport) {}
  work(
    projectId: string,
    sourceId: string,
    workId: string,
    options: SourceRelationOptions = {},
  ) {
    return this.request<Schema["SourceWorkDetail"]>(
      relationPath(
        projectId,
        sourceId,
        `works/${encodeURIComponent(workId)}`,
        options,
      ),
      { signal: options.signal ?? null },
    );
  }
  workMedia(
    projectId: string,
    sourceId: string,
    workId: string,
    options: SourceRelationOptions = {},
  ) {
    return this.request<Schema["WorkMediaPage"]>(
      relationPath(
        projectId,
        sourceId,
        `works/${encodeURIComponent(workId)}/media`,
        options,
      ),
      { signal: options.signal ?? null },
    );
  }
  author(
    projectId: string,
    sourceId: string,
    authorId: string,
    options: SourceRelationOptions = {},
  ) {
    return this.request<Schema["SourceAuthorDetail"]>(
      relationPath(
        projectId,
        sourceId,
        `authors/${encodeURIComponent(authorId)}`,
        options,
      ),
      { signal: options.signal ?? null },
    );
  }
  authorWorks(
    projectId: string,
    sourceId: string,
    authorId: string,
    options: SourceRelationOptions = {},
  ) {
    return this.request<Schema["SourceAuthorWorks"]>(
      relationPath(
        projectId,
        sourceId,
        `authors/${encodeURIComponent(authorId)}/works`,
        options,
      ),
      { signal: options.signal ?? null },
    );
  }
  adapters(signal?: AbortSignal) {
    return this.request<Schema["SourceRegistrations"]>("/v1/source-adapters", {
      signal: signal ?? null,
    });
  }
  probe(body: Schema["ProbeSource"], signal?: AbortSignal) {
    return this.request<Schema["SourcePreflight"]>("/v1/source-probes", {
      method: "POST",
      body: JSON.stringify(body),
      signal: signal ?? null,
    });
  }
  list(id: string) {
    return this.request<Schema["Sources"]>(project(id) + "/sources");
  }
  attach(id: string, body: Schema["AttachSource"]) {
    return this.request<Schema["Source"]>(project(id) + "/sources", {
      method: "POST",
      body: JSON.stringify(body),
    });
  }
  relink(id: string, sourceId: string, body: Schema["RelinkSource"]) {
    return this.request<Schema["SourceRelinked"]>(
      project(id) + "/sources/" + encodeURIComponent(sourceId) + "/relink",
      { method: "POST", body: JSON.stringify(body) },
    );
  }
  requirements(
    id: string,
    scope: Schema["ScopeRef"],
    projections: string[],
    signal?: AbortSignal,
  ) {
    return this.request<Schema["SourceRequirementsResult"]>(
      project(id) + "/source-requirements",
      {
        method: "POST",
        body: JSON.stringify({ scope, projections }),
        signal: signal ?? null,
      },
    );
  }
}
