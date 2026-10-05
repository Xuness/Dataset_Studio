import { useEffect, useRef, useState } from "react";
import type { MouseEvent as ReactMouseEvent, RefObject } from "react";
import { Check, MoreHorizontal } from "lucide-react";
export type MoreMenuItem = {
  label: string;
  action: () => void;
  disabled?: boolean;
  danger?: boolean;
  checked?: boolean;
  /** Display-only accelerator text; the owner still handles the key. */
  shortcut?: string;
  /** Draw a divider before this item. */
  separator?: boolean;
};
type Anchor = { x: number; y: number; alignRight?: boolean };

function placeMenu(node: HTMLElement, anchor: Anchor) {
  const { width, height } = node.getBoundingClientRect();
  const left = anchor.alignRight ? anchor.x - width : anchor.x;
  node.style.left =
    Math.max(8, Math.min(left, window.innerWidth - width - 8)) + "px";
  node.style.top =
    Math.max(8, Math.min(anchor.y, window.innerHeight - height - 8)) + "px";
}

/** A top-layer menu remains visible inside clipped, scrollable object lists. */
function useMenuPopover(onClosed?: () => void) {
  const menu = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const closed = useRef(onClosed);
  closed.current = onClosed;
  useEffect(() => {
    const node = menu.current;
    if (!node) return;
    const toggle = () => {
      const now = node.matches(":popover-open");
      setOpen(now);
      if (!now) closed.current?.();
    };
    node.addEventListener("toggle", toggle);
    return () => node.removeEventListener("toggle", toggle);
  }, []);
  useEffect(() => {
    if (!open) return;
    const closeOnScroll = (event: Event) => {
      if (!menu.current?.contains(event.target as Node))
        menu.current?.hidePopover();
    };
    window.addEventListener("resize", closeOnScroll);
    document.addEventListener("scroll", closeOnScroll, true);
    return () => {
      window.removeEventListener("resize", closeOnScroll);
      document.removeEventListener("scroll", closeOnScroll, true);
    };
  }, [open]);
  function show(anchor: Anchor) {
    const node = menu.current;
    if (!node) return;
    if (!node.matches(":popover-open")) node.showPopover();
    placeMenu(node, anchor);
    requestAnimationFrame(() =>
      node.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus(),
    );
  }
  return { menu, open, show, hide: () => menu.current?.hidePopover() };
}

function MenuList({
  label,
  items,
  menu,
  onClose,
}: {
  label: string;
  items: MoreMenuItem[];
  menu: RefObject<HTMLDivElement | null>;
  onClose: (focusReturn: boolean) => void;
}) {
  const checks = items.some((item) => item.checked !== undefined);
  return (
    <div
      ref={menu}
      popover="auto"
      role="menu"
      aria-label={label + "操作"}
      className="object-popover"
      onClick={(event) => event.stopPropagation()}
      onContextMenu={(event) => event.preventDefault()}
      onKeyDown={(event) => {
        const buttons = [
          ...event.currentTarget.querySelectorAll<HTMLButtonElement>(
            "button:not(:disabled)",
          ),
        ];
        const index = buttons.indexOf(
          document.activeElement as HTMLButtonElement,
        );
        if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
          event.preventDefault();
          const next =
            event.key === "Home"
              ? 0
              : event.key === "End"
                ? buttons.length - 1
                : (index +
                    (event.key === "ArrowUp" ? -1 : 1) +
                    buttons.length) %
                  buttons.length;
          buttons[next]?.focus();
        }
        if (event.key === "Escape" || event.key === "Tab") {
          if (event.key === "Escape") event.preventDefault();
          onClose(event.key === "Escape");
        }
      }}
    >
      {items.map((item, index) => (
        <div key={item.label + index} className="menu-entry">
          {item.separator && index > 0 && (
            <div className="menu-separator" role="separator" />
          )}
          <button
            type="button"
            role={item.checked === undefined ? "menuitem" : "menuitemcheckbox"}
            aria-checked={item.checked}
            className={item.danger ? "danger-text" : ""}
            disabled={item.disabled}
            onClick={() => {
              onClose(true);
              item.action();
            }}
          >
            {checks && (
              <span className="menu-check">
                {item.checked && <Check size={13} />}
              </span>
            )}
            <span className="menu-label">{item.label}</span>
            {item.shortcut && (
              <kbd className="menu-shortcut">{item.shortcut}</kbd>
            )}
          </button>
        </div>
      ))}
    </div>
  );
}

