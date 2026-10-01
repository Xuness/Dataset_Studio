import { spawn } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { lakeWorkerPython } from "./lake-worker-runtime.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const python = lakeWorkerPython(root);
if (!python)
  throw new Error(
    "请先安装 Studio 更新服务环境，或设置 STUDIO_LAKE_TEST_PYTHON。",
  );
const [command, ...args] = process.argv.slice(2);
const retirement = ["handoff", "retire"].includes(command);
if (
  !retirement &&
  ![
    "build",
    "verify",
    "compare",
    "release",
    "activate",
    "cleanup",
    "adopt-legacy-bindings",
  ].includes(command)
)
  throw new Error(
    "命令：build、verify、compare、release、activate、cleanup、adopt-legacy-bindings、handoff、retire。",
  );
const module = `studio_lake.${retirement ? "producer_retirement" : "archive_rebuild"}`;
const bootstrap =
  "import runpy,sys;sys.path.insert(0,sys.argv.pop(1));runpy.run_module(sys.argv.pop(1),run_name='__main__',alter_sys=True)";
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
    module,
    command,
    ...args,
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
