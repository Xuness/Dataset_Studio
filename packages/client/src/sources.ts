import type { Schema, Source } from "@studio/contracts";
type Transport = <T>(path: string, init?: RequestInit) => Promise<T>;
const project = (id: string) => "/v1/projects/" + encodeURIComponent(id);

export function sourceSupports(
  source: Source,
  capability: keyof Schema["SourceCapabilities"],
) {
  return source.descriptor?.capabilities[capability] === true;
}
export class SourceClient {
  constructor(private readonly request: Transport) {}
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
