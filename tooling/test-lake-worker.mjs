import { lakeWorkerPython } from "./lake-worker-runtime.mjs";
import { spawn } from "node:child_process";
import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const python = lakeWorkerPython(root);
if (!python)
  throw new Error(
    "请运行 tooling/setup-lake-worker.ps1 -Dev 或设置 STUDIO_LAKE_TEST_PYTHON",
  );
const run = resolve(root, ".local/test-runs", "lake-worker-" + Date.now());
await mkdir(run, { recursive: true });
const child = spawn(
  python,
  [
    "-m",
    "pytest",
    "-q",
    "-p",
    "no:cacheprovider",
    "--basetemp",
    resolve(run, "pytest"),
  ],
  {
    cwd: resolve(root, "services/lake-worker"),
    stdio: "inherit",
    windowsHide: true,
    env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" },
  },
);
child.on("error", (error) => {
  console.error(
    "运行 tooling/setup-lake-worker.ps1 -Dev 或设置 STUDIO_LAKE_TEST_PYTHON。",
    error.message,
  );
  process.exitCode = 1;
});
child.on("exit", (code) => {
  process.exitCode = code ?? 1;
  console.log(`测试目录：${run}`);
});
