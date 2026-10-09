import { createContext, useContext } from "react";

/** File and image capabilities the shell provides to feature modules. */
export type PlatformFiles = {
  /** Absolute path; `null` when cancelled; `undefined` without a native dialog. */
  chooseSaveFile: (defaultName: string) => Promise<string | null | undefined>;
  chooseDirectory: () => Promise<string | null>;
  copyImage: (image: Blob) => Promise<void>;
};

async function asPng(image: Blob) {
  if (image.type === "image/png") return image;
  const bitmap = await createImageBitmap(image);
  try {
    const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
    canvas.getContext("2d")?.drawImage(bitmap, 0, 0);
    return await canvas.convertToBlob({ type: "image/png" });
  } finally {
    bitmap.close();
  }
}
/** Clipboard images are PNG in Chromium/WebView2. */
export async function copyImageWithBrowser(image: Blob) {
  const png = await asPng(image);
  await navigator.clipboard.write([new ClipboardItem({ "image/png": png })]);
}
export function downloadBlob(blob: Blob, name: string) {
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 2000);
}
export const browserFiles: PlatformFiles = {
  chooseSaveFile: () => Promise.resolve(undefined),
  chooseDirectory: () => Promise.resolve(null),
  copyImage: copyImageWithBrowser,
};
const PlatformFilesContext = createContext<PlatformFiles>(browserFiles);
export const PlatformFilesProvider = PlatformFilesContext.Provider;
export function usePlatformFiles() {
  return useContext(PlatformFilesContext);
}
