import { spawn } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { finished } from "./cargo.mjs";
import { pythonCommand, venvPython } from "./platform.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const args = process.argv.slice(2);
if (
  args.some(
    (a) =>
      a !== "--dev" &&
      a !== "--cpu" &&
      !a.startsWith("--python=") &&
      !a.startsWith("--index-url="),
  )
)
  throw new Error(
    "用法：pnpm setup:visual [--dev] [--cpu] [--python=/path/to/python3.11] [--index-url=依赖索引]",
  );
const python =
  args.find((a) => a.startsWith("--python="))?.slice(9) ?? pythonCommand();
const run = (exe, argv) =>
  finished(spawn(exe, argv, { cwd: root, stdio: "inherit" }));
await run(python, [
  "-c",
  'import sys; assert (3,11) <= sys.version_info < (3,14), "Python 3.11-3.13 required"',
]);
const runtime = resolve(root, ".local/runtime/visual-worker");
await run(python, ["-m", "venv", runtime]);
const executable = venvPython(runtime);
await run(executable, [
  "-m",
  "pip",
  "install",
  "torch==2.11.0",
  "torchvision==0.26.0",
  "--index-url",
  args.includes("--cpu")
    ? "https://download.pytorch.org/whl/cpu"
    : "https://download.pytorch.org/whl/cu128",
]);
await run(executable, [
  "-m",
  "pip",
  "install",
  resolve(root, "services/visual-worker") +
    (args.includes("--dev") ? "[dev]" : ""),
  "--constraint",
  resolve(root, "services/visual-worker/constraints.txt"),
  ...args.filter((a) => a.startsWith("--index-url=")),
]);
await run(executable, [resolve(root, "tooling/setup-visual-architecture.py")]);
console.log(`视觉运行环境：${executable}`);
