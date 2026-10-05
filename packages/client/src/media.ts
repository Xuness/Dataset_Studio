import type { Asset, EngineConnection, Schema } from "@studio/contracts";
type Request = <T>(path: string, init?: RequestInit) => Promise<T>;
export type MediaOptions = {
  signal?: AbortSignal;
  priority?: "interactive" | "background" | "prefetch";
  maxSourceBytes?: number;
};
export type MediaHandle = {
  url: string;
  offline: boolean;
  verifiedMs: number;
  release: () => void;
};
type Cached = {
  url: string;
  bytes: number;
  refs: number;
  used: number;
  offline: boolean;
  verifiedMs: number;
  projectId: string;
  retired: boolean;
  checked: number;
};
type Pending = {
  promise: Promise<Cached>;
  abort: AbortController;
  consumers: Set<symbol>;
  done: boolean;
  projectId: string;
  requestId: string;
  cancelRequest?: Promise<unknown>;
};
const cancelled = () => new DOMException("预览读取已取消", "AbortError");
export class MediaClient {
  private cache = new Map<string, Cached>();
  private pending = new Map<string, Pending>();
  private cancelling = new Map<Promise<unknown>, string>();
  private disposed = false;
  constructor(
    private readonly connection: EngineConnection,
    private readonly request: Request,
  ) {}
  private stop(key: string, pending: Pending) {
    if (pending.cancelRequest) return pending.cancelRequest;
    if (pending.done) return Promise.resolve();
    pending.abort.abort();
    if (this.pending.get(key) === pending) this.pending.delete(key);
    // Abort alone does not prove that a detached HTTP handler stopped its work.
    const cancelledRequest = this.request(
      "/v1/projects/" +
        encodeURIComponent(pending.projectId) +
        "/read-requests/" +
        pending.requestId +
        "/cancel",
      { method: "POST", keepalive: true, signal: AbortSignal.timeout(1500) },
    )
      .catch(() => {})
      .finally(() => this.cancelling.delete(cancelledRequest));
    this.cancelling.set(cancelledRequest, pending.projectId);
    pending.cancelRequest = cancelledRequest;
    return pending.cancelRequest;
  }
  async clear(projectId?: string) {
    const stopped = [];
    for (const [key, pending] of this.pending)
      if (!projectId || pending.projectId === projectId)
        stopped.push(this.stop(key, pending));
    for (const [key, item] of this.cache)
      if (!projectId || item.projectId === projectId) {
        item.retired = true;
        if (!item.refs) URL.revokeObjectURL(item.url);
        this.cache.delete(key);
      }
    for (const [request, pid] of this.cancelling)
      if (!projectId || pid === projectId) stopped.push(request);
    await Promise.allSettled(stopped);
  }
  dispose() {
    this.disposed = true;
    void this.clear();
    for (const item of this.cache.values()) URL.revokeObjectURL(item.url);
    this.cache.clear();
  }
  async acquire(
    projectId: string,
    asset: Asset,
    edge: number,
    options: MediaOptions = {},
  ): Promise<MediaHandle> {
    if (this.disposed || options.signal?.aborted) throw cancelled();
    if (
      options.maxSourceBytes !== undefined &&
      (!Number.isSafeInteger(options.maxSourceBytes) ||
        options.maxSourceBytes < 0)
    )
      throw Object.assign(new Error("源读取预算必须是非负整数"), {
        code: "INVALID_INPUT",
      });
    const key = JSON.stringify([
      projectId,
      asset.key.source_id,
      asset.key.asset_id,
      edge,
    ]);
    const pendingKey =
      key +
      ":" +
      (options.priority ?? "interactive") +
      ":" +
      (options.maxSourceBytes ?? "default");
    let entry = this.cache.get(key);
    if (entry && Date.now() - entry.checked > 15_000) {
      entry.retired = true;
      if (!entry.refs) URL.revokeObjectURL(entry.url);
      this.cache.delete(key);
      entry = undefined;
    }
    let pending: Pending | undefined;
    const consumer = Symbol();
    let released = false;
    let held: Cached | undefined;
    let rejectAbort: ((error: DOMException) => void) | undefined;
    const aborted = new Promise<never>((_, reject) => {
      rejectAbort = reject;
    });
    const release = () => {
      if (released) return;
      released = true;
      options.signal?.removeEventListener("abort", onAbort);
      if (pending) {
        pending.consumers.delete(consumer);
        if (!pending.consumers.size && !pending.done)
          void this.stop(pendingKey, pending);
      }
      if (held) {
        held.refs = Math.max(0, held.refs - 1);
        if (!held.refs && held.retired) URL.revokeObjectURL(held.url);
      }
      this.evict();
    };
    const onAbort = () => {
      release();
      if (!held) rejectAbort?.(cancelled());
    };
    options.signal?.addEventListener("abort", onAbort, { once: true });
    try {
      if (!entry) {
        pending = this.pending.get(pendingKey);
        if (!pending) {
          if (this.pending.size >= 128)
            throw Object.assign(new Error("预览请求过多，请稍后重试"), {
              code: "READ_BUDGET_EXCEEDED",
            });
          const work: Pending = {
            promise: Promise.resolve(null as unknown as Cached),
            abort: new AbortController(),
            consumers: new Set(),
            done: false,
            projectId,
            requestId: crypto.randomUUID(),
          };
          const params = new URLSearchParams({
            edge: String(edge),
            request_id: work.requestId,
            priority: options.priority ?? "interactive",
          });
          if (options.maxSourceBytes !== undefined)
            params.set("max_source_bytes", String(options.maxSourceBytes));
          const path =
            "/v1/projects/" +
            encodeURIComponent(projectId) +
            "/sources/" +
            encodeURIComponent(asset.key.source_id) +
            "/assets/" +
            encodeURIComponent(asset.key.asset_id) +
            "/media?" +
            params;
          work.promise = (async () => {
            const response = await fetch(this.connection.endpoint + path, {
              headers: { Authorization: "Bearer " + this.connection.token },
              signal: work.abort.signal,
              cache: "no-store",
            });
            if (!response.ok) {
              const body = (await response.json().catch(() => ({}))) as {
                code?: string;
                message?: string;
              };
              throw Object.assign(new Error(body.message ?? "预览暂不可用"), {
                code: body.code ?? "MEDIA_UNAVAILABLE",
              });
            }
            const blob = await response.blob();
            if (this.disposed || work.abort.signal.aborted) throw cancelled();
            const existing = this.cache.get(key);
            if (existing) return existing;
            const value: Cached = {
              url: URL.createObjectURL(blob),
              bytes: blob.size,
              refs: 0,
              used: Date.now(),
              offline:
                response.headers.get("x-studio-freshness") === "offline_cached",
              verifiedMs: Number(
                response.headers.get("x-studio-verified-ms") ?? 0,
              ),
              projectId,
              retired: false,
              checked: Date.now(),
            };
            this.cache.set(key, value);
            return value;
          })().finally(() => {
            work.done = true;
            if (this.pending.get(pendingKey) === work)
              this.pending.delete(pendingKey);
          });
          pending = work;
          this.pending.set(pendingKey, work);
        }
        if (pending.consumers.size >= 256)
          throw Object.assign(new Error("同一预览的订阅已满"), {
            code: "READ_BUDGET_EXCEEDED",
          });
        pending.consumers.add(consumer);
        entry = await Promise.race([pending.promise, aborted]);
      }
      if (released || options.signal?.aborted) throw cancelled();
      held = entry;
      held.refs++;
      held.used = Date.now();
      this.evict();
      return {
        url: held.url,
        offline: held.offline,
        verifiedMs: held.verifiedMs,
        release,
      };
    } catch (error) {
      release();
      throw error;
    }
  }
  private evict() {
    let bytes = [...this.cache.values()].reduce(
      (sum, item) => sum + item.bytes,
      0,
    );
    for (const [key, item] of [...this.cache]
      .filter(([, item]) => !item.refs)
      .sort((a, b) => a[1].used - b[1].used)) {
      if (this.cache.size <= 96 && bytes <= 32 * 1024 * 1024) break;
      URL.revokeObjectURL(item.url);
      this.cache.delete(key);
      bytes -= item.bytes;
    }
  }
}
export class ResourceClient {
  constructor(
    private readonly request: Request,
    private readonly clearMemory: () => Promise<void>,
  ) {}
  status(signal?: AbortSignal) {
    return this.request<Schema["ReadServiceStatus"]>("/v1/resources", {
      signal: signal ?? null,
    });
  }
  configure(quotaMib: number) {
    return this.request<Schema["PreviewCacheStatus"]>("/v1/resources/cache", {
      method: "PUT",
      body: JSON.stringify({ quota_mib: quotaMib }),
    });
  }
  configureQuery(memoryGib: number) {
    return this.request<Schema["QueryResourceLimits"]>("/v1/resources/query", {
      method: "PUT",
      body: JSON.stringify({ memory_gib: memoryGib }),
    });
  }
  configureAesthetic(maxRunningStages: number) {
    return this.request<Schema["AestheticEngineStatus"]>(
      "/v1/resources/aesthetic",
      {
        method: "PUT",
        body: JSON.stringify({ max_running_stages: maxRunningStages }),
      },
    );
  }
  configureQueryCache(quotaMib: number, maxAgeDays: number) {
    return this.request<Schema["QueryCacheStatus"]>(
      "/v1/resources/query-cache",
      {
        method: "PUT",
        body: JSON.stringify({ quota_mib: quotaMib, max_age_days: maxAgeDays }),
      },
    );
  }
  clearQueryCache() {
    return this.request<Schema["QueryCacheStatus"]>(
      "/v1/resources/query-cache/clear",
      { method: "POST" },
    );
  }
  async clear() {
    await this.clearMemory();
    return this.request<Schema["PreviewCacheStatus"]>(
      "/v1/resources/cache/clear",
      { method: "POST" },
    );
  }
}
