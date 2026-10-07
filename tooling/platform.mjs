import { resolve } from "node:path";

export const executableName = (name, platform = process.platform) =>
  name + (platform === "win32" ? ".exe" : "");

export function duckdbLibraryName(platform = process.platform) {
  if (platform === "win32") return "duckdb.dll";
  if (platform === "linux") return "libduckdb.so";
  throw new Error(`尚未支持的平台：${platform}`);
}

export const duckdbLibrary = (root) =>
  process.env.STUDIO_DUCKDB_LIBRARY ??
  process.env.STUDIO_DUCKDB_DLL ??
  resolve(root, "vendor/duckdb", duckdbLibraryName());

export const pythonCommand = () =>
  process.env.PYTHON ?? (process.platform === "win32" ? "python" : "python3");

export const venvPython = (directory) =>
  resolve(
    directory,
    process.platform === "win32" ? "Scripts/python.exe" : "bin/python",
  );

export const browserOptions = () =>
  process.platform === "win32" ? { channel: "msedge" } : {};
