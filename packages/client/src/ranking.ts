import type { Schema } from "@studio/contracts";
type Request = <T>(path: string, init?: RequestInit) => Promise<T>;
const path = (pid: string, aid: string) =>
  `/v1/projects/${encodeURIComponent(pid)}/artifacts/${encodeURIComponent(aid)}/ranking`;
export class RankingClient {
  constructor(private readonly request: Request) {}
  browseInfo(pid: string, scope: Schema["ScopeRef"], signal?: AbortSignal) {
    const query = new URLSearchParams();
    if (scope.target.kind === "workset")
      query.set("collection_id", scope.target.collection_id);
    else if (scope.target.kind === "query_result")
      query.set("result_id", scope.target.result_id);
    else throw new Error("排名浏览需要工作集或查询结果范围。");
    return this.request<Schema["RankingBrowseInfo"]>(
      `/v1/projects/${encodeURIComponent(pid)}/ranking-browse?${query}`,
      { signal: signal ?? null },
    );
  }
  browseAssets(
    pid: string,
    body: Schema["RankingBrowseRequest"],
    signal?: AbortSignal,
    priority: "interactive" | "prefetch" = "interactive",
  ) {
    return this.request<Schema["AssetPage"]>(
      `/v1/projects/${encodeURIComponent(pid)}/ranking-browse/assets`,
      {
        method: "POST",
        body: JSON.stringify(body),
        signal: signal ?? null,
        headers: { "x-studio-read-priority": priority },
      },
    );
  }
  summary(pid: string, aid: string, signal?: AbortSignal) {
    return this.request<Schema["RankingSummary"]>(path(pid, aid), {
      signal: signal ?? null,
    });
  }
  rows(
    pid: string,
    aid: string,
    body: Schema["RankingPageRequest"],
    signal?: AbortSignal,
  ) {
    return this.request<Schema["RankingPage"]>(path(pid, aid) + "/rows", {
      method: "POST",
      body: JSON.stringify(body),
      signal: signal ?? null,
    });
  }
  evidence(pid: string, aid: string, signal?: AbortSignal) {
    return this.request<Schema["RankingEvidence"]>(
      path(pid, aid) + "/evidence",
      { signal: signal ?? null },
    );
  }
  count(
    pid: string,
    aid: string,
    filter: Schema["RankingFilter"],
    signal?: AbortSignal,
  ) {
    return this.request<Schema["RankingCount"]>(path(pid, aid) + "/count", {
      method: "POST",
      body: JSON.stringify({ filter }),
      signal: signal ?? null,
      headers: { "x-studio-read-priority": "background" },
    });
  }
  workset(pid: string, aid: string, body: Schema["RankingWorksetRequest"]) {
    return this.request<Schema["Collection"]>(path(pid, aid) + "/worksets", {
      method: "POST",
      body: JSON.stringify(body),
    });
  }
  saveProgress(pid: string, id: string, signal?: AbortSignal) {
    return this.request<Schema["MemberWriteProgress"]>(
      `/v1/projects/${encodeURIComponent(pid)}/member-writes/${encodeURIComponent(id)}`,
      { signal: signal ?? null },
    );
  }
  cancelSave(pid: string, id: string) {
    return this.request<Schema["OkResponse"]>(
      `/v1/projects/${encodeURIComponent(pid)}/member-writes/${encodeURIComponent(id)}/cancel`,
      { method: "POST" },
    );
  }
  jobResult(pid: string, jid: string, signal?: AbortSignal) {
    return this.request<Schema["Artifact"]>(
      `/v1/projects/${encodeURIComponent(pid)}/jobs/${encodeURIComponent(jid)}/ranking`,
      { signal: signal ?? null },
    );
  }
}
