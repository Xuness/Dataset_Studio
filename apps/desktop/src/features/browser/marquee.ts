import { useEffect, useRef } from "react";
import type { RefObject } from "react";

type Drag = {
  pointer: number;
  x: number;
  y: number;
  clientX: number;
  clientY: number;
  active: boolean;
  hits: Set<number>;
  box: HTMLDivElement | null;
  timer: number;
};

/**
 * Rubber-band selection over the grid's empty space. Hits are previewed with
 * `data-marquee` on the cards (no React render per move) and committed once
 * on release; Alt removes instead of adding. Escape cancels.
 */
export function useMarquee(
  scroll: RefObject<HTMLDivElement | null>,
  grid: RefObject<HTMLDivElement | null>,
  enabled: boolean,
  onCommit: (indexes: number[], remove: boolean) => void,
) {
  const commit = useRef(onCommit);
  commit.current = onCommit;
  useEffect(() => {
    const container = scroll.current;
    if (!container || !enabled) return;
    let drag: Drag | null = null;
    const cards = () =>
      Array.from(grid.current?.children ?? []).filter(
        (node): node is HTMLElement => node.classList.contains("asset-card"),
      );
    const content = (clientX: number, clientY: number) => {
      const rect = container.getBoundingClientRect();
      return {
        x: clientX - rect.left + container.scrollLeft,
        y: clientY - rect.top + container.scrollTop,
      };
    };
    const clear = () => {
      if (!drag) return;
      window.clearInterval(drag.timer);
      drag.box?.remove();
      container.classList.remove("marquee-active");
      for (const card of cards()) card.removeAttribute("data-marquee");
      drag = null;
    };
    const update = () => {
      if (!drag) return;
      const end = content(drag.clientX, drag.clientY);
      const left = Math.min(drag.x, end.x);
      const top = Math.min(drag.y, end.y);
      const right = Math.max(drag.x, end.x);
      const bottom = Math.max(drag.y, end.y);
      if (!drag.active) {
        if (right - left < 5 && bottom - top < 5) return;
        drag.active = true;
        container.classList.add("marquee-active");
        document.getSelection()?.removeAllRanges();
        drag.box = document.createElement("div");
        drag.box.className = "grid-marquee";
        container.append(drag.box);
      }
      Object.assign(drag.box!.style, {
        left: left + "px",
        top: top + "px",
        width: right - left + "px",
        height: bottom - top + "px",
      });
      const origin = container.getBoundingClientRect();
      drag.hits.clear();
      cards().forEach((card, index) => {
        const r = card.getBoundingClientRect();
        const x = r.left - origin.left + container.scrollLeft;
        const y = r.top - origin.top + container.scrollTop;
        const hit =
          x < right && x + r.width > left && y < bottom && y + r.height > top;
        if (hit) drag!.hits.add(index);
        card.toggleAttribute("data-marquee", hit);
      });
    };
    const down = (event: PointerEvent) => {
      const target = event.target as Element;
      if (
        event.button !== 0 ||
        event.ctrlKey ||
        event.shiftKey ||
        target.closest(".asset-card, button, input, a, .image-loading")
      )
        return;
      // Clicks on the scrollbar land on the container outside its client box.
      const rect = container.getBoundingClientRect();
      if (
        event.clientX - rect.left >= container.clientWidth ||
        event.clientY - rect.top >= container.clientHeight
      )
        return;
      const start = content(event.clientX, event.clientY);
      drag = {
        pointer: event.pointerId,
        ...start,
        clientX: event.clientX,
        clientY: event.clientY,
        active: false,
        hits: new Set(),
        box: null,
        // Keep scrolling while the pointer rests near an edge.
        timer: window.setInterval(() => {
          if (!drag?.active) return;
          const r = container.getBoundingClientRect();
          const step =
            drag.clientY > r.bottom - 28
              ? 14
              : drag.clientY < r.top + 28
                ? -14
                : 0;
          if (step) {
            container.scrollTop += step;
            update();
          }
        }, 30),
      };
      container.setPointerCapture(event.pointerId);
    };
    const move = (event: PointerEvent) => {
      if (!drag || event.pointerId !== drag.pointer) return;
      drag.clientX = event.clientX;
      drag.clientY = event.clientY;
      update();
    };
    const up = (event: PointerEvent) => {
      if (!drag || event.pointerId !== drag.pointer) return;
      const { active, hits } = drag;
      clear();
      if (active && hits.size)
        commit.current(
          [...hits].sort((a, b) => a - b),
          event.altKey,
        );
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape" && drag?.active) {
        event.preventDefault();
        event.stopPropagation();
        clear();
      }
    };
    container.addEventListener("pointerdown", down);
    container.addEventListener("pointermove", move);
    container.addEventListener("pointerup", up);
    container.addEventListener("pointercancel", clear);
    window.addEventListener("keydown", key, true);
    return () => {
      clear();
      container.removeEventListener("pointerdown", down);
      container.removeEventListener("pointermove", move);
      container.removeEventListener("pointerup", up);
      container.removeEventListener("pointercancel", clear);
      window.removeEventListener("keydown", key, true);
    };
  }, [scroll, grid, enabled]);
}
