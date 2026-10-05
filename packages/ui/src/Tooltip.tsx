import { useEffect, useLayoutEffect, useRef, useState } from "react";

const SKIP = "option";

/**
 * Replaces native `title` tooltips with an editor-styled one. While an element
 * is hovered its title moves aside so the system tooltip stays hidden, and it
 * is restored on leave, so call sites keep using plain `title`.
 */
export function TooltipLayer({ delay = 450 }: { delay?: number }) {
  const [tip, setTip] = useState<{ text: string; x: number; y: number } | null>(
    null,
  );
  const node = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let current: Element | null = null;
    let saved = "";
    let labelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let pointer = { x: 0, y: 0 };
    let warmUntil = 0;
    function hide() {
      clearTimeout(timer);
      setTip((old) => {
        if (old) warmUntil = performance.now() + 500;
        return null;
      });
    }
    function release() {
      hide();
      if (current && saved && !current.hasAttribute("title"))
        current.setAttribute("title", saved);
      if (current && labelled && current.getAttribute("aria-label") === saved)
        current.removeAttribute("aria-label");
      current = null;
      saved = "";
      labelled = false;
    }
    const over = (event: PointerEvent) => {
      const target = event.target as Element;
      if (current?.contains(target)) return;
      release();
      const element = target.closest?.("[title]");
      const text = element?.getAttribute("title")?.trim();
      if (!element || !text || element.closest(SKIP)) return;
      current = element;
      saved = element.getAttribute("title")!;
      // For icon-only controls the title is the accessible name; keep that
      // name while the title is away.
      labelled =
        !element.hasAttribute("aria-label") &&
        !element.hasAttribute("aria-labelledby") &&
        !element.textContent?.trim();
      if (labelled) element.setAttribute("aria-label", saved);
      element.removeAttribute("title");
      pointer = { x: event.clientX, y: event.clientY };
      timer = setTimeout(
        () => setTip({ text, x: pointer.x, y: pointer.y }),
        performance.now() < warmUntil ? 60 : delay,
      );
    };
    const move = (event: PointerEvent) => {
      pointer = { x: event.clientX, y: event.clientY };
    };
    const out = (event: PointerEvent) => {
      const next = event.relatedTarget as Node | null;
      if (current && (!next || !current.contains(next))) release();
    };
    document.addEventListener("pointerover", over);
    document.addEventListener("pointermove", move, { passive: true });
    document.addEventListener("pointerout", out);
    document.addEventListener("pointerdown", hide, true);
    document.addEventListener("keydown", hide, true);
    document.addEventListener("wheel", hide, { capture: true, passive: true });
    window.addEventListener("blur", release);
    return () => {
      release();
      document.removeEventListener("pointerover", over);
      document.removeEventListener("pointermove", move);
      document.removeEventListener("pointerout", out);
      document.removeEventListener("pointerdown", hide, true);
      document.removeEventListener("keydown", hide, true);
      document.removeEventListener("wheel", hide, true);
      window.removeEventListener("blur", release);
    };
  }, [delay]);
  useLayoutEffect(() => {
    const element = node.current;
    if (!element) return;
    if (!tip) {
      if (element.matches(":popover-open")) element.hidePopover();
      return;
    }
    // Re-entering the top layer keeps the tip above any open modal dialog.
    if (element.matches(":popover-open")) element.hidePopover();
    element.showPopover();
    const { width, height } = element.getBoundingClientRect();
    let top = tip.y + 20;
    if (top + height > window.innerHeight - 6) top = tip.y - height - 8;
    element.style.left =
      Math.max(6, Math.min(tip.x + 4, window.innerWidth - width - 6)) + "px";
    element.style.top = Math.max(6, top) + "px";
  }, [tip]);
  return (
    <div ref={node} popover="manual" className="wb-tooltip" aria-hidden="true">
      {tip?.text}
    </div>
  );
}
