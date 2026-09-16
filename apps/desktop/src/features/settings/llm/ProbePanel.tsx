import { useEffect, useRef, useState } from "react";
import { Button, ErrorDetails } from "@studio/ui";
import { LlmCallError, type StudioClient } from "@studio/client";
import type { Schema } from "@studio/contracts";
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
    [text, setText] = useState(""),
    [error, setError] = useState<unknown>(null),
    [result, setResult] = useState<Schema["LlmResponse"] | null>(null);
  const controller = useRef<AbortController | null>(null);
  useEffect(() => () => controller.current?.abort(), []);
  async function run() {
    const cancel = new AbortController();
    controller.current = cancel;
    setBusy(true);
    setText("");
    setError(null);
    setResult(null);
    try {
      for await (const event of client.llm.stream(
        {
          model_id: model.id,
          expected_model_revision: model.revision,
          expected_provider_revision: provider.revision,
          preset_id: null,
          expected_preset_revision: null,
          overrides: {},
          messages: [
            {
              role: "user",
              content: [{ type: "text", text: "Reply with OK." }],
            },
          ],
          tools: [],
        },
        cancel.signal,
      )) {
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
        向已保存的模型发送一条固定测试消息，使用其当前参数。此操作会调用供应商并可能产生费用。
      </p>
      <div className="settings-toolbar">
        <Button
          disabled={busy || !model.config.enabled || !provider.config.enabled}
          onClick={() => void run()}
        >
          发送测试请求
        </Button>
        {busy && (
          <Button onClick={() => controller.current?.abort()}>
            取消测试请求
          </Button>
        )}
      </div>
      {!!error && <ErrorDetails error={error} />} {text && <pre>{text}</pre>}
      {result && (
        <>
          <p role="status">
            调用完成 · 输入 {result.usage.input_tokens ?? "未知"} / 输出{" "}
            {result.usage.output_tokens ?? "未知"} Token ·{" "}
            {result.outputs
              .map((o) => o.finish_reason ?? "未知结束原因")
              .join("、")}
          </p>
          <details>
            <summary>本次参数与能力提示</summary>
            <pre>
              {JSON.stringify(
                {
                  parameters: result.snapshot.parameters,
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
