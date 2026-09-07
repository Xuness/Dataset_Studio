import { mkdir, copyFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { cargo, finished } from "./cargo.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const release = process.argv.includes("--release");
await finished(
  cargo(["build", "-p", "studio-engine", ...(release ? ["--release"] : [])], {
    cwd: root,
  }),
);
const folder = resolve(root, "apps/desktop/src-tauri/binaries");
await mkdir(folder, { recursive: true });
await copyFile(
  resolve(root, "target", release ? "release" : "debug", "studio-engine.exe"),
  resolve(folder, "studio-engine-x86_64-pc-windows-msvc.exe"),
);
