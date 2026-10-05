const EDGE = 6;

function headerCell(target: EventTarget | null) {
  const cell = (target as Element | null)?.closest?.("th");
  return cell?.closest("thead") &&
    !cell.closest(".property-matrix, [data-no-resize]")
    ? (cell as HTMLTableCellElement)
    : null;
}

/**
 * Editor-style column resizing for data tables: drag a header cell's right
 * edge to resize, double-click it to return to automatic widths. The first
 * drag freezes the current widths so other columns keep their size.
 */
export function installColumnResize(document: Document) {
  const down = (event: PointerEvent) => {
    const cell = headerCell(event.target);
    if (!cell || event.button !== 0) return;
    const rect = cell.getBoundingClientRect();
    if (event.clientX < rect.right - EDGE) return;
    const table = cell.closest("table");
    const row = cell.parentElement;
    if (!table || !row) return;
    event.preventDefault();
    if (table.style.tableLayout !== "fixed") {
      const cells = [...row.children] as HTMLElement[];
      const widths = cells.map((c) => c.getBoundingClientRect().width);
      cells.forEach((c, i) => (c.style.width = widths[i] + "px"));
      table.style.width = table.getBoundingClientRect().width + "px";
      table.style.tableLayout = "fixed";
    }
    const start = event.clientX;
    const startCell = cell.getBoundingClientRect().width;
    const startTable = table.getBoundingClientRect().width;
    document.documentElement.classList.add("wb-col-resizing");
    const move = (e: PointerEvent) => {
      const width = Math.max(40, startCell + e.clientX - start);
      cell.style.width = width + "px";
      table.style.width = startTable + width - startCell + "px";
    };
    const up = () => {
      document.removeEventListener("pointermove", move);
      document.removeEventListener("pointerup", up);
      document.removeEventListener("pointercancel", up);
      document.documentElement.classList.remove("wb-col-resizing");
    };
    document.addEventListener("pointermove", move);
    document.addEventListener("pointerup", up);
    document.addEventListener("pointercancel", up);
  };
  // Resizing ends with a click on the header; keep it from sorting or
  // selecting anything.
  const click = (event: MouseEvent) => {
    const cell = headerCell(event.target);
    if (cell && event.clientX >= cell.getBoundingClientRect().right - EDGE)
      event.stopPropagation();
  };
  const reset = (event: MouseEvent) => {
    const cell = headerCell(event.target);
    if (!cell || event.clientX < cell.getBoundingClientRect().right - EDGE)
      return;
    const table = cell.closest("table");
    if (!table) return;
    event.stopPropagation();
    table.style.removeProperty("table-layout");
    table.style.removeProperty("width");
    for (const c of cell.parentElement?.children ?? [])
      (c as HTMLElement).style.removeProperty("width");
  };
  document.addEventListener("pointerdown", down, true);
  document.addEventListener("click", click, true);
  document.addEventListener("dblclick", reset, true);
  return () => {
    document.removeEventListener("pointerdown", down, true);
    document.removeEventListener("click", click, true);
    document.removeEventListener("dblclick", reset, true);
  };
}
