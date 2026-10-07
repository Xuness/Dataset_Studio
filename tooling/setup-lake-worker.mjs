import { spawn } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { finished } from "./cargo.mjs";
import { pythonCommand, venvPython } from "./platform.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const args = process.argv.slice(2);
const python =
  args.find((a) => a.startsWith("--python="))?.slice(9) ?? pythonCommand();
if (args.some((a) => a !== "--dev" && !a.startsWith("--python=")))
  throw new Error(
    "用法：pnpm setup:lake [--dev] [--python=/path/to/python3.13]",
  );
await finished(
  spawn(
    python,
    [
      "-c",
      'import sys; assert (3,11) <= sys.version_info < (3,14), "Python 3.11-3.13 is required"',
    ],
    { stdio: "inherit" },
  ),
);
const runtime = resolve(root, ".local/runtime/lake-worker");
await finished(spawn(python, ["-m", "venv", runtime], { stdio: "inherit" }));
const executable = venvPython(runtime);
await finished(
  spawn(
    executable,
    [
      "-m",
      "pip",
      "install",
      resolve(root, "services/lake-worker") +
        (args.includes("--dev") ? "[dev]" : ""),
    ],
    { stdio: "inherit" },
  ),
);
console.log(`数据湖运行环境：${executable}`);
