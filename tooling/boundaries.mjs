import { readdir, readFile } from "node:fs/promises";
import { resolve, relative } from "node:path";
import { fileURLToPath } from "node:url";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
async function files(path) {
  const entries = await readdir(path, { withFileTypes: true });
  return (
    await Promise.all(
      entries
        .filter((e) => !["node_modules", "dist", "target"].includes(e.name))
        .map((e) =>
          e.isDirectory()
            ? files(resolve(path, e.name))
            : [resolve(path, e.name)],
        ),
    )
  ).flat();
}
const violations = [];
for (const file of await files(resolve(root, "apps/desktop/src/features"))) {
  if (!/\.tsx?$/.test(file)) continue;
  const text = await readFile(file, "utf8");
  if (/@tauri-apps|(?:^|\W)fetch\s*\(/m.test(text))
    violations.push(
      relative(root, file) + ": 功能模块必须通过 SDK 访问后端和平台",
    );
}
for (const name of ["studio-domain", "studio-application"]) {
  const text = await readFile(
    resolve(root, "crates", name, "Cargo.toml"),
    "utf8",
  );
  if (
    /^(tauri|axum|rusqlite|studio-storage|studio-engine|studio-protocol)\s*=/m.test(
      text,
    )
  )
    violations.push(name + ": 核心层依赖了基础设施");
}
if (violations.length) throw new Error(violations.join("\n"));
console.log("模块依赖边界检查通过。");
