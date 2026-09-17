import { Button, Field } from "@studio/ui";
import type { Schema } from "@studio/contracts";

export function SystemPromptEditor({
  value,
  setValue,
  busy,
  save,
  cancel,
}: {
  value: Schema["SaveLlmSystemPrompt"];
  setValue: (value: Schema["SaveLlmSystemPrompt"]) => void;
  busy: boolean;
  save: () => void;
  cancel: () => void;
}) {
  const { config } = value;
  const set = (patch: Partial<typeof config>) =>
    setValue({ ...value, config: { ...config, ...patch } });
  const bytes = new TextEncoder().encode(config.text).length;
  const valid = !!config.name.trim() && !!config.text.trim() && bytes <= 65536;
  return (
    <form
      className="system-prompt-editor"
      onSubmit={(e) => {
        e.preventDefault();
        save();
      }}
    >
      <h4>
        {value.id ? "编辑 System Prompt 预设" : "新建 System Prompt 预设"}
      </h4>
      <fieldset disabled={busy} className="settings-fields">
        <Field label="预设名称">
          <input
            aria-label="预设名称"
            required
            value={config.name}
            onChange={(e) => set({ name: e.target.value })}
            autoFocus
          />
        </Field>
        <Field label="预设备注（可选）">
          <input
            aria-label="预设备注"
            value={config.description}
            onChange={(e) => set({ description: e.target.value })}
          />
        </Field>
        <Field label="System Prompt 正文">
          <textarea
            aria-label="System Prompt 正文"
            required
            rows={14}
            spellCheck={false}
            value={config.text}
            onChange={(e) => set({ text: e.target.value })}
          />
        </Field>
        <p className={bytes > 65536 ? "settings-validation" : "settings-note"}>
          {bytes.toLocaleString()} / 65,536 字节 ·
          正文按原文发送，保留空格和换行。
        </p>
        <div className="settings-toolbar">
          <Button type="submit" disabled={!valid}>
            保存预设
          </Button>
          <Button type="button" onClick={cancel}>
            放弃修改
          </Button>
        </div>
      </fieldset>
    </form>
  );
}
