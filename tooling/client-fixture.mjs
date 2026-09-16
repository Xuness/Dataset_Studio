import { mkdir, readFile, writeFile, readdir } from "node:fs/promises";
import { resolve, dirname } from "node:path";
import { pathToFileURL } from "node:url";
import ts from "typescript";

// Compile the whole SDK directory so independently organized client modules are exercised.
export async function clientFixture(root, output) {
  await mkdir(output, { recursive: true });
  await writeFile(resolve(output, "package.json"), '{"type":"module"}');
  const source = resolve(root, "packages/client/src");
  for (const file of await readdir(source, { recursive: true })) {
    if (!file.endsWith(".ts") || file.endsWith(".d.ts")) continue;
    const compiled = ts.transpileModule(
      await readFile(resolve(source, file), "utf8"),
      {
        compilerOptions: {
          target: ts.ScriptTarget.ES2022,
          module: ts.ModuleKind.ESNext,
        },
      },
    ).outputText;
    const target = resolve(output, file.slice(0, -3) + ".js");
    await mkdir(dirname(target), { recursive: true });
    await writeFile(target, compiled);
  }
  return import(pathToFileURL(resolve(output, "index.js")).href);
}
