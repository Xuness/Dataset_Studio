import { Fragment, useEffect, useRef, useState } from "react";
import { Check, Minus, Square, Copy, X } from "lucide-react";
import { Brand, ErrorDetails } from "@studio/ui";
import { nativeWindow, watchWindow, windowAction } from "../platform/window.js";

export type MenuItems = Record<
  string,
  {
    label: string;
    action: () => void;
    disabled?: boolean;
    checked?: boolean;
    /** Display-only accelerator text. */
    shortcut?: string;
    /** Draw a divider before this item. */
    separator?: boolean;
  }[]
>;
export function MenuBar({
  menus = {},
  busy = false,
  title = "Dataset Studio",
  brand = true,
}: {
  menus?: MenuItems;
  busy?: boolean;
  title?: string;
  /** False when the shell draws a larger logo spanning the tab row. */
  brand?: boolean;
}) {
  const [open, setOpen] = useState<string | null>(null);
  const [state, setState] = useState({ maximized: false, focused: true });
  const [error, setError] = useState<unknown>(null);
  const ref = useRef<HTMLDivElement>(null);
  const names = Object.keys(menus);
  const triggers = useRef(new Map<string, HTMLButtonElement>());
  function focusMenu(name: string) {
    setOpen(name);
    requestAnimationFrame(() =>
      ref.current
        ?.querySelector<HTMLButtonElement>(".menu-popup button:not(:disabled)")
        ?.focus(),
    );
  }
  function close() {
    const name = open;
    setOpen(null);
    if (name) triggers.current.get(name)?.focus();
  }
  useEffect(() => {
    let stop: (() => void) | undefined;
    let disposed = false;
    void watchWindow(setState)
      .then((unlisten) => {
        if (disposed) unlisten();
        else stop = unlisten;
      })
      .catch(setError);
    return () => {
      disposed = true;
      stop?.();
    };
  }, []);
  useEffect(() => {
    const outside = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(null);
    };
    const keyboard = (e: KeyboardEvent) => {
      if (e.key === "Escape" && open) {
        e.preventDefault();
        close();
      }
      if (e.key === "F10" && names[0]) {
        e.preventDefault();
        if (open) close();
        else triggers.current.get(names[0])?.focus();
      }
      if (e.altKey && !e.ctrlKey && !e.metaKey) {
        const key = e.key.toLowerCase();
        const name = (
          {
            f: "项目",
            p: "项目",
            e: "编辑",
            v: "视图",
            t: "工具",
            w: "窗口",
            s: "设置",
            h: "帮助",
          } as Record<string, string>
        )[key];
        if (name && menus[name]) {
          e.preventDefault();
          focusMenu(name);
        }
      }
    };
    document.addEventListener("pointerdown", outside);
    document.addEventListener("keydown", keyboard);
    return () => {
      document.removeEventListener("pointerdown", outside);
      document.removeEventListener("keydown", keyboard);
    };
  }, [open, menus, names]);
  return (
    <>
      <div
        ref={ref}
        className={
          "menu-bar titlebar " + (!state.focused ? "window-inactive" : "")
        }
      >
        {brand && <Brand size={25} />}
        <nav className="application-menu" aria-label="应用菜单" role="menubar">
          {names.map((name, index) => (
            <div className="menu-anchor" key={name}>
              <button
                ref={(node) => {
                  if (node) triggers.current.set(name, node);
                  else triggers.current.delete(name);
                }}
                type="button"
                role="menuitem"
                aria-haspopup="menu"
                aria-expanded={open === name}
                className={"menu-button " + (open === name ? "open" : "")}
                onClick={() => setOpen((v) => (v === name ? null : name))}
                onMouseEnter={() => {
                  if (open) setOpen(name);
                }}
                onKeyDown={(e) => {
                  if (
                    e.key === "ArrowDown" ||
                    e.key === "Enter" ||
                    e.key === " "
                  ) {
                    e.preventDefault();
                    focusMenu(name);
                  }
                  if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
                    e.preventDefault();
                    const next =
                      names[
                        (index +
                          (e.key === "ArrowLeft" ? -1 : 1) +
                          names.length) %
                          names.length
                      ]!;
                    triggers.current.get(next)?.focus();
                    if (open) focusMenu(next);
                  }
                }}
              >
                {name}
              </button>
              {open === name && (
                <div
                  className="menu-popup"
                  role="menu"
                  aria-label={name}
                  onKeyDown={(e) => {
                    const buttons = [
                      ...e.currentTarget.querySelectorAll<HTMLButtonElement>(
                        "button:not(:disabled)",
                      ),
                    ];
                    const current = buttons.indexOf(
                      document.activeElement as HTMLButtonElement,
                    );
                    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                      e.preventDefault();
                      buttons[
                        (current +
                          (e.key === "ArrowUp" ? -1 : 1) +
                          buttons.length) %
                          buttons.length
                      ]?.focus();
                    }
                    if (e.key === "Home" || e.key === "End") {
                      e.preventDefault();
                      (e.key === "Home"
                        ? buttons[0]
                        : buttons[buttons.length - 1]
                      )?.focus();
                    }
                    if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
                      e.preventDefault();
                      focusMenu(
                        names[
                          (index +
                            (e.key === "ArrowLeft" ? -1 : 1) +
                            names.length) %
                            names.length
                        ]!,
                      );
                    }
                    if (e.key === "Escape") {
                      e.preventDefault();
                      e.stopPropagation();
                      close();
                    }
                    if (e.key === "Tab") setOpen(null);
                  }}
                >
                  {menus[name]?.map((item, itemIndex) => (
                    <Fragment key={item.label}>
                      {item.separator && itemIndex > 0 && (
                        <div className="menu-separator" role="separator" />
                      )}
                      <button
                        role={
                          item.checked === undefined
                            ? "menuitem"
                            : "menuitemcheckbox"
                        }
                        aria-checked={item.checked}
                        disabled={busy || item.disabled}
                        onClick={() => {
                          item.action();
                          setOpen(null);
                        }}
                      >
                        <span className="menu-check">
                          {item.checked && <Check size={13} />}
                        </span>
                        <span className="menu-label">{item.label}</span>
                        {item.shortcut && (
                          <kbd className="menu-shortcut">{item.shortcut}</kbd>
                        )}
                      </button>
                    </Fragment>
                  ))}
                </div>
              )}
            </div>
          ))}
        </nav>
        <div
          className="titlebar-drag"
          data-tauri-drag-region
          title="拖动移动窗口，双击最大化"
        >
          <span data-tauri-drag-region>{title}</span>
        </div>
        <span className="version-label" data-tauri-drag-region>
          0.9.1
        </span>
        {nativeWindow && (
          <div className="window-controls">
            <button
              aria-label="最小化窗口"
              title="最小化"
              onClick={() => void windowAction("minimize").catch(setError)}
            >
              <Minus size={15} />
            </button>
            <button
              aria-label={state.maximized ? "还原窗口" : "最大化窗口"}
              title={state.maximized ? "还原" : "最大化"}
              onClick={() => void windowAction("maximize").catch(setError)}
            >
              {state.maximized ? <Copy size={13} /> : <Square size={12} />}
            </button>
            <button
              aria-label="关闭窗口"
              title="关闭"
              className="window-close"
              onClick={() => void windowAction("close").catch(setError)}
            >
              <X size={16} />
            </button>
          </div>
        )}
      </div>
      {!!error && (
        <div className="window-error">
          <ErrorDetails error={error} />
          <button onClick={() => setError(null)}>关闭提示</button>
        </div>
      )}
    </>
  );
}
