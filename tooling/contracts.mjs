import { readFile, writeFile, mkdir } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";
import openapiTS, { astToString } from "openapi-typescript";
import { cargo, finished } from "./cargo.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
await finished(cargo(["build", "-p", "studio-engine"], { cwd: root }));
await mkdir(resolve(root, ".local"), { recursive: true });
const temporary = resolve(root, ".local/openapi.json");
await finished(
  spawn(
    resolve(root, "target/debug/studio-engine.exe"),
    ["schema", "--output", temporary],
    { stdio: "inherit" },
  ),
);
const schema = await readFile(temporary, "utf8");
const types = astToString(await openapiTS(JSON.parse(schema)));
const targets = [
  ["packages/contracts/openapi.json", schema],
  ["packages/contracts/src/schema.d.ts", types],
];
for (const [path, text] of targets) {
  const full = resolve(root, path);
  await mkdir(resolve(full, ".."), { recursive: true });
  if (process.argv.includes("--check")) {
    if ((await readFile(full, "utf8")) !== text)
      throw new Error("契约生成物与源码不同：" + path);
  } else await writeFile(full, text);
}
console.log("OpenAPI 与 TypeScript 契约一致。");
