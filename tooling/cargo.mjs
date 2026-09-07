import { spawn } from "node:child_process";
export function cargo(args, options = {}) {
  const env = { ...process.env, ...options.env };
  // A bare CC override prevents cc-rs from discovering the MSVC include/library environment.
  if (process.platform === "win32" && !env.INCLUDE) {
    delete env.CC;
    delete env.CXX;
  }
  return spawn("cargo", args, {
    ...options,
    env,
    stdio: options.stdio ?? "inherit",
  });
}
export function finished(child) {
  return new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("exit", (code) =>
      code === 0 ? resolve() : reject(new Error("子进程退出码 " + code)),
    );
  });
}
