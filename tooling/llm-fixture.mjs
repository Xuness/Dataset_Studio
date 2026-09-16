import { createServer } from "node:http";
import { setTimeout as sleep } from "node:timers/promises";

export const network = {
  connect_timeout_ms: 3000,
  request_timeout_ms: 10000,
  idle_timeout_ms: 3000,
  max_concurrency: 2,
  min_interval_ms: 0,
  rate_limit_retries: 0,
  proxy_url: "",
};
export async function llmFixture() {
  const state = {
    calls: [],
    catalogFailure: false,
    catalogDelay: 0,
    active: 0,
    peak: 0,
    attempts: new Map(),
    closed: 0,
  };
  const server = createServer(async (request, response) => {
    response.on("error", () => {});
    try {
      const url = new URL(request.url, "http://127.0.0.1");
      const chunks = [];
      for await (const chunk of request) chunks.push(chunk);
      const body = chunks.length ? JSON.parse(Buffer.concat(chunks)) : {};
      state.calls.push({
        path: url.pathname,
        query: Object.fromEntries(url.searchParams),
        body,
        authenticated:
          !!request.headers.authorization ||
          !!request.headers["x-goog-api-key"],
        geminiKey: !!request.headers["x-goog-api-key"],
      });
      const json = (value, status = 200) => {
        response.writeHead(status, {
          "content-type": "application/json",
          "x-request-id": "fixture-upstream-id",
        });
        response.end(JSON.stringify(value));
      };
      if (url.pathname.endsWith("/models")) {
        await sleep(state.catalogDelay);
        if (state.catalogFailure)
          return json({ error: { message: "catalog unavailable" } }, 503);
        if (url.pathname.startsWith("/gemini"))
          return json({
            models: [
              {
                name: "models/text-model",
                displayName: "文本模型",
                inputTokenLimit: 8192,
                outputTokenLimit: 4096,
                temperature: 1,
                topP: 0.9,
              },
            ],
            ...(url.searchParams.has("pageToken")
              ? {}
              : { nextPageToken: "next-page" }),
          });
        return json({
          data: [
            {
              id: "text-model",
              name: "文本模型",
              supported_parameters: ["temperature", "max_tokens"],
              architecture: { input_modalities: ["text", "image"] },
            },
            { id: "other-model", name: "另一个模型" },
          ],
        });
      }
      const model =
        body.model ?? url.pathname.split("/models/")[1]?.split(":")[0];
      const attempts = (state.attempts.get(model) ?? 0) + 1;
      state.attempts.set(model, attempts);
      if (model === "error-model")
        return json({ error: { message: "failure" } }, 500);
      if (model === "rate-model" && attempts === 1) {
        response.setHeader("retry-after", "1");
        return json({ error: { message: "rate limited" } }, 429);
      }
      state.active++;
      state.peak = Math.max(state.peak, state.active);
      response.once("close", () => {
        state.active--;
        state.closed++;
      });
      if (model === "slow-model") await sleep(500);
      if (response.destroyed) return;
      const isGemini = url.pathname.startsWith("/gemini"),
        isResponses = url.pathname.endsWith("/responses");
      const usage = {
        prompt_tokens: 7,
        completion_tokens: 3,
        total_tokens: 10,
      };
      const complete = isGemini
        ? {
            responseId: "g-response",
            modelVersion: model,
            candidates: [
              {
                index: 0,
                content: { parts: [{ text: "你好 OK" }] },
                finishReason: "STOP",
              },
            ],
            usageMetadata: {
              promptTokenCount: 7,
              candidatesTokenCount: 3,
              totalTokenCount: 10,
            },
          }
        : isResponses
          ? {
              id: "r-response",
              model,
              status: "completed",
              error: null,
              output: [
                {
                  type: "message",
                  content: [{ type: "output_text", text: "你好 OK" }],
                },
              ],
              usage: { input_tokens: 7, output_tokens: 3, total_tokens: 10 },
            }
          : {
              id: "c-response",
              model,
              choices: [
                {
                  index: 0,
                  message: { content: "你好 OK" },
                  finish_reason: "stop",
                },
              ],
              usage,
            };
      if (!body.stream && !url.pathname.includes("streamGenerateContent"))
        return json(complete);
      response.writeHead(200, {
        "content-type": "text/event-stream",
        "x-request-id": "fixture-upstream-id",
      });
      const events = isGemini
        ? [complete]
        : isResponses
          ? [
              { type: "response.output_text.delta", delta: "你好 " },
              { type: "response.output_text.delta", delta: "OK" },
              { type: "response.completed", response: complete },
            ]
          : [
              {
                id: "c-response",
                model,
                choices: [{ index: 0, delta: { content: "你好 " } }],
              },
              {
                choices: [
                  { index: 0, delta: { content: "OK" }, finish_reason: "stop" },
                ],
              },
              { choices: [], usage },
              "[DONE]",
            ];
      const selected = model === "stream-break" ? events.slice(0, 1) : events;
      if (model === "stream-error")
        selected.splice(1, selected.length, { error: { code: "failure" } });
      for (const event of selected) {
        const bytes = Buffer.from(
          ": ping\r\ndata: " +
            (typeof event === "string" ? event : JSON.stringify(event)) +
            "\r\n\r\n",
        );
        // Deliberately fragment UTF-8 and SSE boundaries.
        for (let i = 0; i < bytes.length; i += 7) {
          if (response.destroyed) return;
          response.write(bytes.subarray(i, i + 7));
          await sleep(1);
        }
      }
      response.end();
    } catch {
      if (!response.destroyed) {
        response.statusCode = 500;
        response.end();
      }
    }
  });
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  return {
    state,
    url: "http://127.0.0.1:" + server.address().port,
    close: async () => {
      server.closeAllConnections();
      await new Promise((done) => server.close(done));
    },
  };
}
