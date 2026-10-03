import { invoke, isTauri } from "@tauri-apps/api/core";
import { StudioError, type CollectionLoginAssistant } from "@studio/client";

async function call<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    if (
      error &&
      typeof error === "object" &&
      "code" in error &&
      "message" in error
    )
      throw new StudioError(String(error.code), String(error.message));
    throw new StudioError(
      "LOGIN_HOST_UNAVAILABLE",
      "登录助手暂不可用，请重启桌面版或使用 Cookie 导入。",
    );
  }
}
export const collectionLogin: CollectionLoginAssistant | undefined = isTauri()
  ? {
      status: () => call("pixiv_login_status"),
      start: (input) => call("pixiv_login_start", { input }),
      show: (id) => call("pixiv_login_show", { id }),
      finish: (id) => call("pixiv_login_finish", { id }),
      cancel: (id) => call("pixiv_login_cancel", { id }),
    }
  : undefined;
