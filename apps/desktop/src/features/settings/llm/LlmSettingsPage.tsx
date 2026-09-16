import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button, ErrorDetails } from "@studio/ui";
import type { SettingsPageProps } from "../types.js";
import { ConnectionEditor } from "./ConnectionEditor.js";
import { ModelEditor } from "./ModelEditor.js";
import { ProbePanel } from "./ProbePanel.js";
import {
  connectionDraft,
  modelDraft,
  providers as providerTypes,
  protocols,
} from "./types.js";
import "./llm.css";

export function LlmSettingsPage({
  client,
  busy,
  action,
  llmDraft: draft,
  setLlmDraft: setDraft,
}: SettingsPageProps) {
  const [search, setSearch] = useState(""),
    [probe, setProbe] = useState<string | null>(null),
    [shown, setShown] = useState(100);
  const providers = useQuery({
    queryKey: ["settings", "llm", "providers", client.connection.instance_id],
    queryFn: ({ signal }) => client.llm.providers.list(signal),
  });
  const provider =
    providers.data?.items.find((p) => p.id === draft.providerId) ??
    providers.data?.items[0];
  const models = useQuery({
    queryKey: [
      "settings",
      "llm",
      "models",
      client.connection.instance_id,
      provider?.id,
    ],
    queryFn: ({ signal }) => client.llm.models.list(provider!.id, signal),
    enabled: !!provider,
  });
  const catalog = useQuery({
    queryKey: [
      "settings",
      "llm",
      "catalog",
      client.connection.instance_id,
      provider?.id,
    ],
    queryFn: ({ signal }) => client.llm.models.catalog(provider!.id, signal),
    enabled: !!provider,
  });
  const editing = !!draft.connection || !!draft.model;
  const entries = (catalog.data?.catalog?.models ?? []).filter((m) =>
    (m.id + " " + m.name).toLowerCase().includes(search.toLowerCase()),
  );
  const probeModel = models.data?.items.find((m) => m.id === probe);
  return (
    <section className="settings-page llm-settings" aria-label="API 与模型设置">
      <header className="settings-page-heading">
        <h3>API 与模型</h3>
        <p>
          连接与模型配置在本机保存，供所有项目使用。每个模型可分别设置协议和调用参数。
        </p>
      </header>
      {providers.error && <ErrorDetails error={providers.error} />}
      <div className="settings-toolbar">
        <select
          aria-label="供应商连接"
          disabled={busy || editing}
          value={provider?.id ?? ""}
          onChange={(e) => {
            setDraft({ ...draft, providerId: e.target.value });
            setProbe(null);
            setShown(100);
          }}
        >
          <option value="" disabled>
            选择供应商连接
          </option>
          {providers.data?.items.map((p) => (
            <option key={p.id} value={p.id}>
              {p.config.name}
              {p.config.enabled ? "" : "（已停用）"}
            </option>
          ))}
        </select>
        <Button
          disabled={busy || editing}
          onClick={() => setDraft({ ...draft, connection: connectionDraft() })}
        >
          添加连接
        </Button>
        {provider && (
          <>
            <Button
              disabled={busy || editing}
              onClick={() =>
                setDraft({ ...draft, connection: connectionDraft(provider) })
              }
            >
              编辑连接
            </Button>
            <Button
              disabled={
                busy || editing || !models.data || !!models.data.items.length
              }
              onClick={() =>
                void action(async () => {
                  await client.llm.providers.remove(
                    provider.id,
                    provider.revision,
                  );
                  setDraft({ ...draft, providerId: null });
                }, "供应商连接已删除。")
              }
            >
              删除连接
            </Button>
          </>
        )}
      </div>
      {provider && !draft.connection && (
        <p className="settings-note">
          {providerTypes[provider.config.kind].label} ·{" "}
          {provider.config.base_url} ·{" "}
          {provider.credential_set ? "已保存凭据" : "未设置凭据"}
          {models.data?.items.length ? " · 删除连接前需先移除其模型配置" : ""}
        </p>
      )}
      {draft.connection && (
        <ConnectionEditor
          value={draft.connection}
          setValue={(connection) => setDraft({ ...draft, connection })}
          busy={busy}
          close={() => setDraft({ ...draft, connection: null })}
          save={() =>
            void action(async () => {
              const value = draft.connection!;
              const headers: unknown = JSON.parse(value.headers);
              if (
                !headers ||
                typeof headers !== "object" ||
                Array.isArray(headers) ||
                Object.values(headers).some((v) => typeof v !== "string")
              )
                throw new Error("附加请求头须为字符串值的 JSON 对象");
              const saved = await client.llm.providers.save({
                id: value.id,
                expected_revision: value.revision,
                config: {
                  ...value.config,
                  headers: headers as Record<string, string>,
                },
                api_key: value.apiKey || null,
                clear_credential: value.clearKey,
              });
              setDraft({ ...draft, providerId: saved.id, connection: null });
            }, "供应商连接已保存。")
          }
        />
      )}
      {provider && (
        <>
          <div className="settings-section">
            <h4>已保存的模型</h4>
            {models.error && <ErrorDetails error={models.error} />}
            <div className="settings-toolbar">
              <Button
                disabled={busy || editing}
                onClick={() =>
                  setDraft({ ...draft, model: modelDraft(provider) })
                }
              >
                手动添加模型
              </Button>
              <span>{models.data?.items.length ?? 0} 个模型配置</span>
            </div>
            <div className="llm-model-list">
              {models.data?.items.map((model) => (
                <div className="llm-model-row" key={model.id}>
                  <div>
                    <strong>
                      {model.config.name}
                      {model.config.enabled ? "" : "（已停用）"}
                    </strong>
                    <small>
                      {model.config.remote_model_id} ·{" "}
                      {protocols[model.config.protocol]}
                    </small>
                  </div>
                  <Button
                    disabled={busy || editing}
                    onClick={() =>
                      setDraft({ ...draft, model: modelDraft(provider, model) })
                    }
                  >
                    配置
                  </Button>
                  <Button
                    disabled={busy || editing}
                    onClick={() => setProbe(model.id)}
                  >
                    调用检查
                  </Button>
                  <Button
                    disabled={busy || editing}
                    onClick={() =>
                      void action(
                        () =>
                          client.llm.models.remove(model.id, model.revision),
                        "模型配置已删除。",
                      )
                    }
                  >
                    移除
                  </Button>
                </div>
              ))}
            </div>
          </div>
          {draft.model && (
            <ModelEditor
              key={draft.model.id ?? "new"}
              value={draft.model}
              setValue={(model) => setDraft({ ...draft, model })}
              provider={provider}
              client={client}
              busy={busy}
              action={action}
              close={() => setDraft({ ...draft, model: null })}
              saved={() => setDraft({ ...draft, model: null })}
            />
          )}
          {probeModel && !editing && (
            <ProbePanel
              key={
                probeModel.id +
                ":" +
                probeModel.revision +
                ":" +
                provider.revision
              }
              client={client}
              provider={provider}
              model={probeModel}
            />
          )}
          <div className="settings-section">
            <h4>供应商模型目录</h4>
            <p className="settings-note">
              获取目录不会发起生成请求，也不会修改已保存的模型参数。目录不可用时仍可手动添加模型。
            </p>
            {catalog.error && <ErrorDetails error={catalog.error} />}
            <div className="settings-toolbar">
              <Button
                disabled={busy || editing || !provider.config.enabled}
                onClick={() =>
                  void action(
                    () =>
                      client.llm.refreshModels(provider.id, provider.revision),
                    "模型目录已更新，已有模型配置保持不变。",
                  )
                }
              >
                获取模型
              </Button>
              <input
                aria-label="搜索远端模型"
                placeholder="搜索模型 ID 或名称"
                value={search}
                onChange={(e) => {
                  setSearch(e.target.value);
                  setShown(100);
                }}
              />
            </div>
            {catalog.data?.catalog && (
              <p className="settings-note">
                上次获取：
                {new Date(
                  Number(catalog.data.catalog.fetched_at),
                ).toLocaleString()}{" "}
                · {catalog.data.catalog.models.length} 个模型
                {catalog.data.catalog.provider_revision !== provider.revision
                  ? " · 连接已更新，此目录需要重新获取"
                  : ""}
              </p>
            )}
            <div className="llm-catalog">
              {entries.slice(0, shown).map((remote) => (
                <div className="llm-model-row" key={remote.id}>
                  <div>
                    <strong>{remote.name || remote.id}</strong>
                    <small>
                      {remote.id}
                      {remote.input_token_limit
                        ? " · 上下文 " + remote.input_token_limit
                        : ""}
                    </small>
                  </div>
                  <Button
                    disabled={busy || editing}
                    onClick={() =>
                      setDraft({
                        ...draft,
                        model: modelDraft(provider, undefined, remote),
                      })
                    }
                  >
                    添加配置
                  </Button>
                </div>
              ))}
            </div>
            {entries.length > shown && (
              <Button onClick={() => setShown((v) => v + 100)}>
                显示更多模型（剩余 {entries.length - shown}）
              </Button>
            )}
          </div>
        </>
      )}
    </section>
  );
}
