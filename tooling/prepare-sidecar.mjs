import { mkdir, copyFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { cargo, finished } from "./cargo.mjs";
import {
  engineProfile,
  engineBuildArguments,
  engineExecutable,
} from "./engine-profile.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const profile = engineProfile();
await finished(
  cargo(engineBuildArguments(profile), {
    cwd: root,
  }),
);
const folder = resolve(root, "apps/desktop/src-tauri/binaries");
await mkdir(folder, { recursive: true });
await copyFile(
  engineExecutable(root, profile),
  resolve(folder, "studio-engine-x86_64-pc-windows-msvc.exe"),
);
