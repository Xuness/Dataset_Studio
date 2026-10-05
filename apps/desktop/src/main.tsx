import React from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { App } from "./app/App.js";
import {
  ClipboardProvider,
  TooltipLayer,
  installNumberScrub,
  installColumnResize,
} from "@studio/ui";
import { nativeClipboard, writeClipboard } from "./platform/clipboard.js";
import "@studio/ui/styles.css";
import "./app/studio.css";
import "./app/workbench-theme.css";
const queries = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      staleTime: 30_000,
      gcTime: 60_000,
      refetchOnWindowFocus: false,
    },
    mutations: { retry: false },
  },
});
// The WebView's own menu (back, reload, inspect) is not part of the editor.
// Text fields and selected text keep it for copy and paste; Shift bypasses.
document.addEventListener("contextmenu", (event) => {
  const target = event.target as Element | null;
  if (
    event.defaultPrevented ||
    event.shiftKey ||
    target?.closest("input,textarea,[contenteditable=true]") ||
    !document.getSelection()?.isCollapsed
  )
    return;
  event.preventDefault();
});
installNumberScrub(document);
installColumnResize(document);
const element = document.getElementById("root");
if (!element) throw new Error("Root element missing");
createRoot(element).render(
  <React.StrictMode>
    <ClipboardProvider
      write={writeClipboard}
      captureSelection={nativeClipboard}
    >
      <QueryClientProvider client={queries}>
        <App />
      </QueryClientProvider>
      <TooltipLayer />
    </ClipboardProvider>
  </React.StrictMode>,
);
