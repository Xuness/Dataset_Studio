import type { Schema } from "@studio/contracts";
import type { DraftClient } from "./tools.js";

export type DraftStatus =
  | "loading"
  | "saved"
  | "dirty"
  | "saving"
  | "error"
  | "conflict"
  | "unsupported";
export type DraftSnapshot<T> = {
  value: T;
  status: DraftStatus;
  error: string;
  revision: number;
  dirty: boolean;
  raw: unknown;
};
type Identity = {
  projectId: string;
  moduleId: string;
  instanceId: string;
  schemaVersion: number;
};
type Port = Pick<DraftClient, "get" | "save">;
const message = (error: unknown) =>
  error instanceof Error ? error.message : String(error);
const conflict = (error: unknown) =>
  typeof error === "object" &&
  error !== null &&
  "code" in error &&
  error.code === "REVISION_CONFLICT";

/** One serial write stream per persistent identity. It outlives mounted views. */
export class DraftController<T> {
  private snapshot: DraftSnapshot<T>;
  private listeners = new Set<() => void>();
  private generation = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private pending: Promise<void> | undefined;
  private loaded: Promise<void>;
  constructor(
    readonly identity: Identity,
    private port: Port,
    private initial: T,
    private decode: (value: unknown) => T | null,
    private changed: () => void,
  ) {
    this.snapshot = {
      value: initial,
      status: "loading",
      error: "",
      revision: 0,
      dirty: false,
      raw: null,
    };
    this.loaded = this.load(false);
  }
  rebind(port: Port) {
    this.port = port;
  }
  getSnapshot = () => this.snapshot;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  get retained() {
    return (
      this.listeners.size > 0 ||
      this.snapshot.dirty ||
      this.snapshot.status === "loading"
    );
  }
  private emit(update: Partial<DraftSnapshot<T>>) {
    this.snapshot = { ...this.snapshot, ...update };
    for (const listener of this.listeners) listener();
    this.changed();
  }
  private async read() {
    const i = this.identity;
    return (await this.port.get(i.projectId, i.moduleId, i.instanceId)).draft;
  }
  private async load(notify = true) {
    if (notify) this.emit({ status: "loading", error: "" });
    try {
      const draft = await this.read();
      const decoded = draft ? this.decode(draft.value) : this.initial;
      if (
        draft &&
        (draft.schema_version !== this.identity.schemaVersion ||
          decoded === null)
      ) {
        this.emit({
          status: "unsupported",
          error: "已有草稿的格式与当前工具不兼容，原内容已保留。",
          raw: draft.value,
          revision: draft.revision,
          dirty: false,
        });
      } else {
        this.emit({
          value: decoded ?? this.initial,
          status: "saved",
          error: "",
          revision: draft?.revision ?? 0,
          raw: draft?.value ?? null,
          dirty: false,
        });
      }
    } catch (error) {
      this.emit({ status: "error", error: message(error) });
    }
  }
  set(update: T | ((value: T) => T)) {
    if (
      this.snapshot.status === "loading" ||
      this.snapshot.status === "unsupported" ||
      (this.snapshot.status === "error" && !this.snapshot.dirty)
    )
      throw new Error("草稿尚未成功读取，不能覆盖已有内容。");
    const value =
      typeof update === "function"
        ? (update as (value: T) => T)(this.snapshot.value)
        : update;
    const json = JSON.stringify(value);
    if (new TextEncoder().encode(json).byteLength > 65536)
      throw new Error("草稿超过 64 KiB，尚未接受本次编辑。");
    if (json === JSON.stringify(this.snapshot.value)) return;
    this.generation++;
    const conflicted = this.snapshot.status === "conflict";
    this.emit({
      value,
      dirty: true,
      status: conflicted ? "conflict" : "dirty",
      ...(conflicted ? {} : { error: "" }),
    });
    if (this.timer) clearTimeout(this.timer);
    if (!conflicted)
      this.timer = setTimeout(() => {
        this.timer = undefined;
        void this.flush().catch(() => {});
      }, 400);
  }
  async flush(): Promise<void> {
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = undefined;
    }
    await this.loaded;
    if (this.pending) return this.pending;
    if (!this.snapshot.dirty) return;
    if (this.snapshot.status === "conflict")
      throw new Error("草稿存在并发冲突，请先选择保留本地编辑或重新载入。");
    this.pending = (async () => {
      try {
        while (this.snapshot.dirty) {
          const generation = this.generation;
          const i = this.identity;
          const request: Schema["SaveDraft"] = {
            schema_version: i.schemaVersion,
            expected_revision: this.snapshot.revision,
            value: this.snapshot.value,
          };
          this.emit({ status: "saving", error: "" });
          const saved = await this.port.save(
            i.projectId,
            i.moduleId,
            i.instanceId,
            request,
          );
          const dirty = this.generation !== generation;
          this.emit({
            revision: saved.revision,
            dirty,
            status: dirty ? "dirty" : "saved",
            raw: saved.value,
          });
        }
      } catch (error) {
        this.emit({
          status: conflict(error) ? "conflict" : "error",
          error: message(error),
        });
        throw error;
      }
    })().finally(() => {
      this.pending = undefined;
    });
    return this.pending;
  }
  async reload() {
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = undefined;
    }
    await this.pending?.catch(() => {});
    this.generation++;
    this.loaded = this.load();
    await this.loaded;
  }
  async keepLocal() {
    await this.pending?.catch(() => {});
    const latest = await this.read();
    if (latest && latest.schema_version !== this.identity.schemaVersion)
      throw new Error("远端草稿格式不兼容，不能覆盖。");
    this.emit({
      revision: latest?.revision ?? 0,
      status: "dirty",
      error: "",
      dirty: true,
    });
    await this.flush();
  }
}

