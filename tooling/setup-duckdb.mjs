import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { mkdir, rename, unlink } from "node:fs/promises";
import { spawn } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { finished } from "./cargo.mjs";
import { duckdbLibraryName, pythonCommand } from "./platform.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const assets = {
  win32: [
    "windows",
    "74e73afd3b010c6f310e14a961fff679f876952bc196f82584b0e2e76d11a91f",
  ],
  linux: [
    "linux",
    "838d98a85e697bab9935010c88a8c67d3312ccedcab4cb4a0ba01da65113bb70",
  ],
};
if (process.arch !== "x64" || !assets[process.platform])
  throw new Error("DuckDB 开发运行库目前支持 Windows / Linux x86_64。");
const [platform, expected] = assets[process.platform];
const folder = resolve(root, "vendor/duckdb");
await mkdir(folder, { recursive: true });
const archive = resolve(folder, `libduckdb-${platform}-amd64.zip`);
async function digest(path) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest("hex");
}
if ((await digest(archive).catch(() => null)) !== expected) {
  const legacy = resolve(folder, "duckdb.zip");
  if (
    process.platform === "win32" &&
    (await digest(legacy).catch(() => null)) === expected
  ) {
    await rename(legacy, archive);
  } else {
    const temporary = archive + `.${process.pid}.part`;
    try {
      await finished(
        spawn(
          process.platform === "win32" ? "curl.exe" : "curl",
          [
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--retry",
            "3",
            "--connect-timeout",
            "30",
            "--max-time",
            "180",
            "--output",
            temporary,
            `https://github.com/duckdb/duckdb/releases/download/v1.5.4/libduckdb-${platform}-amd64.zip`,
          ],
          { stdio: "inherit" },
        ),
      );
      if ((await digest(temporary)) !== expected)
        throw new Error("DuckDB archive checksum mismatch.");
      await rename(temporary, archive);
    } finally {
      await unlink(temporary).catch(() => {});
    }
  }
}
// Extract only the known library. Keep matching files in place, including a
// Windows DLL already loaded by another engine.
await finished(
  spawn(
    pythonCommand(),
    [
      "-c",
      `
import hashlib, os, pathlib, sys, zipfile
archive, folder, name = sys.argv[1:]
with zipfile.ZipFile(archive) as bundle:
    data = bundle.read(name)
path = pathlib.Path(folder) / name
if not path.is_file() or hashlib.sha256(path.read_bytes()).digest() != hashlib.sha256(data).digest():
    temporary = path.with_name(name + '.' + str(os.getpid()) + '.tmp')
    try:
        temporary.write_bytes(data)
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)
`,
      archive,
      folder,
      duckdbLibraryName(),
    ],
    { stdio: "inherit" },
  ),
);
console.log("DuckDB 1.5.4 metadata runtime is ready.");
