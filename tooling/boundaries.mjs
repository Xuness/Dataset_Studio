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
    /^(tauri|axum|reqwest|rusqlite|studio-llm|studio-storage|studio-sources|studio-resources|studio-engine|studio-protocol)\s*=/m.test(
      text,
    )
  )
    violations.push(name + ": 核心层依赖了基础设施");
}
// Source I/O is assembled once and reached through the admitted service.
for (const file of await files(resolve(root, "crates/studio-engine/src"))) {
  const local = relative(root, file).replaceAll("\\", "/");
  if (
    !file.endsWith(".rs") ||
    /(?:^|\/)(?:tests|[^/]*_tests)\.rs$/.test(local) ||
    local.includes("/tests/")
  )
    continue;
  const text = await readFile(file, "utf8");
  if (
    /\bSourceRouter\b|\b(?:QueryReader|MetadataReader|RankingReader)::/.test(
      text,
    )
  )
    violations.push(
      local + ": 来源读取必须经过 SourceService，具体 Reader 由后端注册表装配",
    );
}
if (violations.length) throw new Error(violations.join("\n"));
console.log("模块依赖边界检查通过。");
