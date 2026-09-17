import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { RotateCw } from "lucide-react";
import { Button, Dialog, ErrorDetails } from "@studio/ui";
import type { StudioClient } from "@studio/client";
import type { Schema, Source } from "@studio/contracts";
import { settingsPages } from "./pages.js";
import type { SettingsPageId } from "./pages.js";
import "./settings.css";
import { emptyLlmDraft, type LlmSettingsDraft } from "./llm/types.js";
import {
  emptySystemPromptDraft,
  type SystemPromptDraft,
} from "./system-prompts/types.js";

export function SettingsDialog({
  client,
  initialPage,
  project,
  sources,
  activeResultId,
  onClose,
}: {
  client: StudioClient;
  initialPage: SettingsPageId;
  project: { id: string; name: string } | null;
  sources: Source[];
  activeResultId: string | null;
  onClose: () => void;
}) {
  const cache = useQueryClient();
  const [page, setPage] = useState(initialPage);
  const [llmDraft, setLlmDraft] = useState<LlmSettingsDraft>(emptyLlmDraft);
  const [systemPromptDraft, setSystemPromptDraft] = useState<SystemPromptDraft>(
    emptySystemPromptDraft,
  );
  const [cacheDraft, setCacheDraft] = useState<Schema["CacheSettings"] | null>(
    null,
  );
  const [memoryDraft, setMemoryDraft] = useState<string | null>(null);
  const [undoDraft, setUndoDraft] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const [error, setError] = useState<unknown>(null);
  const status = useQuery({
    queryKey: ["settings", "status", client.connection.instance_id],
    queryFn: ({ signal }) => client.settings.read(signal),
    refetchInterval: 2000,
  });
  async function action(run: () => Promise<unknown>, message: string) {
    if (busy) return;
    setBusy(true);
    setError(null);
    setNotice("正在提交操作…");
    try {
      await run();
      void Promise.all([
        cache.invalidateQueries({ queryKey: ["settings"] }),
        cache.invalidateQueries({ queryKey: ["resources"] }),
        ...(project
          ? [cache.invalidateQueries({ queryKey: ["project", project.id] })]
          : []),
      ]);
      setNotice(message);
    } catch (failure) {
      setError(failure);
    } finally {
      setBusy(false);
    }
  }
  const CurrentPage = settingsPages.find((item) => item.id === page)!.Component;
  const dirty =
    cacheDraft !== null ||
    memoryDraft !== null ||
    undoDraft !== null ||
    !!llmDraft.connection ||
    !!llmDraft.model ||
    !!systemPromptDraft.editor;
  return (
    <Dialog title="设置" onClose={onClose} className="settings-dialog">
      <div className="settings-layout">
        <nav className="settings-navigation" aria-label="设置分类">
          {settingsPages.map(({ id, title, Icon }) => (
            <button
              type="button"
              key={id}
              className={page === id ? "active" : ""}
              aria-current={page === id ? "page" : undefined}
              onClick={() => {
                setPage(id);
                setError(null);
                setNotice("");
              }}
            >
              <Icon size={16} />
              {title}
            </button>
          ))}
          <p>
            全局设置适用于所有项目。
            <br />
            查询成员由各项目管理。
          </p>
        </nav>
        <main className="settings-main">
          {error || status.error ? (
            <div className="settings-message">
              <ErrorDetails error={error || status.error} />
            </div>
          ) : null}
          {notice && (
            <p className="settings-message settings-success" role="status">
              {notice}
            </p>
          )}
          {status.data ? (
            <CurrentPage
              systemPromptDraft={systemPromptDraft}
              setSystemPromptDraft={setSystemPromptDraft}
              llmDraft={llmDraft}
              setLlmDraft={setLlmDraft}
              client={client}
              project={project}
              sources={sources}
              activeResultId={activeResultId}
              data={status.data}
              busy={busy}
              action={action}
              cacheDraft={cacheDraft}
              setCacheDraft={setCacheDraft}
              memoryDraft={memoryDraft}
              setMemoryDraft={setMemoryDraft}
              undoDraft={undoDraft}
              setUndoDraft={setUndoDraft}
            />
          ) : (
            <p className="settings-empty">正在读取设置…</p>
          )}
        </main>
      </div>
      <footer className="settings-footer">
        <span>
          {dirty ? "有尚未保存的修改" : "设置保存在本机，重启后仍有效"}
        </span>
        <span className="grow" />
        <Button
          disabled={busy || status.isFetching}
          onClick={() =>
            void cache.invalidateQueries({ queryKey: ["settings"] })
          }
        >
          <RotateCw size={13} />
          刷新状态
        </Button>
        <Button onClick={onClose}>关闭</Button>
      </footer>
    </Dialog>
  );
}
