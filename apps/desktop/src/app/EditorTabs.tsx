import { useState } from "react";
import {
  X,
  Images,
  Sparkles,
  Calculator,
  Archive,
  Database,
} from "lucide-react";
import { ContextMenu, MoreMenu, contextMenuAt } from "@studio/ui";
import type { ContextMenuState } from "@studio/ui";
const views = [
  { id: "app.lakes", title: "数据湖", Icon: Database },
  { id: "core.browser", title: "资料浏览", Icon: Images },
  { id: "core.aesthetic", title: "美学排序", Icon: Sparkles },
  { id: "core.tools", title: "计算工具", Icon: Calculator },
  { id: "core.artifacts", title: "项目成果", Icon: Archive },
];
export function EditorTabs({
  open,
  active,
  onOpen,
  onClose,
  disabled,
  projectAvailable = true,
}: {
  open: string[];
  active: string;
  onOpen: (id: string) => void;
  onClose: (ids: string | string[]) => void;
  disabled: boolean;
  projectAvailable?: boolean;
}) {
  const [menu, setMenu] = useState<ContextMenuState | null>(null);
  const shown = open.filter((id) => views.some((v) => v.id === id));
  return (
    <nav className="editor-tabs" aria-label="工作标签">
      <div className="editor-tab-list">
        {shown.map((id, index) => {
          const view = views.find((v) => v.id === id)!;
          const locked = id !== "app.lakes" && disabled;
          return (
            <div
              key={id}
              className={"editor-tab" + (active === id ? " active" : "")}
              onContextMenu={(event) =>
                setMenu(
                  contextMenuAt(event, view.title, [
                    {
                      label: "关闭",
                      ...(active === id ? { shortcut: "Ctrl+W" } : {}),
                      disabled: locked,
                      action: () => onClose(id),
                    },
                    {
                      label: "关闭其他标签",
                      disabled: disabled || shown.length < 2,
                      action: () => onClose(shown.filter((v) => v !== id)),
                    },
                    {
                      label: "关闭右侧标签",
                      disabled: disabled || index === shown.length - 1,
                      action: () => onClose(shown.slice(index + 1)),
                    },
                  ]),
                )
              }
              onAuxClick={(event) => {
                // Middle click closes, as in editor and browser tab strips.
                if (event.button === 1 && !locked) {
                  event.preventDefault();
                  onClose(id);
                }
              }}
            >
              <button
                type="button"
                disabled={locked}
                aria-pressed={active === id}
                onClick={() => onOpen(id)}
              >
                <view.Icon size={14} />
                {view.title}
              </button>
              <button
                type="button"
                className="editor-tab-close"
                disabled={locked}
                aria-label={"关闭" + view.title + "标签"}
                title={active === id ? "关闭（Ctrl+W）" : "关闭"}
                onClick={() => onClose(id)}
              >
                <X size={12} />
              </button>
            </div>
          );
        })}
      </div>
      <MoreMenu
        label="打开功能"
        items={views.map((view) => ({
          label: view.title,
          action: () => onOpen(view.id),
          disabled: view.id !== "app.lakes" && (!projectAvailable || disabled),
        }))}
      />
      <ContextMenu state={menu} onClose={() => setMenu(null)} />
    </nav>
  );
}
