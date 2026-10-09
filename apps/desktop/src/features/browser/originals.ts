import { useMemo } from "react";
import {
  assetTitle,
  downloadBlob,
  errorText,
  useClipboardWriter,
  useNotify,
  usePlatformFiles,
} from "@studio/ui";
import type { StudioClient } from "@studio/client";
import type { Asset } from "@studio/contracts";

export function originalFileName(asset: Asset) {
  const extension = asset.extension.toLowerCase();
  return extension && !asset.name.toLowerCase().endsWith("." + extension)
    ? asset.name + "." + extension
    : asset.name;
}

/** Save, copy and identity actions shared by image menus and viewers. */
export function useImageActions(client: StudioClient, projectId: string) {
  const files = usePlatformFiles();
  const notify = useNotify();
  const writeClipboard = useClipboardWriter();
  return useMemo(
    () => ({
      async save(asset: Asset) {
        const name = originalFileName(asset);
        try {
          const path = await files.chooseSaveFile(name);
          if (path === null) return;
          if (path === undefined) {
            downloadBlob(await client.original(projectId, asset.key), name);
            return;
          }
          const saved = await client.saveOriginal(projectId, asset.key, path);
          notify({ tone: "success", title: "原图已保存", detail: saved.path });
        } catch (error) {
          notify({
            tone: "error",
            title: "未能保存原图",
            detail: errorText(error),
          });
        }
      },
      async copy(asset: Asset) {
        try {
          await files.copyImage(await client.original(projectId, asset.key));
          notify({
            tone: "info",
            title: "已复制图像",
            detail: assetTitle(asset),
          });
        } catch (error) {
          notify({
            tone: "error",
            title: "未能复制图像",
            detail: errorText(error),
          });
        }
      },
      copyIdentity(asset: Asset) {
        void writeClipboard({ text: asset.key.asset_id }).catch(() =>
          notify({
            tone: "error",
            title: "未能写入剪贴板",
            detail: "可在检查器中复制图像身份。",
          }),
        );
      },
    }),
    [client, projectId, files, notify, writeClipboard],
  );
}
export type ImageActions = ReturnType<typeof useImageActions>;