/**
 * Overflow button for an object row. With `contextMenu`, right-clicking the
 * row that contains the button opens the same items at the pointer.
 */
export function MoreMenu({
  label,
  items,
  disabled = false,
  contextMenu = false,
}: {
  label: string;
  items: MoreMenuItem[];
  disabled?: boolean;
  contextMenu?: boolean;
}) {
  const trigger = useRef<HTMLButtonElement>(null);
  const returnFocus = useRef<HTMLElement | null>(null);
  const popover = useMenuPopover();
  const show = useRef<(anchor: Anchor, from: HTMLElement | null) => void>(
    () => {},
  );
  show.current = (anchor, from) => {
    returnFocus.current = from;
    popover.show(anchor);
  };
  useEffect(() => {
    const row = trigger.current?.parentElement;
    if (!contextMenu || !row) return;
    const open = (event: MouseEvent) => {
      if (disabled || event.shiftKey || event.defaultPrevented) return;
      event.preventDefault();
      show.current(
        { x: event.clientX, y: event.clientY },
        (event.target as Element).closest<HTMLElement>("button,[tabindex]") ??
          trigger.current,
      );
    };
    row.addEventListener("contextmenu", open);
    return () => row.removeEventListener("contextmenu", open);
  }, [contextMenu, disabled]);
  function close(focus: boolean) {
    popover.hide();
    if (focus) (returnFocus.current ?? trigger.current)?.focus();
  }
  function toggle() {
    if (popover.open) {
      close(true);
      return;
    }
    const rect = trigger.current?.getBoundingClientRect();
    if (rect)
      show.current(
        { x: rect.right, y: rect.bottom + 3, alignRight: true },
        trigger.current,
      );
  }
  return (
    <>
      <button
        ref={trigger}
        type="button"
        className="icon-button object-more"
        aria-label={label + "更多操作"}
        title={label + "更多操作"}
        aria-haspopup="menu"
        aria-expanded={popover.open}
        disabled={disabled}
        onClick={(event) => {
          event.stopPropagation();
          toggle();
        }}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown") {
            event.preventDefault();
            toggle();
          }
        }}
      >
        <MoreHorizontal size={16} />
      </button>
      <MenuList
        label={label}
        items={items}
        menu={popover.menu}
        onClose={close}
      />
    </>
  );
}

export type ContextMenuState = {
  x: number;
  y: number;
  label: string;
  items: MoreMenuItem[];
  returnFocus?: HTMLElement | null;
};
/** Build state for `ContextMenu` from a React contextmenu event. */
export function contextMenuAt(
  event: ReactMouseEvent,
  label: string,
  items: MoreMenuItem[],
): ContextMenuState | null {
  if (event.shiftKey) return null;
  event.preventDefault();
  return {
    x: event.clientX,
    y: event.clientY,
    label,
    items,
    returnFocus: event.currentTarget as HTMLElement,
  };
}

/** One pointer-positioned menu shared by many rows or cards. */
export function ContextMenu({
  state,
  onClose,
}: {
  state: ContextMenuState | null;
  onClose: () => void;
}) {
  const popover = useMenuPopover(onClose);
  useEffect(() => {
    if (state) popover.show({ x: state.x, y: state.y });
    else popover.hide();
  }, [state]);
  return (
    <MenuList
      label={state?.label ?? ""}
      items={state?.items ?? []}
      menu={popover.menu}
      onClose={(focus) => {
        popover.hide();
        if (focus) state?.returnFocus?.focus();
      }}
    />
  );
}