export class DraftCoordinator {
  private controllers = new Map<string, DraftController<unknown>>();
  private listeners = new Set<() => void>();
  private version = 0;
  constructor(private port: DraftClient) {}
  private adapter(identity: Identity): Port {
    if (identity.projectId !== "$application") return this.port;
    const wrap = (preference: Schema["Preference"]): Schema["Draft"] => ({
      project_id: "$application",
      module_id: identity.moduleId,
      instance_id: "default",
      updated_at: "not_applicable",
      schema_version: preference.schema_version,
      revision: preference.revision,
      value: preference.value,
    });
    return {
      get: async () => {
        const value = (await this.port.preference(identity.moduleId))
          .preference;
        return { draft: value ? wrap(value) : null };
      },
      save: async (_pid, _module, _instance, request) =>
        wrap(await this.port.savePreference(identity.moduleId, request)),
    };
  }
  rebind(port: DraftClient) {
    this.port = port;
    for (const controller of this.controllers.values())
      controller.rebind(this.adapter(controller.identity));
  }
  preference<T>(key: string, initial: T, decode: (value: unknown) => T | null) {
    return this.open(
      {
        projectId: "$application",
        moduleId: key,
        instanceId: "default",
        schemaVersion: 1,
      },
      initial,
      decode,
    );
  }
  getSnapshot = () => this.version;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private changed = () => {
    this.version++;
    for (const listener of this.listeners) listener();
  };
  open<T>(
    identity: Identity,
    initial: T,
    decode: (value: unknown) => T | null,
  ): DraftController<T> {
    const key = JSON.stringify([
      identity.projectId,
      identity.moduleId,
      identity.instanceId,
    ]);
    const old = this.controllers.get(key);
    if (old) {
      if (old.identity.schemaVersion !== identity.schemaVersion)
        throw new Error("同一草稿身份不能同时使用不同模式版本。");
      return old as DraftController<T>;
    }
    if (this.controllers.size >= 128) {
      for (const [id, controller] of this.controllers) {
        if (!controller.retained) this.controllers.delete(id);
        if (this.controllers.size < 96) break;
      }
      if (this.controllers.size >= 128)
        throw new Error("打开的草稿过多，请先保存并关闭部分工具。");
    }
    const controller = new DraftController(
      identity,
      this.adapter(identity),
      initial,
      decode,
      this.changed,
    );
    this.controllers.set(key, controller as DraftController<unknown>);
    return controller;
  }
  status(projectId?: string) {
    const values = [...this.controllers.values()]
      .filter((c) => !projectId || c.identity.projectId === projectId)
      .map((c) => c.getSnapshot());
    return {
      dirty: values.some((v) => v.dirty),
      saving: values.some((v) => v.status === "saving"),
      error:
        values.find((v) => v.dirty && ["conflict", "error"].includes(v.status))
          ?.error ?? "",
    };
  }
  async flush(projectId?: string) {
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      await Promise.race([
        Promise.allSettled(
          [...this.controllers.values()]
            .filter((c) => !projectId || c.identity.projectId === projectId)
            .map((c) => c.flush()),
        ).then((results) => {
          const failure = results.find((r) => r.status === "rejected");
          if (failure?.status === "rejected") throw failure.reason;
        }),
        new Promise<void>((_, reject) => {
          timer = setTimeout(
            () =>
              reject(
                new Error("草稿保存尚未完成，窗口保持打开，请检查连接后重试。"),
              ),
            8000,
          );
        }),
      ]);
    } finally {
      if (timer) clearTimeout(timer);
    }
  }
}
