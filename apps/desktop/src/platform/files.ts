import { isTauri } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { browserFiles } from "@studio/ui";
import type { PlatformFiles } from "@studio/ui";
import { chooseDirectory } from "./connection.js";

export const platformFiles: PlatformFiles = {
  ...browserFiles,
  chooseDirectory,
  async chooseSaveFile(defaultName) {
    if (!isTauri()) return undefined;
    const extension = /\.([a-z0-9]{1,8})$/i.exec(defaultName)?.[1];
    return save({
      defaultPath: defaultName,
      ...(extension
        ? {
            filters: [
              {
                name: extension.toUpperCase() + " 文件",
                extensions: [extension],
              },
            ],
          }
        : {}),
    });
  },
};
