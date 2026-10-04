import type { Schema } from "@studio/contracts";
import { AestheticAnalysisClient } from "./aesthetic-analysis.js";
type Request = <T>(path: string, init?: RequestInit) => Promise<T>;
const root = (pid: string) =>
  `/v1/projects/${encodeURIComponent(pid)}/aesthetic`;
const stage = (pid: string, id: string) =>
  `${root(pid)}/stages/${encodeURIComponent(id)}`;
const post = (body: unknown): RequestInit => ({
  method: "POST",
  body: JSON.stringify(body),
});
export class AestheticClient {
  readonly analysis: AestheticAnalysisClient;
  constructor(private readonly request: Request) {
    this.analysis = new AestheticAnalysisClient(request);
  }
  create(pid: string, value: Schema["AestheticCreate"]) {
    return this.request<Schema["AestheticStage"]>(
      `${root(pid)}/stages`,
      post(value),
    );
  }
  capabilities(pid: string, signal?: AbortSignal) {
    return this.request<Schema["AestheticCapabilities"]>(
      `${root(pid)}/capabilities`,
      { signal: signal ?? null },
    );
  }
  preflight(
    pid: string,
    value: Schema["AestheticCreate"],
    signal?: AbortSignal,
  ) {
    return this.request<Schema["AestheticPreflight"]>(
      `${root(pid)}/preflight`,
      { ...post(value), signal: signal ?? null },
    );
  }
  decideCandidate(
    pid: string,
    id: string,
    ordinal: number,
    value: Schema["AestheticCandidateDecision"],
  ) {
    return this.request<Schema["AestheticCandidate"]>(
      `${stage(pid, id)}/candidates/${ordinal}/disposition`,
      post(value),
    );
  }
  abandonCreation(pid: string, id: string) {
    return this.request<Schema["OkResponse"]>(
      `${root(pid)}/creation-intents/${encodeURIComponent(id)}/abandon`,
      post({}),
    );
  }
  stages(
    pid: string,
    after?: string,
    signal?: AbortSignal,
    options: { archived?: boolean; search?: string; state?: string } = {},
  ) {
    return this.request<Schema["AestheticStages"]>(
      `${root(pid)}/stages?${new URLSearchParams({ ...(after ? { after } : {}), ...(options.archived ? { archived: "true" } : {}), ...(options.search ? { search: options.search } : {}), ...(options.state ? { state: options.state } : {}) })}`,
      { signal: signal ?? null },
    );
  }
  stage(pid: string, id: string, signal?: AbortSignal) {
    return this.request<Schema["AestheticStage"]>(stage(pid, id), {
      signal: signal ?? null,
    });
  }
  control(
    pid: string,
    id: string,
    action: "start" | "pause" | "cancel" | "parse",
  ) {
    return this.request<Schema["AestheticStage"]>(
      `${stage(pid, id)}/control`,
      post({ action }),
    );
  }
  configureSampling(
    pid: string,
    id: string,
    value: Schema["AestheticSamplingRequest"],
  ) {
    return this.request<Schema["AestheticStage"]>(
      `${stage(pid, id)}/sampling`,
      post(value),
    );
  }
  configureExecution(
    pid: string,
    id: string,
    value: Schema["AestheticExecutionUpdate"],
  ) {
    return this.request<Schema["AestheticStage"]>(
      `${stage(pid, id)}/execution`,
      post(value),
    );
  }
  stageMetadata(
    pid: string,
    id: string,
    value: Schema["AestheticStageMetadata"],
  ) {
    return this.request<Schema["AestheticStage"]>(
      `${stage(pid, id)}/metadata`,
      post(value),
    );
  }
  batchAction(pid: string, id: string, value: Schema["AestheticBatchAction"]) {
    return this.request<Schema["AestheticBatchActionResult"]>(
      `${stage(pid, id)}/batch-actions`,
      post(value),
    );
  }
  samplingDiagnostic(
    pid: string,
    id: string,
    ordinal: number,
    signal?: AbortSignal,
  ) {
    return this.request<Schema["AestheticSamplingDiagnostic"] | null>(
      `${stage(pid, id)}/sampling/${ordinal}`,
      { signal: signal ?? null },
    );
  }
  batches(
    pid: string,
    id: string,
    after?: string,
    signal?: AbortSignal,
    options: { state?: string; sequence?: number } = {},
  ) {
    return this.request<Schema["AestheticBatches"]>(
      `${stage(pid, id)}/batches?${new URLSearchParams({ ...(after ? { after } : {}), ...(options.state ? { state: options.state } : {}), ...(options.sequence ? { sequence: String(options.sequence) } : {}) })}`,
      { signal: signal ?? null },
    );
  }
  candidates(
    pid: string,
    id: string,
    protectedOnly = false,
    after?: string,
    signal?: AbortSignal,
    disposition?: Schema["AestheticDisposition"],
    blocked = false,
  ) {
    const query = new URLSearchParams({
      protected: String(protectedOnly),
      ...(after ? { after } : {}),
      ...(disposition ? { disposition } : {}),
      ...(blocked ? { blocked: "true" } : {}),
    });
    return this.request<Schema["AestheticCandidates"]>(
      `${stage(pid, id)}/candidates?${query}`,
      { signal: signal ?? null },
    );
  }
  candidate(pid: string, id: string, ordinal: number, signal?: AbortSignal) {
    return this.request<Schema["AestheticCandidate"]>(
      `${stage(pid, id)}/candidates/${ordinal}`,
      { signal: signal ?? null },
    );
  }
  attempts(pid: string, id: string, batch: number) {
    return this.request<Schema["AestheticAttempts"]>(
      `${stage(pid, id)}/batches/${batch}/attempts`,
    );
  }
  reparse(pid: string, id: string, batch: number) {
    return this.request<Schema["OkResponse"]>(
      `${stage(pid, id)}/batches/${batch}/reparse`,
      post({}),
    );
  }
  restore(packageDirectory: string, destination: string) {
    return this.request<Schema["OkResponse"]>(
      "/v1/recovery/restore",
      post({ package_directory: packageDirectory, destination }),
    );
  }
  retry(
    pid: string,
    id: string,
    batch: number,
    acknowledgePossibleCharge: boolean,
  ) {
    return this.request<Schema["OkResponse"]>(
      `${stage(pid, id)}/batches/${batch}/retry`,
      post({ acknowledge_possible_charge: acknowledgePossibleCharge }),
    );
  }
  metrics(pid: string, signal?: AbortSignal) {
    return this.request<Schema["AestheticMetrics"]>(`${root(pid)}/metrics`, {
      signal: signal ?? null,
    });
  }
  recoveryPackage(pid: string) {
    return this.request<Schema["AestheticBackup"]>(
      `${root(pid)}/recovery-package`,
      post({}),
    );
  }
  backup(pid: string) {
    return this.request<Schema["AestheticBackup"]>(
      `${root(pid)}/backup`,
      post({}),
    );
  }
}
