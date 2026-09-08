import type { BrowserHistory, BrowserPosition } from "./modules.js";

const MAX_PAGES = 128;
const MAX_BYTES = 48 * 1024;
export function initialHistory(): BrowserHistory {
  return { cursors: [null], index: 0, firstPage: 1 };
}
export function restoreHistory(
  position: BrowserPosition | null,
  scopeKey: string,
): BrowserHistory {
  if (position?.scopeKey !== scopeKey) return initialHistory();
  const h = position.history;
  if (
    h &&
    Array.isArray(h.cursors) &&
    h.cursors.length > 0 &&
    h.cursors.length <= MAX_PAGES &&
    h.cursors.every(
      (c) => c === null || (typeof c === "string" && c.length <= 16384),
    ) &&
    Number.isInteger(h.index) &&
    h.index >= 0 &&
    h.index < h.cursors.length &&
    Number.isInteger(h.firstPage) &&
    h.firstPage >= 1 &&
    JSON.stringify(h).length <= MAX_BYTES &&
    h.cursors[h.index] === position.cursor &&
    h.firstPage + h.index === position.pageNumber
  )
    return { cursors: [...h.cursors], index: h.index, firstPage: h.firstPage };
  return {
    cursors: [position.cursor],
    index: 0,
    firstPage: Math.max(1, position.pageNumber),
  };
}
export function nextHistory(
  history: BrowserHistory,
  cursor: string,
): BrowserHistory {
  const cursors = [...history.cursors.slice(0, history.index + 1), cursor];
  let firstPage = history.firstPage;
  while (
    cursors.length > MAX_PAGES ||
    JSON.stringify({ cursors, index: cursors.length - 1, firstPage }).length >
      MAX_BYTES
  ) {
    cursors.shift();
    firstPage++;
  }
  return { cursors, index: cursors.length - 1, firstPage };
}
