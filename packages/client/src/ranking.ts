import type { Schema } from "@studio/contracts";
type Request = <T>(path: string, init?: RequestInit) => Promise<T>;
const path = (pid: string, aid: string) =>
  `/v1/projects/${encodeURIComponent(pid)}/artifacts/${encodeURIComponent(aid)}/ranking`;
export class RankingClient {
  constructor(private readonly request: Request) {}
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
  workset(pid: string, aid: string, body: Schema["RankingWorksetRequest"]) {
    return this.request<Schema["Collection"]>(path(pid, aid) + "/worksets", {
      method: "POST",
      body: JSON.stringify(body),
    });
  }
  jobResult(pid: string, jid: string, signal?: AbortSignal) {
    return this.request<Schema["Artifact"]>(
      `/v1/projects/${encodeURIComponent(pid)}/jobs/${encodeURIComponent(jid)}/ranking`,
      { signal: signal ?? null },
    );
  }
}
