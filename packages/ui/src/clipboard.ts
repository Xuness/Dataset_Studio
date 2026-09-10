export type ClipboardContent = { text: string; html?: string };
export type ClipboardWriter = (content: ClipboardContent) => Promise<void>;

export const writeBrowserClipboard: ClipboardWriter = async ({
  text,
  html,
}) => {
  if (html && typeof ClipboardItem !== "undefined") {
    await navigator.clipboard.write([
      new ClipboardItem({
        "text/plain": new Blob([text], { type: "text/plain" }),
        "text/html": new Blob([html], { type: "text/html" }),
      }),
    ]);
  } else {
    await navigator.clipboard.writeText(text);
  }
};

function selectedClipboardContent(document: Document): ClipboardContent | null {
  const active = document.activeElement;
  if (
    active instanceof HTMLInputElement ||
    active instanceof HTMLTextAreaElement
  ) {
    // Keep the browser's protected-field behavior and ignore any stale page
    // selection while a control without a text selection has focus.
    if (active instanceof HTMLInputElement && active.type === "password")
      return null;
    const { selectionStart, selectionEnd } = active;
    if (selectionStart === null || selectionEnd === null) {
      // Chromium exposes number/email editor selections through Selection,
      // while those input types deliberately have no selectionStart/End API.
      if (
        active instanceof HTMLInputElement &&
        (active.type === "number" || active.type === "email")
      ) {
        const text = document.getSelection()?.toString();
        return text ? { text } : null;
      }
      return null;
    }
    if (selectionStart === selectionEnd) return null;
    return { text: active.value.slice(selectionStart, selectionEnd) };
  }
  const selection = document.getSelection();
  if (!selection || selection.isCollapsed || !selection.rangeCount) return null;
  const text = selection.toString();
  // Image-only copies still need the WebView's native image handling.
  if (!text) return null;
  const fragment = document.createElement("div");
  for (let index = 0; index < selection.rangeCount; index++) {
    fragment.append(selection.getRangeAt(index).cloneContents());
  }
  return { text, html: fragment.innerHTML };
}

export function installSelectionCopy(
  document: Document,
  write: ClipboardWriter,
  onError: () => void,
  onSuccess: () => void,
) {
  const copy = (event: ClipboardEvent) => {
    if (!event.isTrusted || event.defaultPrevented) return;
    const content = selectedClipboardContent(document);
    if (!content) return;
    // Cancel the WebView write synchronously, otherwise it can overwrite the
    // native write. Do not re-publish the global clipboard after a timer.
    event.preventDefault();
    void write(content).then(onSuccess, onError);
  };
  // Bubble after component handlers so custom copy behavior keeps precedence.
  document.addEventListener("copy", copy);
  return () => document.removeEventListener("copy", copy);
}
