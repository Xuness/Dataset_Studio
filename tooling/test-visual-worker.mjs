import { spawn } from "node:child_process";
import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { venvPython } from "./platform.mjs";
import { finished } from "./cargo.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(root, ".local/test-runs", "visual-worker-" + Date.now());
await mkdir(run, { recursive: true });
const args = process.argv.slice(2).filter((a) => a !== "--");
await finished(
  spawn(
    process.env.STUDIO_VISUAL_TEST_PYTHON ??
      venvPython(resolve(root, ".local/runtime/visual-worker")),
    [
      "-m",
      "pytest",
      "-q",
      "-p",
      "no:cacheprovider",
      "--basetemp",
      resolve(run, "pytest"),
      ...args,
    ],
    {
      cwd: resolve(root, "services/visual-worker"),
      stdio: "inherit",
      windowsHide: true,
      env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" },
    },
  ),
);
console.log(`测试目录：${run}`);
