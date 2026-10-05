import { Button, Field } from "@studio/ui";
import type { Schema } from "@studio/contracts";
import { providers, type ConnectionDraft } from "./types.js";
export function ConnectionEditor({
  value,
  setValue,
  busy,
  save,
  close,
}: {
  value: ConnectionDraft;
  setValue: (v: ConnectionDraft) => void;
  busy: boolean;
  save: () => void;
  close: () => void;
}) {
  const config = value.config;
  const set = (patch: Partial<typeof config>) =>
    setValue({ ...value, config: { ...config, ...patch } });
  return (
    <form
      className="llm-editor"
      onSubmit={(e) => {
        e.preventDefault();
        save();
      }}
    >
      <h4>{value.id ? "编辑供应商连接" : "添加供应商连接"}</h4>
      <fieldset disabled={busy}>
        <div className="llm-form-grid">
          <Field label="连接名称">
            <input
              aria-label="连接名称"
              required
              value={config.name}
              onChange={(e) => set({ name: e.target.value })}
            />
          </Field>
          <Field label="服务类型">
            <select
              aria-label="服务类型"
              value={config.kind}
              onChange={(e) => {
                const kind = e.target.value as Schema["LlmProviderKind"];
                set({ kind, base_url: providers[kind].url });
              }}
            >
              {Object.entries(providers).map(([key, p]) => (
                <option value={key} key={key}>
                  {p.label}
                </option>
              ))}
            </select>
          </Field>
        </div>
        <Field label="API 基础地址">
          <input
            aria-label="API 基础地址"
            required
            type="url"
            value={config.base_url}
            onChange={(e) => set({ base_url: e.target.value })}
          />
        </Field>
        <p className="settings-note">
          填写版本根地址，例如
          https://api.openai.com/v1。请求接口路径由所选协议处理。
        </p>
        <Field label="API Key">
          <input
            aria-label="API Key"
            type="password"
            autoComplete="new-password"
            value={value.apiKey}
            placeholder={
              value.id ? "留空保留已保存的凭据" : "无认证的本地服务可留空"
            }
            onChange={(e) =>
              setValue({ ...value, apiKey: e.target.value, clearKey: false })
            }
          />
        </Field>
        {value.id && (
          <label className="llm-check">
            <input
              type="checkbox"
              checked={value.clearKey}
              onChange={(e) =>
                setValue({ ...value, clearKey: e.target.checked, apiKey: "" })
              }
            />
            清除已保存的凭据
          </label>
        )}
        <label className="llm-check">
          <input
            type="checkbox"
            checked={config.enabled}
            onChange={(e) => set({ enabled: e.target.checked })}
          />
          启用此连接
        </label>
        <details>
          <summary>网络与高级设置</summary>
          <div className="llm-form-grid">
            {(
              [
                ["connect_timeout_ms", "连接超时（毫秒）", 100, 120000],
                ["request_timeout_ms", "调用总时限（毫秒）", 100, 3600000],
                ["idle_timeout_ms", "响应空闲超时（毫秒）", 100, 600000],
                ["max_concurrency", "最大并发", 1, 1024],
                ["min_interval_ms", "请求最小间隔（毫秒）", 0, 60000],
                ["rate_limit_retries", "429 最大重试次数", 0, 3],
              ] as const
            ).map(([key, label, min, max]) => (
              <Field label={label} key={key}>
                <input
                  aria-label={label}
                  type="number"
                  required
                  min={min}
                  max={max}
                  step={1}
                  value={config.network[key]}
                  onChange={(e) =>
                    set({
                      network: {
                        ...config.network,
                        [key]:
                          e.target.value === "" ? min : Number(e.target.value),
                      },
                    })
                  }
                />
              </Field>
            ))}
          </div>
          <Field label="代理模式">
            <select
              aria-label="代理模式"
              value={
                config.network.proxy_url === null ||
                config.network.proxy_url === undefined
                  ? "system"
                  : config.network.proxy_url === ""
                    ? "direct"
                    : "custom"
              }
              onChange={(e) =>
                set({
                  network: {
                    ...config.network,
                    proxy_url:
                      e.target.value === "system"
                        ? null
                        : e.target.value === "direct"
                          ? ""
                          : "http://127.0.0.1:7890",
                  },
                })
              }
            >
              <option value="system">使用进程代理环境</option>
              <option value="direct">直连</option>
              <option value="custom">指定 HTTP(S) 代理</option>
            </select>
          </Field>
          {!!config.network.proxy_url && (
            <Field label="代理地址">
              <input
                type="url"
                value={config.network.proxy_url}
                onChange={(e) =>
                  set({
                    network: { ...config.network, proxy_url: e.target.value },
                  })
                }
              />
            </Field>
          )}
          <Field label="附加请求头（JSON）">
            <textarea
              rows={4}
              value={value.headers}
              onChange={(e) => setValue({ ...value, headers: e.target.value })}
            />
          </Field>
          <p className="settings-note">
            附加请求头用于非敏感设置。API Key 单独保存。仅明确的 429
            响应按设置重试；已发送后超时和流中断不自动重发。
          </p>
        </details>
        <div className="settings-page-actions">
          <Button type="button" onClick={close}>
            取消编辑连接
          </Button>
          <Button type="submit">保存连接</Button>
        </div>
      </fieldset>
    </form>
  );
}
