import { existsSync } from "node:fs";
import { resolve } from "node:path";
export function lakeWorkerPython(root) {
  if (process.env.STUDIO_LAKE_TEST_PYTHON)
    return process.env.STUDIO_LAKE_TEST_PYTHON;
  const local = resolve(
    root,
    ".local/runtime/lake-worker",
    process.platform === "win32" ? "Scripts/python.exe" : "bin/python",
  );
  return existsSync(local) ? local : undefined;
}
