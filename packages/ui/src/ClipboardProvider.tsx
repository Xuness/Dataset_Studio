import { createContext, useContext, useEffect, useState } from "react";
import type { ReactNode } from "react";
import { installSelectionCopy, writeBrowserClipboard } from "./clipboard.js";
import type { ClipboardWriter } from "./clipboard.js";

const ClipboardContext = createContext<ClipboardWriter>(writeBrowserClipboard);

export function ClipboardProvider({
  write,
  captureSelection = false,
  children,
}: {
  write: ClipboardWriter;
  captureSelection?: boolean;
  children: ReactNode;
}) {
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!captureSelection) return;
    return installSelectionCopy(
      document,
      write,
      () => setFailed(true),
      () => setFailed(false),
    );
  }, [captureSelection, write]);
  return (
    <ClipboardContext.Provider value={write}>
      {children}
      {failed && (
        <div className="clipboard-notice" role="alert">
          <span>复制失败，请重新复制所选文本。</span>
          <button
            type="button"
            className="icon-button"
            aria-label="关闭复制提示"
            onClick={() => setFailed(false)}
          >
            ×
          </button>
        </div>
      )}
    </ClipboardContext.Provider>
  );
}

export function useClipboardWriter() {
  return useContext(ClipboardContext);
}
