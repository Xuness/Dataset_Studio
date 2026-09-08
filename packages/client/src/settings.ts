import type { Schema } from "@studio/contracts";

type Request = <T>(path: string, init?: RequestInit) => Promise<T>;
const project = (id: string) => "/v1/projects/" + encodeURIComponent(id);
const basis = (sourceId: string, rating: string) =>
  "/v1/cache/rating-bases/" +
  encodeURIComponent(sourceId) +
  "/" +
  encodeURIComponent(rating);

export class SettingsClient {
  constructor(private readonly request: Request) {}
  read(signal?: AbortSignal) {
    return this.request<Schema["SettingsStatus"]>("/v1/settings", {
      signal: signal ?? null,
    });
  }
  configureCache(value: Schema["CacheSettings"]) {
    return this.request<Schema["SettingsStatus"]>("/v1/settings/cache", {
      method: "PUT",
      body: JSON.stringify(value),
    });
  }
  clear(tier: "long_term" | "temporary" | null) {
    return this.request<Schema["SettingsStatus"]>("/v1/settings/cache/clear", {
      method: "POST",
      body: JSON.stringify({ tier }),
    });
  }
  entries(projectId: string, cursor?: string, signal?: AbortSignal) {
    const query = new URLSearchParams({ limit: "32" });
    if (cursor) query.set("cursor", cursor);
    return this.request<Schema["CacheEntries"]>(
      project(projectId) + "/cache-entries?" + query,
      { signal: signal ?? null },
    );
  }
  retention(
    projectId: string,
    resultId: string,
    tier: "long_term" | "temporary",
    fixed: boolean,
  ) {
    return this.request<Schema["QueryResult"]>(
      project(projectId) +
        "/query-results/" +
        encodeURIComponent(resultId) +
        "/retention",
      {
        method: "PUT",
        body: JSON.stringify({ tier, fixed }),
      },
    );
  }
  release(projectId: string, resultId: string) {
    return this.request<Schema["QueryResult"]>(
      project(projectId) +
        "/query-results/" +
        encodeURIComponent(resultId) +
        "/cache-release",
      { method: "POST" },
    );
  }
  ratingBases(signal?: AbortSignal) {
    return this.request<Schema["RatingBases"]>("/v1/cache/rating-bases", {
      signal: signal ?? null,
    });
  }
  prebuild(projectId: string, sourceId: string) {
    return this.request<Schema["RatingBuild"]>(
      project(projectId) +
        "/sources/" +
        encodeURIComponent(sourceId) +
        "/rating-bases",
      { method: "POST" },
    );
  }
  cancelBuild(sourceId: string) {
    return this.request<Schema["OkResponse"]>(
      "/v1/cache/rating-bases/" + encodeURIComponent(sourceId) + "/cancel",
      { method: "POST" },
    );
  }
  fixBasis(sourceId: string, rating: string, fixed: boolean) {
    return this.request<Schema["OkResponse"]>(basis(sourceId, rating), {
      method: "PUT",
      body: JSON.stringify({ fixed }),
    });
  }
  releaseBasis(sourceId: string, rating: string) {
    return this.request<Schema["OkResponse"]>(
      basis(sourceId, rating) + "/release",
      { method: "POST" },
    );
  }
}
