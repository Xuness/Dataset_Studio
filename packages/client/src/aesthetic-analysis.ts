import type { Schema } from "@studio/contracts";
type Request = <T>(path: string, init?: RequestInit) => Promise<T>;
const root = (pid: string) =>
  `/v1/projects/${encodeURIComponent(pid)}/aesthetic/analysis`;
const post = (value?: unknown): RequestInit => ({
  method: "POST",
  ...(value === undefined ? {} : { body: JSON.stringify(value) }),
});

/** Offline jobs never dispatch provider calls. Published snapshots are immutable. */
export class AestheticAnalysisClient {
  constructor(private readonly request: Request) {}
  select(
    pid: string,
    id: string,
    value: Schema["AestheticRankingQuery"],
    signal?: AbortSignal,
  ) {
    return this.request<Schema["AestheticRankingSelection"]>(
      `${root(pid)}/snapshots/${encodeURIComponent(id)}/select`,
      { ...post(value), signal: signal ?? null },
    );
  }
  create(pid: string, value: Schema["AestheticAnalysisCreate"]) {
    return this.request<Schema["AestheticAnalysisJob"]>(
      `${root(pid)}/jobs`,
      post(value),
    );
  }
  jobs(
    pid: string,
    options: { after?: string; experiment_id?: string; limit?: number } = {},
    signal?: AbortSignal,
  ) {
    return this.request<Schema["AestheticAnalysisJobs"]>(
      `${root(pid)}/jobs?${query(options)}`,
      { signal: signal ?? null },
    );
  }
  job(pid: string, id: string, signal?: AbortSignal) {
    return this.request<Schema["AestheticAnalysisJob"]>(
      `${root(pid)}/jobs/${encodeURIComponent(id)}`,
      { signal: signal ?? null },
    );
  }
  control(pid: string, id: string, action: "cancel" | "resume") {
    return this.request<Schema["AestheticAnalysisJob"]>(
      `${root(pid)}/jobs/${encodeURIComponent(id)}/control`,
      post({ action }),
    );
  }
  latestForStage(pid: string, id: string, signal?: AbortSignal) {
    return this.request<Schema["AestheticAnalysisJob"] | null>(
      `${root(pid)}/stages/${encodeURIComponent(id)}/latest-snapshot`,
      { signal: signal ?? null },
    );
  }
  snapshot(pid: string, id: string, signal?: AbortSignal) {
    return this.request<Schema["AestheticAnalysisJob"]>(
      `${root(pid)}/snapshots/${encodeURIComponent(id)}`,
      { signal: signal ?? null },
    );
  }
  rows(
    pid: string,
    id: string,
    options: { after?: string; rating?: string; limit?: number } = {},
    signal?: AbortSignal,
  ) {
    return this.request<Schema["AestheticRankingRows"]>(
      `${root(pid)}/snapshots/${encodeURIComponent(id)}/rows?${query(options)}`,
      { signal: signal ?? null },
    );
  }
  candidate(pid: string, id: string, ordinal: number, signal?: AbortSignal) {
    return this.request<Schema["AestheticRankingRow"]>(
      `${root(pid)}/snapshots/${encodeURIComponent(id)}/candidates/${ordinal}`,
      { signal: signal ?? null },
    );
  }
  comparison(
    pid: string,
    id: string,
    options: { after?: string; limit?: number } = {},
    signal?: AbortSignal,
  ) {
    return this.request<Schema["AestheticComparisonRows"]>(
      `${root(pid)}/jobs/${encodeURIComponent(id)}/comparison?${query(options)}`,
      { signal: signal ?? null },
    );
  }
  createExperiment(pid: string, value: Schema["AestheticExperimentCreate"]) {
    return this.request<Schema["AestheticExperiment"]>(
      `${root(pid)}/experiments`,
      post(value),
    );
  }
  experiments(pid: string, after?: string, signal?: AbortSignal) {
    return this.request<Schema["AestheticExperiments"]>(
      `${root(pid)}/experiments?${query({ after })}`,
      { signal: signal ?? null },
    );
  }
  experiment(pid: string, id: string, signal?: AbortSignal) {
    return this.request<Schema["AestheticExperiment"]>(
      `${root(pid)}/experiments/${encodeURIComponent(id)}`,
      { signal: signal ?? null },
    );
  }
  runExperiment(pid: string, id: string) {
    return this.request<Schema["AestheticAnalysisJobs"]>(
      `${root(pid)}/experiments/${encodeURIComponent(id)}/run`,
      post(),
    );
  }
  review(pid: string, value: Schema["AestheticReviewCreate"]) {
    return this.request<Schema["AestheticReview"]>(
      `${root(pid)}/reviews`,
      post(value),
    );
  }
  reviews(pid: string, snapshot: string, after?: string, signal?: AbortSignal) {
    return this.request<Schema["AestheticReviews"]>(
      `${root(pid)}/snapshots/${encodeURIComponent(snapshot)}/reviews?${query({ after })}`,
      { signal: signal ?? null },
    );
  }
  /** Newest first, bounded to one candidate; never scans the snapshot in the UI. */
  candidateReviews(
    pid: string,
    snapshot: string,
    ordinal: number,
    after?: string,
    signal?: AbortSignal,
  ) {
    return this.request<Schema["AestheticReviews"]>(
      `${root(pid)}/snapshots/${encodeURIComponent(snapshot)}/reviews?${query({ ordinal, after })}`,
      { signal: signal ?? null },
    );
  }
}
function query(options: Record<string, string | number | undefined>): string {
  const result = new URLSearchParams();
  for (const [key, value] of Object.entries(options))
    if (value !== undefined) result.set(key, String(value));
  return result.toString();
}
