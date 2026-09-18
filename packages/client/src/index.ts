import type {
  Schema,
  EngineConnection,
  ProjectEvent,
  AssetKey,
  Asset,
} from "@studio/contracts";
import { QueryClient } from "./queries.js";
import { ToolClient, DraftClient } from "./tools.js";
import { DraftCoordinator } from "./drafts.js";
import { MediaClient, ResourceClient } from "./media.js";
import { SettingsClient } from "./settings.js";
import { LlmClient } from "./llm/index.js";
export { LlmCallError } from "./llm/index.js";
export type { LlmInvocationInput } from "./llm/index.js";
import { RankingClient } from "./ranking.js";
import { AestheticClient } from "./aesthetic.js";
import { ManagementClient } from "./management.js";
export type { ObjectTarget, ObjectListOptions } from "./management.js";
import type { MediaOptions, MediaHandle } from "./media.js";
export { DraftController, DraftCoordinator } from "./drafts.js";
export type { DraftSnapshot, DraftStatus } from "./drafts.js";
export type { PageOptions } from "./queries.js";
export class StudioError extends Error {
  constructor(
    public readonly code: string,
    message: string,
    public readonly requestId?: string,
  ) {
    super(message);
    this.name = "StudioError";
  }
}
function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
export function validateConnection(value: unknown): EngineConnection {
  if (
    !record(value) ||
    value.api_version !== 1 ||
    typeof value.endpoint !== "string" ||
    typeof value.token !== "string" ||
    typeof value.instance_id !== "string" ||
    typeof value.pid !== "number"
  )
    throw new StudioError("API_VERSION_MISMATCH", "本机引擎协议不兼容。");
  const url = new URL(value.endpoint);
  if (
    url.protocol !== "http:" ||
    url.hostname !== "127.0.0.1" ||
    url.username ||
    url.password ||
    url.pathname !== "/"
  )
    throw new StudioError("CONNECTION_INVALID", "本机引擎地址无效。");
  return value as EngineConnection;
}
export const assetIdentity = (key: AssetKey) =>
  key.source_id + ":" + key.asset_id;
