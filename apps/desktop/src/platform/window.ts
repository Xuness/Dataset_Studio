import { isTauri } from "@tauri-apps/api/core";

export const nativeWindow = isTauri();
export async function windowAction(action: "minimize" | "maximize" | "close") {
  if (!nativeWindow) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const window = getCurrentWindow();
  if (action === "minimize") await window.minimize();
  else if (action === "maximize") await window.toggleMaximize();
  else await window.close();
}
export async function watchWindow(
  onChange: (state: { maximized: boolean; focused: boolean }) => void,
) {
  if (!nativeWindow) return () => {};
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const window = getCurrentWindow();
  let active = true;
  const sync = async () => {
    const [maximized, focused] = await Promise.all([
      window.isMaximized(),
      window.isFocused(),
    ]);
    if (active) onChange({ maximized, focused });
  };
  await sync();
  const stops = await Promise.all([
    window.onResized(() => {
      void sync().catch(() => {});
    }),
    window.onFocusChanged(() => {
      void sync().catch(() => {});
    }),
  ]);
  return () => {
    active = false;
    stops.forEach((stop) => stop());
  };
}
