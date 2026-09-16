import type { EngineConnection, Schema } from "@studio/contracts";

export class LlmCallError extends Error {
  constructor(public readonly detail: Schema["LlmFailure"]) {
    super(
      detail.message +
        (detail.outcome_unknown
          ? "；供应商可能已处理请求，请勿直接重复提交。"
          : ""),
    );
    this.name = "LlmCallError";
  }
  get code() {
    return this.detail.code;
  }
}

export async function call(
  connection: EngineConnection,
  path: string,
  value: unknown,
  signal: AbortSignal,
): Promise<Response> {
  const response = await fetch(connection.endpoint + path, {
    method: "POST",
    signal,
    headers: {
      Authorization: "Bearer " + connection.token,
      "Content-Type": "application/json",
    },
    body: JSON.stringify(value),
  });
  if (!response.ok) {
    const value = (await response.json().catch(() => null)) as Partial<
      Schema["LlmFailure"]
    > | null;
    throw new LlmCallError({
      code: value?.code ?? "LLM_HTTP_ERROR",
      message: value?.message ?? "本机调用接口返回错误",
      http_status: value?.http_status ?? response.status,
      provider_request_id: value?.provider_request_id ?? null,
      retryable: value?.retryable ?? false,
      outcome_unknown: value?.outcome_unknown ?? false,
    });
  }
  return response;
}

function event(value: unknown, id: string): Schema["LlmEvent"] {
  if (!value || typeof value !== "object" || !("type" in value))
    throw new Error("无效的 LLM 事件");
  const v = value as Record<string, unknown>;
  if (v.type === "started" && v.invocation_id === id)
    return v as Schema["LlmEvent"];
  if (
    v.type === "delta" &&
    typeof v.text === "string" &&
    typeof v.kind === "string" &&
    Number.isInteger(v.index)
  )
    return v as Schema["LlmEvent"];
  if (v.type === "completed" && v.response && typeof v.response === "object") {
    const r = v.response as Schema["LlmResponse"];
    if (r.snapshot?.invocation_id === id && Array.isArray(r.outputs) && r.usage)
      return v as Schema["LlmEvent"];
  }
  if (
    v.type === "failed" &&
    v.error &&
    typeof v.error === "object" &&
    "code" in v.error &&
    "message" in v.error
  )
    return v as Schema["LlmEvent"];
  throw new Error("LLM 事件与当前调用不匹配");
}

export async function* readEvents(
  response: Response,
  id: string,
): AsyncGenerator<Schema["LlmEvent"]> {
  if (
    !response.headers.get("content-type")?.startsWith("text/event-stream") ||
    !response.body
  )
    throw new Error("本机引擎未返回事件流");
  const reader = response.body.getReader();
  const decoder = new TextDecoder("utf-8", { fatal: true });
  let buffer = "",
    data: string[] = [],
    terminal = false,
    total = 0;
  try {
    while (!terminal) {
      const { done, value } = await reader.read();
      if (done) {
        buffer += decoder.decode();
        break;
      }
      total += value.length;
      if (total > 128 * 1024 * 1024) throw new Error("本机事件流超出缓冲预算");
      buffer += decoder.decode(value, { stream: true });
      let end: number;
      while ((end = buffer.indexOf("\n")) >= 0) {
        const line = buffer.slice(0, end).replace(/\r$/, "");
        buffer = buffer.slice(end + 1);
        if (!line) {
          if (data.length) {
            const parsed = event(JSON.parse(data.join("\n")), id);
            data = [];
            terminal = parsed.type === "completed" || parsed.type === "failed";
            yield parsed;
            if (terminal) break;
          }
        } else if (line.startsWith("data:"))
          data.push(line.slice(5).replace(/^ /, ""));
      }
      if (
        buffer.length + data.reduce((n, s) => n + s.length, 0) >
        64 * 1024 * 1024
      )
        throw new Error("本机流事件超过大小上限");
    }
    if (!terminal)
      throw new LlmCallError({
        code: "LLM_STREAM_INTERRUPTED",
        message: "本机连接在调用完成前中断",
        http_status: null,
        provider_request_id: null,
        retryable: false,
        outcome_unknown: true,
      });
  } finally {
    await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
}