export type MetadataOptions = {
  observationId?: string;
  cursor?: string;
  limit?: number;
  version?: string;
  signal?: AbortSignal;
};
function metadataPath(projectId: string, key: AssetKey) {
  return (
    "/v1/projects/" +
    encodeURIComponent(projectId) +
    "/sources/" +
    encodeURIComponent(key.source_id) +
    "/assets/" +
    encodeURIComponent(key.asset_id)
  );
}
function metadataQuery(options: MetadataOptions) {
  const query = new URLSearchParams();
  if (options.cursor) query.set("cursor", options.cursor);
  if (options.limit !== undefined) query.set("limit", String(options.limit));
  if (options.version) query.set("version", options.version);
  if (options.observationId) query.set("observation_id", options.observationId);
  return query;
}
export class StudioClient {
  readonly aesthetic = new AestheticClient(
    <T>(path: string, init?: RequestInit) => this.request<T>(path, init),
  );
  readonly llm = new LlmClient(
    <T>(path: string, init?: RequestInit) => this.request<T>(path, init),
    () => this.connection,
  );
  readonly management = new ManagementClient(
    <T>(path: string, init?: RequestInit) => this.request<T>(path, init),
  );
  readonly ranking = new RankingClient(<T>(path: string, init?: RequestInit) =>
    this.request<T>(path, init),
  );
  private sessionId = crypto.randomUUID();
  private projectSessions = new Map<string, string>();
  private sessionTimer: ReturnType<typeof setInterval> | null = null;
  readonly settings = new SettingsClient(
    <T>(path: string, init?: RequestInit) => this.request<T>(path, init),
  );
  readonly tools = new ToolClient(<T>(path: string, init?: RequestInit) =>
    this.request<T>(path, init),
  );
  readonly drafts = new DraftClient(<T>(path: string, init?: RequestInit) =>
    this.request<T>(path, init),
  );
  edits = new DraftCoordinator(this.drafts);
  preserveEdits(previous: StudioClient) {
    this.edits = previous.edits;
    this.edits.rebind(this.drafts);
    this.sessionId = previous.sessionId;
    this.projectSessions = new Map(previous.projectSessions);
    this.openedProjects = new Set(previous.openedProjects);
    this.startSessionHeartbeat();
  }
  private openedProjects = new Set<string>();
  private openingProjects = new Set<Promise<Schema["Project"]>>();
  private closingViews = false;
  private trackOpen(request: () => Promise<Schema["Project"]>) {
    if (this.closingViews)
      return Promise.reject(
        new StudioError("PROJECT_CLOSED", "应用窗口正在关闭。"),
      );
    const pending = request()
      .then((project) => {
        this.openedProjects.add(project.id);
        if (!this.projectSessions.has(project.id))
          this.projectSessions.set(project.id, crypto.randomUUID());
        this.startSessionHeartbeat();
        void this.heartbeat(project.id);
        void this.edits.flush(project.id).catch(() => {});
        return project;
      })
      .finally(() => this.openingProjects.delete(pending));
    this.openingProjects.add(pending);
    return pending;
  }
  async releaseProjectViews() {
    await this.edits.flush();
    this.closingViews = true;
    try {
      await Promise.allSettled([...this.openingProjects]);
      await Promise.all(
        [...this.openedProjects].map((id) => this.closeProject(id)),
      );
    } catch (error) {
      this.closingViews = false;
      throw error;
    }
  }
  readonly queries = new QueryClient(<T>(path: string, init?: RequestInit) =>
    this.request<T>(path, init),
  );
  private mediaClient: MediaClient;
  readonly resources: ResourceClient;
  dispose() {
    this.llm.dispose();
    if (this.sessionTimer !== null) clearInterval(this.sessionTimer);
    this.sessionTimer = null;
    this.mediaClient.dispose();
  }
  private async heartbeat(projectId: string) {
    try {
      await this.request("/v1/projects/" + projectId + "/cache-session", {
        method: "POST",
      });
    } catch {
      /* Reconnection renews the same live client session. */
    }
  }
  private startSessionHeartbeat() {
    if (this.sessionTimer !== null || !this.openedProjects.size) return;
    this.sessionTimer = setInterval(() => {
      for (const id of this.openedProjects) void this.heartbeat(id);
    }, 20000);
    // SDK scripts must not be kept alive solely by an idle desktop heartbeat.
    if (typeof this.sessionTimer === "object" && "unref" in this.sessionTimer)
      this.sessionTimer.unref();
  }
  constructor(readonly connection: EngineConnection) {
    validateConnection(connection);
    this.mediaClient = new MediaClient(connection, (path, init) =>
      this.request(path, init),
    );
    this.resources = new ResourceClient(
      (path, init) => this.request(path, init),
      () => this.mediaClient.clear(),
    );
  }
  private async request<T>(path: string, init: RequestInit = {}): Promise<T> {
    const headers = new Headers(init.headers);
    headers.set("Authorization", "Bearer " + this.connection.token);
    if (init.body) headers.set("Content-Type", "application/json");
    const projectId = /^\/v1\/projects\/([0-9a-f-]{36})(?:\/|\?|$)/i.exec(
      path,
    )?.[1];
    headers.set(
      "x-studio-session",
      (projectId && this.projectSessions.get(projectId)) || this.sessionId,
    );
    const readOperation =
      !init.method ||
      init.method === "GET" ||
      (init.method === "POST" &&
        /\/(?:ranking-browse\/assets|artifacts\/[^/]+\/ranking\/(?:rows|count)|selection\/members|assets\/summaries)$/.test(
          path,
        ));
    const readId =
      readOperation && projectId && init.signal ? crypto.randomUUID() : null;
    if (readId) headers.set("x-studio-read-id", readId);
    const cancel = () => {
      if (readId && projectId)
        void this.request(
          "/v1/projects/" + projectId + "/read-requests/" + readId + "/cancel",
          { method: "POST", keepalive: true },
        ).catch(() => {});
    };
    init.signal?.addEventListener("abort", cancel, { once: true });
    try {
      let response: Response;
      try {
        response = await fetch(this.connection.endpoint + path, {
          ...init,
          headers,
        });
      } catch (error) {
        if (error instanceof DOMException && error.name === "AbortError")
          throw error;
        throw new StudioError(
          "ENGINE_DISCONNECTED",
          "本机引擎连接中断，请重连。",
        );
      }
      if (!response.ok) {
        let body: unknown;
        try {
          body = await response.json();
        } catch {
          body = null;
        }
        throw new StudioError(
          record(body) && typeof body.code === "string"
            ? body.code
            : "HTTP_ERROR",
          record(body) && typeof body.message === "string"
            ? body.message
            : "请求失败（" + response.status + "）",
          record(body) && typeof body.request_id === "string"
            ? body.request_id
            : undefined,
        );
      }
      return (await response.json()) as T;
    } finally {
      init.signal?.removeEventListener("abort", cancel);
    }
  }
  async health() {
    const result = await this.request<Schema["Health"]>("/v1/health");
    if (
      result.api_version !== 1 ||
      result.instance_id !== this.connection.instance_id
    )
      throw new StudioError("ENGINE_REPLACED", "本机引擎已变化，请重新连接。");
    return result;
  }
  projects() {
    return this.request<Schema["Projects"]>("/v1/projects");
  }
  createProject(body: Schema["CreateProject"]) {
    return this.trackOpen(() =>
      this.request<Schema["Project"]>("/v1/projects", {
        method: "POST",
        body: JSON.stringify(body),
      }),
    );
  }
  openProject(directory: string) {
    return this.trackOpen(() =>
      this.request<Schema["Project"]>("/v1/projects/open", {
        method: "POST",
        body: JSON.stringify({ directory }),
      }),
    );
  }
  openRecentProject(id: string) {
    return this.trackOpen(() =>
      this.request<Schema["Project"]>(
        "/v1/projects/" + encodeURIComponent(id) + "/open",
        { method: "POST" },
      ),
    );
  }
  async closeProject(id: string) {
    await this.edits.flush(id);
    await this.mediaClient.clear(id);
    const result = await this.request<Schema["ProjectClose"]>(
      "/v1/projects/" + encodeURIComponent(id) + "/close",
      { method: "POST" },
    );
    this.openedProjects.delete(id);
    this.projectSessions.delete(id);
    if (!this.openedProjects.size && this.sessionTimer !== null) {
      clearInterval(this.sessionTimer);
      this.sessionTimer = null;
    }
    return result;
  }
  project(id: string) {
    return this.request<Schema["Project"]>(
      "/v1/projects/" + encodeURIComponent(id),
    );
  }
  sources(id: string) {
    return this.request<Schema["Sources"]>("/v1/projects/" + id + "/sources");
  }
  attachSource(id: string, body: Schema["AttachSource"]) {
    return this.request<Schema["Source"]>("/v1/projects/" + id + "/sources", {
      method: "POST",
      body: JSON.stringify(body),
    });
  }
  relinkSource(
    projectId: string,
    sourceId: string,
    body: Schema["RelinkSource"],
  ) {
    return this.request<Schema["SourceRelinked"]>(
      "/v1/projects/" +
        encodeURIComponent(projectId) +
        "/sources/" +
        encodeURIComponent(sourceId) +
        "/relink",
      { method: "POST", body: JSON.stringify(body) },
    );
  }
  assets(
    id: string,
    options: {
      sourceId?: string;
      collectionId?: string;
      selection?: boolean;
      order?: Schema["QueryOrder"];
      cursor?: string;
      limit?: number;
      signal?: AbortSignal;
      priority?: "interactive" | "background" | "prefetch";
    } = {},
  ) {
    const query = new URLSearchParams({ limit: String(options.limit ?? 48) });
    if (options.sourceId) query.set("source_id", options.sourceId);
    if (options.collectionId) query.set("collection_id", options.collectionId);
    if (options.selection) query.set("selection", "true");
    if (options.cursor) query.set("cursor", options.cursor);
    if (options.order) query.set("order", options.order);
    return this.request<Schema["AssetPage"]>(
      "/v1/projects/" + id + "/assets?" + query,
      {
        ...(options.signal ? { signal: options.signal } : {}),
        headers: {
          "x-studio-read-priority": options.priority ?? "interactive",
        },
      },
    );
  }
  selection(id: string) {
    return this.request<Schema["Selection"]>(
      "/v1/projects/" + id + "/selection",
    );
  }
  metadata(projectId: string, key: AssetKey, options: MetadataOptions = {}) {
    return this.request<Schema["MetadataOverview"]>(
      metadataPath(projectId, key) + "/metadata?" + metadataQuery(options),
      options.signal ? { signal: options.signal } : {},
    );
  }
  asset(projectId: string, key: AssetKey, signal?: AbortSignal) {
    return this.request<Schema["Asset"]>(
      metadataPath(projectId, key),
      signal ? { signal } : {},
    );
  }
  observations(
    projectId: string,
    key: AssetKey,
    recordId: string,
    options: MetadataOptions = {},
  ) {
    return this.request<Schema["ObservationPage"]>(
      metadataPath(projectId, key) +
        "/records/" +
        encodeURIComponent(recordId) +
        "/observations?" +
        metadataQuery(options),
      options.signal ? { signal: options.signal } : {},
    );
  }
  rawMetadata(
    projectId: string,
    key: AssetKey,
    recordId: string,
    observationId: string,
    version: string,
    signal?: AbortSignal,
  ) {
    return this.request<Schema["RawMetadata"]>(
      metadataPath(projectId, key) +
        "/records/" +
        encodeURIComponent(recordId) +
        "/observations/" +
        encodeURIComponent(observationId) +
        "/raw?" +
        new URLSearchParams({ version }),
      signal ? { signal } : {},
    );
  }
  changeSelection(id: string, body: Schema["ChangeSelection"]) {
    return this.request<Schema["Selection"]>(
      "/v1/projects/" + id + "/selection",
      { method: "PATCH", body: JSON.stringify(body) },
    );
  }
  selectionMembers(id: string, keys: AssetKey[], signal?: AbortSignal) {
    return this.request<Schema["SelectionMembers"]>(
      "/v1/projects/" + encodeURIComponent(id) + "/selection/members",
      {
        method: "POST",
        body: JSON.stringify({ keys }),
        signal: signal ?? null,
      },
    );
  }
  assetSummaries(id: string, keys: AssetKey[], signal?: AbortSignal) {
    return this.request<Schema["AssetSummaries"]>(
      "/v1/projects/" + encodeURIComponent(id) + "/assets/summaries",
      {
        method: "POST",
        body: JSON.stringify({ keys }),
        signal: signal ?? null,
      },
    );
  }
  changeSelectionScope(id: string, body: Schema["ChangeSelectionScope"]) {
    return this.request<Schema["Selection"]>(
      "/v1/projects/" + encodeURIComponent(id) + "/selection/scope",
      { method: "POST", body: JSON.stringify(body) },
    );
  }
  collections(id: string) {
    return this.request<Schema["Collections"]>(
      "/v1/projects/" + id + "/collections",
    );
  }
  createCollection(id: string, name: string, scope?: Schema["ScopeRef"]) {
    return this.request<Schema["Collection"]>(
      "/v1/projects/" + id + "/collections",
      { method: "POST", body: JSON.stringify({ name, scope }) },
    );
  }
  jobs(id: string) {
    return this.request<Schema["Jobs"]>("/v1/projects/" + id + "/jobs");
  }
  submitJob(id: string, body: Schema["SubmitJob"]) {
    return this.request<Schema["Job"]>("/v1/projects/" + id + "/jobs", {
      method: "POST",
      body: JSON.stringify(body),
    });
  }
  cancelJob(id: string, job: string) {
    return this.request<Schema["Job"]>(
      "/v1/projects/" + id + "/jobs/" + job + "/cancel",
      { method: "POST" },
    );
  }
  async downloadArtifact(id: string, job: string) {
    const response = await fetch(
      this.connection.endpoint +
        "/v1/projects/" +
        id +
        "/jobs/" +
        job +
        "/artifact",
      { headers: { Authorization: "Bearer " + this.connection.token } },
    );
    if (!response.ok)
      throw new StudioError("ARTIFACT_UNAVAILABLE", "成果暂不可用。");
    const size = Number(response.headers.get("content-length") ?? NaN);
    if (!Number.isFinite(size) || size > 64 * 1024 * 1024) {
      await response.body?.cancel();
      throw new StudioError(
        "ARTIFACT_DOWNLOAD_LIMIT",
        "成果已保存在项目 artifacts 目录中。超过 64 MiB 的成果请直接从项目目录读取。",
      );
    }
    const blob = await response.blob();
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = "dataset-manifest-" + job.slice(0, 8) + ".jsonl";
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 2000);
  }
  acquireMedia(
    projectId: string,
    asset: Asset,
    edge: number,
    options: MediaOptions = {},
  ): Promise<MediaHandle> {
    return this.mediaClient.acquire(projectId, asset, edge, options);
  }
  async watch(
    projectId: string,
    onEvent: (event: ProjectEvent) => void,
    signal: AbortSignal,
  ) {
    let after: number | undefined;
    while (!signal.aborted) {
      try {
        const response = await fetch(
          this.connection.endpoint +
            "/v1/projects/" +
            projectId +
            "/events" +
            (after === undefined ? "" : "?after=" + after),
          {
            signal,
            headers: { Authorization: "Bearer " + this.connection.token },
          },
        );
        if (!response.ok) {
          const problem: unknown = await response.json().catch(() => null);
          if (record(problem) && problem.code === "PROJECT_CLOSED") {
            onEvent({
              sequence: after ?? 0,
              project_id: projectId,
              kind: "project.closed",
              resource_id: projectId,
            });
            return;
          }
        }
        if (!response.ok || !response.body)
          throw new Error("stream unavailable");
        const reader = response.body.getReader();
        const decoder = new TextDecoder();
        let buffer = "";
        try {
          while (!signal.aborted) {
            const part = await reader.read();
            if (part.done) break;
            buffer += decoder
              .decode(part.value, { stream: true })
              .replace(/\r\n/g, "\n");
            if (buffer.length > 1024 * 1024) throw new Error("event overflow");
            let index: number;
            while ((index = buffer.indexOf("\n\n")) >= 0) {
              const block = buffer.slice(0, index);
              buffer = buffer.slice(index + 2);
              const data = block
                .split("\n")
                .filter((l) => l.startsWith("data:"))
                .map((l) => l.slice(5).trim())
                .join("\n");
              if (!data) continue;
              const event: unknown = JSON.parse(data);
              if (
                !record(event) ||
                typeof event.sequence !== "number" ||
                !Number.isSafeInteger(event.sequence) ||
                event.project_id !== projectId ||
                typeof event.kind !== "string" ||
                typeof event.resource_id !== "string"
              )
                throw new Error("event schema invalid");
              if (after === undefined || event.sequence > after) {
                after = event.sequence;
                onEvent(event as ProjectEvent);
              }
            }
          }
        } finally {
          await reader.cancel().catch(() => {});
        }
      } catch {
        if (signal.aborted) return;
      }
      await new Promise<void>((resolve) => {
        const timer = setTimeout(done, 1500);
        function done() {
          clearTimeout(timer);
          signal.removeEventListener("abort", done);
          resolve();
        }
        signal.addEventListener("abort", done, { once: true });
        if (signal.aborted) done();
      });
    }
  }
}
