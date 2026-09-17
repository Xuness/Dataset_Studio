import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button, ErrorDetails } from "@studio/ui";
import type { Schema } from "@studio/contracts";
import type { SettingsPageProps } from "../types.js";
import { promptEditor } from "./types.js";
import { SystemPromptEditor } from "./SystemPromptEditor.js";
import { systemPromptsQuery } from "./queries.js";
import "./system-prompts.css";

export function SystemPromptsPage({
  client,
  busy,
  action,
  systemPromptDraft: draft,
  setSystemPromptDraft: setDraft,
}: SettingsPageProps) {
  const cache = useQueryClient();
  const [removing, setRemoving] = useState<Schema["LlmSystemPrompt"] | null>(
    null,
  );
  const options = systemPromptsQuery(client);
  const { queryKey } = options;
  const prompts = useQuery(options);
  const items = [...(prompts.data?.items ?? [])].sort((a, b) =>
    a.config.name.localeCompare(b.config.name),
  );
  const selected =
    items.find((p) => p.id === draft.selectedId) ??
    (draft.selectedId ? undefined : items[0]);
  const filtered = items.filter((p) =>
    (p.config.name + " " + p.config.description)
      .toLocaleLowerCase()
      .includes(draft.search.toLocaleLowerCase()),
  );
  const editing = !!draft.editor;
  return (
    <section
      className="settings-page system-prompts"
      aria-label="System Prompt 预设设置"
    >
      <header className="settings-page-heading">
        <h3>System Prompt 预设</h3>
        <p>
          保存可复用的系统指令，由每次任务选择使用。User Prompt
          由任务提供，不在这里保存。
        </p>
      </header>
      {prompts.error && <ErrorDetails error={prompts.error} />}
      <div className="settings-toolbar">
        <input
          type="search"
          aria-label="搜索 System Prompt 预设"
          placeholder="搜索名称或备注"
          value={draft.search}
          onChange={(e) => setDraft({ ...draft, search: e.target.value })}
        />
        <Button
          disabled={
            busy ||
            editing ||
            !!removing ||
            !prompts.data ||
            items.length >= 128
          }
          onClick={() => setDraft({ ...draft, editor: promptEditor() })}
        >
          新建预设
        </Button>
        <span>{items.length} / 128 个预设</span>
      </div>
      {removing && (
        <div className="system-prompt-confirm" role="alert">
          <p>
            删除“{removing.config.name}
            ”？删除后新调用将无法选用此预设，已开始的调用保留原有内容。
          </p>
          <Button
            disabled={busy}
            onClick={() =>
              void action(async () => {
                await client.llm.systemPrompts.remove(
                  removing.id,
                  removing.revision,
                );
                cache.setQueryData<Schema["LlmSystemPrompts"]>(
                  queryKey,
                  (old) => ({
                    items: (old?.items ?? []).filter(
                      (p) => p.id !== removing.id,
                    ),
                  }),
                );
                setRemoving(null);
                setDraft({ ...draft, selectedId: null });
              }, "System Prompt 预设已删除。")
            }
          >
            确认删除预设
          </Button>
          <Button disabled={busy} onClick={() => setRemoving(null)}>
            取消删除
          </Button>
        </div>
      )}
      <div className="system-prompts-layout">
        <nav
          className="system-prompt-list"
          aria-label="已保存的 System Prompt 预设"
        >
          {filtered.map((p) => (
            <button
              key={p.id}
              type="button"
              disabled={busy || editing || !!removing}
              aria-current={selected?.id === p.id ? "true" : undefined}
              onClick={() => setDraft({ ...draft, selectedId: p.id })}
            >
              <strong>{p.config.name}</strong>
              <small>{p.config.description || "无备注"}</small>
            </button>
          ))}
          {!filtered.length && (
            <p className="settings-note">
              {prompts.isPending
                ? "正在读取预设…"
                : items.length
                  ? "没有匹配的预设"
                  : "还没有保存的预设"}
            </p>
          )}
        </nav>
        <div className="system-prompt-detail">
          {draft.editor ? (
            <SystemPromptEditor
              value={draft.editor}
              setValue={(editor) => setDraft({ ...draft, editor })}
              busy={busy}
              cancel={() => {
                setDraft({ ...draft, editor: null });
                void prompts.refetch();
              }}
              save={() =>
                void action(async () => {
                  const saved = await client.llm.systemPrompts.save(
                    draft.editor!,
                  );
                  cache.setQueryData<Schema["LlmSystemPrompts"]>(
                    queryKey,
                    (old) => ({
                      items: [
                        ...(old?.items ?? []).filter((p) => p.id !== saved.id),
                        saved,
                      ],
                    }),
                  );
                  setDraft({
                    ...draft,
                    selectedId: saved.id,
                    editor: null,
                    search: "",
                  });
                }, "System Prompt 预设已保存。")
              }
            />
          ) : selected ? (
            <>
              <h4>{selected.config.name}</h4>
              {selected.config.description && (
                <p className="settings-note">{selected.config.description}</p>
              )}
              <p className="settings-note">
                版本 {selected.revision} · 适用于所有供应商与模型
              </p>
              <div className="settings-toolbar">
                <Button
                  disabled={busy || !!removing}
                  onClick={() =>
                    setDraft({ ...draft, editor: promptEditor(selected) })
                  }
                >
                  编辑预设
                </Button>
                <Button
                  disabled={busy || !!removing || items.length >= 128}
                  onClick={() =>
                    setDraft({ ...draft, editor: promptEditor(selected, true) })
                  }
                >
                  复制预设
                </Button>
                <Button
                  disabled={busy || !!removing}
                  onClick={() => setRemoving(selected)}
                >
                  删除预设
                </Button>
              </div>
              <pre className="system-prompt-preview">
                {selected.config.text}
              </pre>
            </>
          ) : (
            <p className="settings-empty">
              {draft.selectedId
                ? "该预设已不存在，请选择其它预设。"
                : "新建预设，保存需要反复使用的系统指令。"}
            </p>
          )}
        </div>
      </div>
    </section>
  );
}
