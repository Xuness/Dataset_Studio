import { mkdir, copyFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";
import { finished } from "./cargo.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const source = resolve(root, "packages/ui/src/ds.svg");
const output = resolve(root, ".local/generated-icons");
await mkdir(output, { recursive: true });
await finished(
  spawn(
    process.execPath,
    [
      resolve(root, "apps/desktop/node_modules/@tauri-apps/cli/tauri.js"),
      "icon",
      source,
      "--output",
      output,
    ],
    { cwd: root, stdio: "inherit", windowsHide: true },
  ),
);
for (const name of ["32x32.png", "128x128.png", "icon.ico"])
  await copyFile(
    resolve(output, name),
    resolve(root, "apps/desktop/src-tauri/icons", name),
  );
await copyFile(source, resolve(root, "apps/desktop/src-tauri/app-icon.svg"));
console.log("Ds 矢量标记与 Windows 图标已同步。");
