import {
  X,
  Images,
  Sparkles,
  Calculator,
  Archive,
  Database,
} from "lucide-react";
import { MoreMenu } from "@studio/ui";
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
  onClose: (id: string) => void;
  disabled: boolean;
  projectAvailable?: boolean;
}) {
  return (
    <nav className="editor-tabs" aria-label="工作标签">
      <div className="editor-tab-list">
        {open.map((id) => {
          const view = views.find((v) => v.id === id);
          if (!view) return null;
          return (
            <div
              key={id}
              className={"editor-tab" + (active === id ? " active" : "")}
            >
              <button
                type="button"
                disabled={id !== "app.lakes" && disabled}
                aria-pressed={active === id}
                onClick={() => onOpen(id)}
              >
                <view.Icon size={14} />
                {view.title}
              </button>
              <button
                type="button"
                className="editor-tab-close"
                disabled={id !== "app.lakes" && disabled}
                aria-label={"关闭" + view.title + "标签"}
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
    </nav>
  );
}
