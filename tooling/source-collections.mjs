import { spawn } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { lakeWorkerPython } from "./lake-worker-runtime.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const python = lakeWorkerPython(root);
if (!python) throw new Error("请先运行 tooling/setup-lake-worker.ps1 -Dev");
const bootstrap =
  "import runpy,sys;sys.path.insert(0,sys.argv.pop(1));runpy.run_module('studio_lake.collections',run_name='__main__',alter_sys=True)";
const child = spawn(
  python,
  [
    "-I",
    "-B",
    "-X",
    "utf8",
    "-c",
    bootstrap,
    resolve(root, "services/lake-worker/src"),
    ...process.argv.slice(2),
  ],
  { cwd: root, stdio: "inherit", windowsHide: true },
);
child.on("error", (error) => {
  console.error(error.message);
  process.exitCode = 1;
});
child.on("exit", (code) => {
  process.exitCode = code ?? 1;
});
