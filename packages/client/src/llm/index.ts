import type { EngineConnection, Schema } from "@studio/contracts";
import {
  LlmModelsClient,
  LlmPresetsClient,
  LlmProvidersClient,
  type Request,
} from "./configuration.js";
import { call, readEvents } from "./transport.js";
export { LlmCallError } from "./transport.js";
export type LlmInvocationInput = Omit<
  Schema["LlmInvocationRequest"],
  "invocation_id"
> & { invocation_id?: string };

export class LlmClient {
  readonly providers: LlmProvidersClient;
  readonly models: LlmModelsClient;
  readonly presets: LlmPresetsClient;
  private readonly active = new Set<AbortController>();
  constructor(
    private readonly request: Request,
    private readonly connection: () => EngineConnection,
  ) {
    this.providers = new LlmProvidersClient(request);
    this.models = new LlmModelsClient(request);
    this.presets = new LlmPresetsClient(request);
  }
  parameters(
    protocol: Schema["LlmProtocol"],
    kind: Schema["LlmProviderKind"],
    signal?: AbortSignal,
  ) {
    return this.request<Schema["LlmParameters"]>(
      "/v1/llm/parameters?" + new URLSearchParams({ protocol, kind }),
      { signal: signal ?? null },
    );
  }
  prepare(value: LlmInvocationInput, signal?: AbortSignal) {
    return this.request<Schema["LlmPrepared"]>("/v1/llm/prepare", {
      method: "POST",
      body: JSON.stringify({
        ...value,
        invocation_id: value.invocation_id ?? crypto.randomUUID(),
      }),
      signal: signal ?? null,
    });
  }
  cancel(id: string) {
    return this.request<Schema["OkResponse"]>(
      "/v1/llm/invocations/" + encodeURIComponent(id) + "/cancel",
      { method: "POST", keepalive: true },
    );
  }
  private context(id: string, signal?: AbortSignal) {
    const controller = new AbortController();
    this.active.add(controller);
    const abort = () => controller.abort();
    const remoteCancel = () => {
      void this.cancel(id).catch(() => {});
    };
    controller.signal.addEventListener("abort", remoteCancel, { once: true });
    signal?.addEventListener("abort", abort, { once: true });
    if (signal?.aborted) controller.abort();
    return {
      signal: controller.signal,
      close: () => {
        signal?.removeEventListener("abort", abort);
        controller.signal.removeEventListener("abort", remoteCancel);
        this.active.delete(controller);
      },
    };
  }
  async refreshModels(
    providerId: string,
    revision: number,
    signal?: AbortSignal,
  ) {
    const id = crypto.randomUUID(),
      context = this.context(id, signal);
    try {
      return (await (
        await call(
          this.connection(),
          "/v1/llm/providers/" +
            encodeURIComponent(providerId) +
            "/catalog/refresh",
          { invocation_id: id, expected_revision: revision },
          context.signal,
        )
      ).json()) as Schema["LlmCatalog"];
    } finally {
      context.close();
    }
  }
  async generate(
    value: LlmInvocationInput,
    signal?: AbortSignal,
  ): Promise<Schema["LlmResponse"]> {
    const id = value.invocation_id ?? crypto.randomUUID(),
      context = this.context(id, signal);
    try {
      return (await (
        await call(
          this.connection(),
          "/v1/llm/generate",
          { ...value, invocation_id: id },
          context.signal,
        )
      ).json()) as Schema["LlmResponse"];
    } finally {
      context.close();
    }
  }
  async *stream(
    value: LlmInvocationInput,
    signal?: AbortSignal,
  ): AsyncGenerator<Schema["LlmEvent"]> {
    const id = value.invocation_id ?? crypto.randomUUID(),
      context = this.context(id, signal);
    let terminal = false;
    try {
      const response = await call(
        this.connection(),
        "/v1/llm/stream",
        { ...value, invocation_id: id },
        context.signal,
      );
      for await (const event of readEvents(response, id)) {
        terminal = event.type === "completed" || event.type === "failed";
        yield event;
      }
    } finally {
      if (!terminal) void this.cancel(id).catch(() => {});
      context.close();
    }
  }
  dispose() {
    for (const controller of this.active) controller.abort();
    this.active.clear();
  }
}
