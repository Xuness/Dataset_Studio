import { useEffect, useRef, useState } from "react";
import { MoreHorizontal } from "lucide-react";
export type MoreMenuItem = { label: string; action: () => void; disabled?: boolean; danger?: boolean };

/** A top-layer menu remains visible inside clipped, scrollable object lists. */
export function MoreMenu({ label, items, disabled = false }: { label: string; items: MoreMenuItem[]; disabled?: boolean }) {
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState({ left: 0, top: 0 });
  useEffect(() => {
    const node = menu.current;
    if (!node) return;
    const toggle = () => setOpen(node.matches(":popover-open"));
    node.addEventListener("toggle", toggle);
    return () => node.removeEventListener("toggle", toggle);
  }, []);
  function close(focus = true) {
    menu.current?.hidePopover();
    if (focus) trigger.current?.focus();
  }
  useEffect(() => {
    if (!open) return;
    const closeOnScroll = (event: Event) => { if (!menu.current?.contains(event.target as Node)) menu.current?.hidePopover(); };
    window.addEventListener("resize", closeOnScroll);
    document.addEventListener("scroll", closeOnScroll, true);
    return () => { window.removeEventListener("resize", closeOnScroll); document.removeEventListener("scroll", closeOnScroll, true); };
  }, [open]);
  function show() {
    if (open) { close(); return; }
    const rect = trigger.current?.getBoundingClientRect();
    if (!rect) return;
    setPosition({ left: Math.max(8, Math.min(rect.right - 212, window.innerWidth - 220)), top: Math.max(8, Math.min(rect.bottom + 3, window.innerHeight - Math.min(items.length * 33 + 12, 360) - 8)) });
    menu.current?.showPopover();
    requestAnimationFrame(() => menu.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus());
  }
  return <>
    <button ref={trigger} type="button" className="icon-button object-more" aria-label={label + "更多操作"} title={label + "更多操作"} aria-haspopup="menu" aria-expanded={open} disabled={disabled} onClick={(event) => { event.stopPropagation(); show(); }} onKeyDown={(event) => { if (event.key === "ArrowDown") { event.preventDefault(); show(); } }}><MoreHorizontal size={16} /></button>
    <div ref={menu} popover="auto" role="menu" aria-label={label + "操作"} className="object-popover" style={position} onClick={(event) => event.stopPropagation()} onKeyDown={(event) => {
      const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
      const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
      if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
        event.preventDefault();
        const next = event.key === "Home" ? 0 : event.key === "End" ? buttons.length - 1 : (index + (event.key === "ArrowUp" ? -1 : 1) + buttons.length) % buttons.length;
        buttons[next]?.focus();
      }
      if (event.key === "Escape" || event.key === "Tab") { if (event.key === "Escape") event.preventDefault(); close(event.key === "Escape"); }
    }}>
      {items.map((item) => <button key={item.label} type="button" role="menuitem" className={item.danger ? "danger-text" : ""} disabled={item.disabled} onClick={() => { close(); item.action(); }}>{item.label}</button>)}
    </div>
  </>;
}
