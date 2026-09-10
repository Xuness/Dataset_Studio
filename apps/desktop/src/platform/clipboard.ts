import { invoke, isTauri } from "@tauri-apps/api/core";
import { writeBrowserClipboard } from "@studio/ui";
import type { ClipboardWriter } from "@studio/ui";

export const nativeClipboard = isTauri();

export const writeClipboard: ClipboardWriter = async (content) => {
  if (!nativeClipboard) return writeBrowserClipboard(content);
  if (content.html) {
    await invoke("plugin:clipboard-manager|write_html", {
      html: content.html,
      altText: content.text,
    });
  } else {
    await invoke("plugin:clipboard-manager|write_text", { text: content.text });
  }
};
