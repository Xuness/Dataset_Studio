import { mkdir, copyFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";
import { cargo, finished } from "./cargo.mjs";
import { executableName } from "./platform.mjs";
import {
  engineProfile,
  engineBuildArguments,
  engineExecutable,
} from "./engine-profile.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const profile = engineProfile();
const target = execFileSync("rustc", ["--print", "host-tuple"], {
  encoding: "utf8",
}).trim();
if (process.env.CARGO_BUILD_TARGET && process.env.CARGO_BUILD_TARGET !== target)
  throw new Error(
    "源码开发入口使用本机 Rust target，请移除 CARGO_BUILD_TARGET 后启动。",
  );
await finished(
  cargo(engineBuildArguments(profile), {
    cwd: root,
  }),
);
const folder = resolve(root, "apps/desktop/src-tauri/binaries");
await mkdir(folder, { recursive: true });
await copyFile(
  engineExecutable(root, profile),
  // Keep Tauri's staged copy distinct from Cargo's original engine binary.
  resolve(folder, executableName(`studio-engine-sidecar-${target}`)),
);
