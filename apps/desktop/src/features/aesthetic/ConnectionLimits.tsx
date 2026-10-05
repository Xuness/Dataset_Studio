import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ErrorDetails } from "@studio/ui";
import type { ModuleContext } from "@studio/ui";

type Network = { max_concurrency: number; min_interval_ms: number };

/** Connection-wide limits also cap a stage; they are shared by every caller of the connection. */
export function ConnectionLimits({
  context,
  providerId,
  concurrency,
  disabled,
  onSaved,
}: {
  context: ModuleContext;
  providerId: string | undefined;
  concurrency: number;
  disabled: boolean;
  onSaved?: () => void;
}) {
  const { client } = context;
  const queryClient = useQueryClient();
  const providers = useQuery({
    queryKey: ["aesthetic", "connection-limits"],
    queryFn: ({ signal }) => client.llm.providers.list(signal),
    staleTime: 0,
    refetchOnMount: "always",
    enabled: !!providerId,
  });
  const [draft, setDraft] = useState<(Network & { id: string }) | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [saved, setSaved] = useState(false);
  if (!providerId) return null;
  if (providers.error) return <ErrorDetails error={providers.error} />;
  const provider = providers.data?.items.find((p) => p.id === providerId);
  if (!provider)
    return (
      <p className="aesthetic-help">
        {providers.isPending ? "正在读取连接限制…" : "未找到评审模型所属连接。"}
      </p>
    );
  const network: Network =
    draft?.id === provider.id
      ? draft
      : {
          max_concurrency: provider.config.network.max_concurrency,
          min_interval_ms: provider.config.network.min_interval_ms,
        };
  const changed =
    network.max_concurrency !== provider.config.network.max_concurrency ||
    network.min_interval_ms !== provider.config.network.min_interval_ms;
  const pacedRate =
    network.min_interval_ms > 0 ? 1000 / network.min_interval_ms : null;
  async function apply() {
    if (!provider) return;
    setBusy(true);
    setError(null);
    try {
      await client.llm.providers.save({
        id: provider.id,
        expected_revision: provider.revision,
        config: {
          ...provider.config,
          network: { ...provider.config.network, ...network },
        },
      });
      setDraft(null);
      setSaved(true);
      await providers.refetch();
      void queryClient.invalidateQueries({ queryKey: ["settings", "llm"] });
      onSaved?.();
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }
  return (
    <fieldset className="connection-limits" disabled={disabled || busy}>
      <legend>连接限制 · {provider.config.name}</legend>
      <label>
        连接最大并发
        <input
          aria-label="连接最大并发"
          type="number"
          min={1}
          max={1024}
          step={1}
          value={network.max_concurrency}
          onChange={(e) => {
            setSaved(false);
            setDraft({
              ...network,
              id: provider.id,
              max_concurrency: Number(e.target.value),
            });
          }}
        />
      </label>
      <label>
        请求最小间隔（毫秒）
        <input
          aria-label="请求最小间隔（毫秒）"
          type="number"
          min={0}
          max={60000}
          step={1}
          value={network.min_interval_ms}
          onChange={(e) => {
            setSaved(false);
            setDraft({
              ...network,
              id: provider.id,
              min_interval_ms: Number(e.target.value),
            });
          }}
        />
      </label>
      <p
        className={
          concurrency > network.max_concurrency
            ? "aesthetic-notice"
            : "aesthetic-help"
        }
      >
        实际在途请求取阶段并发 {concurrency} 与连接并发{" "}
        {network.max_concurrency} 的较小值
        {pacedRate != null &&
          `；最小间隔使派发速度不超过每秒 ${pacedRate.toFixed(pacedRate < 10 ? 1 : 0)} 个`}
        。连接设置由所有使用此连接的功能共享，修改会更新连接版本；已有阶段需重新保存执行设置以关联新版本。
      </p>
      {error != null && <ErrorDetails error={error} />}
      <div className="wb-dialog-actions">
        {saved && !changed && <span>已应用到连接</span>}
        <button type="button" disabled={!changed} onClick={() => void apply()}>
          应用到连接
        </button>
      </div>
    </fieldset>
  );
}
