import { useQuery } from "@tanstack/react-query";
import { Button, ErrorDetails, Field } from "@studio/ui";
import type { SettingsPageProps } from "./types.js";
export function EditingSettingsPage({
  client,
  project,
  busy,
  action,
  undoDraft,
  setUndoDraft,
}: SettingsPageProps) {
  const query = useQuery({
    queryKey: ["settings", "editing", client.connection.instance_id],
    queryFn: ({ signal }) => client.management.editing(signal),
  });
  const history = useQuery({
    queryKey: ["project", project?.id ?? "", "selection-history"],
    queryFn: ({ signal }) => client.management.history(project!.id, signal),
    enabled: !!project,
  });
  const value = undoDraft ?? String(query.data?.undo_limit ?? 50);
  const limit = Number(value);
  const invalid =
    !value.trim() || !Number.isInteger(limit) || limit < 0 || limit > 200;
  return (
    <section className="settings-page" aria-label="编辑与撤销设置">
      <h3>编辑与撤销</h3>
      <p>图片选择、清空选择、替换范围、添加、移除和交集操作都可撤销。</p>
      {query.error && <ErrorDetails error={query.error} />}
      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (query.data && !invalid)
            void action(async () => {
              await client.management.configureEditing({
                undo_limit: limit,
                expected_revision: query.data!.revision,
              });
              setUndoDraft(null);
            }, "撤销上限已保存，打开的项目已应用新设置。");
        }}
      >
        <Field label="每个项目最多保留的撤销步数">
          <input
            aria-label="撤销步数上限"
            type="number"
            min={0}
            max={200}
            step={1}
            value={value}
            disabled={busy || !query.data}
            onChange={(event) => setUndoDraft(event.target.value)}
          />
        </Field>
        <p className="settings-hint">
          默认 50 步，可设置 0–200 步。0
          表示关闭历史；减小上限会丢弃较早的记录。已关闭的项目在下次打开时应用。
        </p>
        {invalid && <p role="alert">请输入 0–200 的整数。</p>}
        <Button
          type="submit"
          disabled={busy || !query.data || invalid || undoDraft === null}
        >
          保存撤销设置
        </Button>
      </form>
      <div className="settings-card">
        <h4>快捷键</h4>
        <p>
          <kbd>Ctrl+Z</kbd> 撤销选择 · <kbd>Ctrl+Y</kbd> 或{" "}
          <kbd>Ctrl+Shift+Z</kbd> 重做
        </p>
        <p>编辑名称、备注和其他输入框时，快捷键仍用于文本编辑。</p>
        <p>
          选择历史随项目保存。删除对象、清理文件和提交计算任务不属于选择撤销。
        </p>
      </div>
      {project && (
        <div className="settings-card">
          <h4>当前项目：{project.name}</h4>
          {history.error && <ErrorDetails error={history.error} compact />}
          <p>
            可撤销 {history.data?.undo_steps ?? 0} 步 · 可重做{" "}
            {history.data?.redo_steps ?? 0} 步
          </p>
          <p>
            历史会保留需要的查询和成果引用。清空后当前选择保持不变，相应历史保护会解除。
          </p>
          <Button
            disabled={
              busy ||
              !history.data ||
              !(history.data.undo_steps || history.data.redo_steps)
            }
            onClick={() =>
              void action(
                () =>
                  client.management.restore(
                    project.id,
                    "clear",
                    history.data!.selection.revision,
                  ),
                "当前项目的选择历史已清空，当前选择保持不变。",
              )
            }
          >
            清空当前项目的选择历史
          </Button>
        </div>
      )}
    </section>
  );
}
