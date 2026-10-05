import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button, ErrorDetails } from "@studio/ui";
import {
  LlmCallError,
  type StudioClient,
  type LlmInvocationInput,
} from "@studio/client";
import type { Schema } from "@studio/contracts";
import { systemPromptsQuery } from "../system-prompts/queries.js";
export function ProbePanel({
  client,
  provider,
  model,
}: {
  client: StudioClient;
  provider: Schema["LlmProviderView"];
  model: Schema["LlmModel"];
}) {
  const [busy, setBusy] = useState(false),
    [promptId, setPromptId] = useState(""),
    [userText, setUserText] = useState("Reply with OK."),
    [prepared, setPrepared] = useState<{
      input: LlmInvocationInput;
      value: Schema["LlmPrepared"];
    } | null>(null),
    [text, setText] = useState(""),
    [error, setError] = useState<unknown>(null),
    [result, setResult] = useState<Schema["LlmResponse"] | null>(null);
  const [prefixProbe, setPrefixProbe] = useState(false);
  const controller = useRef<AbortController | null>(null);
  useEffect(() => () => controller.current?.abort(), []);
  const prompts = useQuery(systemPromptsQuery(client));
  function clearOutput() {
    setPrepared(null);
    setText("");
    setResult(null);
    setError(null);
    setPrefixProbe(false);
  }
  function input(): LlmInvocationInput {
    const prompt = prompts.data?.items.find((p) => p.id === promptId);
    if (promptId && !prompt)
      throw new Error("所选 System Prompt 预设已不存在，请重新选择。");
    return {
      model_id: model.id,
      expected_model_revision: model.revision,
      expected_provider_revision: provider.revision,
      system_prompt_id: prompt?.id ?? null,
      expected_system_prompt_revision: prompt?.revision ?? null,
      messages: [{ role: "user", content: [{ type: "text", text: userText }] }],
      tools: [],
      overrides: {},
    };
  }
  async function preview() {
    const cancel = new AbortController();
    controller.current = cancel;
    setBusy(true);
    setError(null);
    setPrepared(null);
    try {
      const request = input();
      setPrepared({
        input: request,
        value: await client.llm.prepare(request, cancel.signal),
      });
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
      controller.current = null;
    }
  }
  async function run(prefixOnly = false) {
    const cancel = new AbortController();
    controller.current = cancel;
    setBusy(true);
    setText("");
    setError(null);
    setResult(null);
    setPrefixProbe(prefixOnly);
    if (prefixOnly) setPrepared(null);
    try {
      const request = {
        ...(prefixOnly ? input() : (prepared?.input ?? input())),
      };
      if (prefixOnly) {
        request.messages = [
          { role: "user", content: [{ type: "text", text: "Reply with OK." }] },
        ];
        request.overrides = {
          max_output_tokens: 64,
          ...(provider.config.kind === "openrouter"
            ? {
                "openrouter.cache_strategy": "implicit",
                stream_usage: true,
              }
            : {}),
        };
      } else if (provider.config.kind === "openrouter") {
        request.overrides = { ...request.overrides, stream_usage: true };
      }
      for await (const event of client.llm.stream(request, cancel.signal)) {
        if (event.type === "delta" && event.kind === "text")
          setText((t) => t + event.text);
        if (event.type === "completed") setResult(event.response);
        if (event.type === "failed") throw new LlmCallError(event.error);
      }
    } catch (e) {
      setError(
        cancel.signal.aborted
          ? new Error("已取消本次调用；已发送的请求可能仍由供应商计费。")
          : e,
      );
    } finally {
      setBusy(false);
      controller.current = null;
    }
  }
  return (
    <div className="llm-probe">
      <h4>调用检查：{model.config.name}</h4>
      <p className="settings-note">
        使用已保存模型的参数与可选 System
        Prompt。测试消息仅用于本次检查，不会保存。发送测试请求会调用供应商并可能产生费用。
      </p>
      {prompts.error && <ErrorDetails error={prompts.error} />}
      <label className="llm-probe-field">
        System Prompt 预设
        <select
          aria-label="测试 System Prompt 预设"
          disabled={busy || prompts.isPending}
          value={promptId}
          onChange={(e) => {
            setPromptId(e.target.value);
            clearOutput();
          }}
        >
          <option value="">不使用预设</option>
          {promptId &&
            prompts.data &&
            !prompts.data.items.some((p) => p.id === promptId) && (
              <option value={promptId}>所选预设已不存在</option>
            )}
          {prompts.data?.items.map((p) => (
            <option key={p.id} value={p.id}>
              {p.config.name} · v{p.revision}
            </option>
          ))}
        </select>
      </label>
      <label className="llm-probe-field">
        测试消息（仅本次使用）
        <textarea
          aria-label="测试消息"
          rows={3}
          value={userText}
          disabled={busy}
          onChange={(e) => {
            setUserText(e.target.value);
            clearOutput();
          }}
        />
      </label>
      <div className="settings-toolbar">
        <Button
          disabled={
            busy ||
            !userText.trim() ||
            !model.config.enabled ||
            !provider.config.enabled
          }
          onClick={() => void preview()}
        >
          预览请求（不联网）
        </Button>
        <Button
          disabled={
            busy ||
            !userText.trim() ||
            !model.config.enabled ||
            !provider.config.enabled
          }
          onClick={() => void run()}
        >
          发送测试请求
        </Button>
        {provider.config.kind === "openrouter" && (
          <Button
            disabled={
              busy ||
              !promptId ||
              !model.config.enabled ||
              !provider.config.enabled
            }
            onClick={() => void run(true)}
          >
            测量 System 输入（1 次调用）
          </Button>
        )}
        {busy && (
          <Button onClick={() => controller.current?.abort()}>
            取消测试请求
          </Button>
        )}
      </div>
      {!!error && <ErrorDetails error={error} />} {text && <pre>{text}</pre>}
      {prepared && (
        <details className="llm-request-preview" open>
          <summary>原生请求预览</summary>
          <p className="settings-note">
            预览展示消息与参数；测试请求使用流式传输。
          </p>
          <pre>{JSON.stringify(prepared.value.native_request, null, 2)}</pre>
        </details>
      )}
      {result && (
        <>
          <p role="status">
            调用完成 · 输入 {result.usage.input_tokens ?? "未知"} / 输出{" "}
            {result.usage.output_tokens ?? "未知"} Token ·{" "}
            {result.outputs
              .map((o) => o.finish_reason ?? "未知结束原因")
              .join("、")}
          </p>
          <p className="settings-note">
            缓存读取 {result.usage.cached_input_tokens ?? "未知"} / 写入{" "}
            {result.usage.cache_write_tokens ?? "未知"} Token
            {` · 费用 ${result.usage.cost_usd != null ? `$${result.usage.cost_usd.toFixed(6)}` : "未知"}`}
            {` · 上游 ${result.usage.upstream_provider ?? "未知"} · 实际层级 ${result.usage.service_tier ?? "未知"}`}
          </p>
          {prefixProbe && (
            <p className="settings-note">
              本次仅发送所选 System Prompt 和测试短句，最多输出 64
              tokens，使用隐式缓存。 输入 {result.usage.input_tokens ?? "未知"}{" "}
              tokens 为上游实测值，包含短句和消息封装，可用于核对 System
              缓存最低长度；不是纯 System 的精确分词数。
            </p>
          )}
          <details>
            <summary>本次消息、参数与能力提示</summary>
            <pre>
              {JSON.stringify(
                {
                  parameters: result.snapshot.parameters,
                  system_prompt_id: result.snapshot.system_prompt_id,
                  system_prompt_revision:
                    result.snapshot.system_prompt_revision,
                  messages: result.snapshot.messages,
                  warnings: result.snapshot.warnings,
                },
                null,
                2,
              )}
            </pre>
          </details>
        </>
      )}
    </div>
  );
}
