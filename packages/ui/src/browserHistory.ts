import type {
  BrowserHistory,
  BrowserPosition,
  BrowseScope,
} from "./modules.js";

/** Display labels are editable; they never identify a data range or its page. */
export function browseScopeIdentity(scope: BrowseScope) {
  return "id" in scope
    ? { kind: scope.kind, id: scope.id }
    : { kind: scope.kind };
}
export function normalizeBrowseScopeKey(value: string): string {
  try {
    const parts: unknown = JSON.parse(value);
    if (
      !Array.isArray(parts) ||
      parts.length !== 4 ||
      !parts[0] ||
      typeof parts[0] !== "object" ||
      !("kind" in parts[0])
    )
      return value;
    const scope = parts[0] as BrowseScope;
    if (
      !["all", "selection", "source", "collection", "result"].includes(
        scope.kind,
      )
    )
      return value;
    return JSON.stringify([browseScopeIdentity(scope), ...parts.slice(1)]);
  } catch {
    return value;
  }
}

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
