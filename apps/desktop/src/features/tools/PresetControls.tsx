import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button, Dialog, ErrorDetails, Field, MoreMenu } from "@studio/ui";
import type { StudioClient } from "@studio/client";
import type { OperatorRun, ToolPreset } from "@studio/contracts";
import "../management/management.css";
export function PresetControls({
  client,
  projectId,
  run,
  onApply,
  disabled = false,
}: {
  client: StudioClient;
  projectId: string;
  run: OperatorRun;
  onApply: (run: OperatorRun) => void;
  disabled?: boolean;
}) {
  const cache = useQueryClient();
  const [cursor, setCursor] = useState<string | null>(null);
  const [chosen, setChosen] = useState<ToolPreset | null>(null);
  const [mode, setMode] = useState<
    "new" | "edit" | "replace" | "delete" | null
  >(null);
  const [name, setName] = useState("");
  const [notes, setNotes] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [notice, setNotice] = useState("");
  const query = useQuery({
    queryKey: ["project", projectId, "presets", run.operator_id, cursor],
    queryFn: ({ signal }) =>
      client.management.presets(projectId, run.operator_id, cursor, signal),
  });
  useEffect(() => {
    setCursor(null);
    setChosen(null);
    setNotice("");
  }, [run.operator_id]);
  const selected =
    chosen?.run.operator_id === run.operator_id
      ? (query.data?.items.find((item) => item.id === chosen.id) ?? chosen)
      : null;
  const options =
    selected && !query.data?.items.some((item) => item.id === selected.id)
      ? [selected, ...(query.data?.items ?? [])]
      : (query.data?.items ?? []);
  function open(next: NonNullable<typeof mode>) {
    setName(next === "new" ? "" : (selected?.name ?? ""));
    setNotes(next === "new" ? "" : (selected?.notes ?? ""));
    setError(null);
    setMode(next);
  }
  async function save() {
    if (pending || !mode) return;
    setPending(true);
    setError(null);
    try {
      if (mode === "delete" && selected) {
        await client.management.deletePreset(
          projectId,
          selected.id,
          selected.revision,
        );
        setChosen(null);
        setNotice("参数预设已删除。");
      } else {
        const saved = await client.management.savePreset(projectId, {
          id: mode === "new" ? null : (selected?.id ?? null),
          name,
          notes,
          expected_revision: mode === "new" ? 0 : (selected?.revision ?? 0),
          run: mode === "edit" && selected ? selected.run : run,
        });
        setChosen(saved);
        setNotice("参数预设已保存。");
      }
      setMode(null);
      setCursor(null);
      await cache.invalidateQueries({
        queryKey: ["project", projectId, "presets"],
      });
    } catch (failure) {
      setError(failure);
    } finally {
      setPending(false);
    }
  }
  return (
    <>
      <div className="preset-controls" aria-label="项目参数预设">
        <span>参数预设</span>
        <select
          aria-label="选择参数预设"
          value={selected?.id ?? ""}
          disabled={disabled || pending}
          onChange={(event) =>
            setChosen(
              options.find((item) => item.id === event.target.value) ?? null,
            )
          }
        >
          <option value="">选择项目内的预设</option>
          {options.map((item) => (
            <option key={item.id} value={item.id}>
              {item.name}
            </option>
          ))}
        </select>
        <Button
          type="button"
          disabled={disabled || !selected || pending}
          onClick={() => {
            if (selected) {
              onApply(selected.run);
              setNotice("");
            }
          }}
        >
          应用预设
        </Button>
        <Button
          type="button"
          disabled={disabled || pending}
          onClick={() => open("new")}
        >
          保存当前参数
        </Button>
        {selected && (
          <MoreMenu
            label="参数预设"
            disabled={disabled || pending}
            items={[
              { label: "修改名称与备注…", action: () => open("edit") },
              { label: "用当前参数更新预设…", action: () => open("replace") },
              {
                label: "删除预设…",
                danger: true,
                action: () => open("delete"),
              },
            ]}
          />
        )}
        {cursor && (
          <Button type="button" onClick={() => setCursor(null)}>
            首批预设
          </Button>
        )}
        {query.data?.next_cursor && (
          <Button
            type="button"
            onClick={() => setCursor(query.data!.next_cursor!)}
          >
            更多预设
          </Button>
        )}
      </div>
      {notice && (
        <p className="preset-notice" role="status">
          {notice}
        </p>
      )}
      {query.error && <ErrorDetails error={query.error} compact />}
      {mode && (
        <Dialog
          title={
            mode === "delete"
              ? "删除参数预设"
              : mode === "new"
                ? "保存参数预设"
                : mode === "replace"
                  ? "更新预设参数"
                  : "编辑预设信息"
          }
          className="preset-dialog"
          onClose={() => {
            if (!pending) setMode(null);
          }}
        >
          <form
            onSubmit={(event) => {
              event.preventDefault();
              void save();
            }}
          >
            {mode === "delete" ? (
              <p>删除「{selected?.name}」。已提交任务与计算结果会保留。</p>
            ) : (
              <>
                <Field label="预设名称">
                  <input
                    autoFocus
                    aria-label="预设名称"
                    value={name}
                    maxLength={120}
                    required
                    onChange={(event) => setName(event.target.value)}
                  />
                </Field>
                <Field label="备注">
                  <textarea
                    aria-label="预设备注"
                    value={notes}
                    maxLength={4000}
                    rows={3}
                    onChange={(event) => setNotes(event.target.value)}
                  />
                </Field>
                <p>
                  预设保存工具参数，应用时仍需检查当前输入范围。
                  {mode === "replace"
                    ? "确认后使用当前配置覆盖选中的预设参数。"
                    : ""}
                </p>
              </>
            )}
            {!!error && <ErrorDetails error={error} />}
            <div className="dialog-actions">
              <Button
                type="button"
                disabled={pending}
                onClick={() => setMode(null)}
              >
                取消
              </Button>
              <Button
                type="submit"
                disabled={pending || (mode !== "delete" && !name.trim())}
                className={mode === "delete" ? "danger-text" : "primary"}
              >
                {pending
                  ? "处理中…"
                  : mode === "delete"
                    ? "确认删除预设"
                    : "保存预设"}
              </Button>
            </div>
          </form>
        </Dialog>
      )}
    </>
  );
}
