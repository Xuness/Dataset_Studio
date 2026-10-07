import { existsSync } from "node:fs";
import { resolve } from "node:path";
import { venvPython } from "./platform.mjs";
export function lakeWorkerPython(root) {
  if (process.env.STUDIO_LAKE_TEST_PYTHON)
    return process.env.STUDIO_LAKE_TEST_PYTHON;
  const local = venvPython(resolve(root, ".local/runtime/lake-worker"));
  return existsSync(local) ? local : undefined;
}
