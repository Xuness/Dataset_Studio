import { spawn } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { cargo, finished } from "./cargo.mjs";
import { engineBuildArguments, engineProfile } from "./engine-profile.mjs";

// Keep the full run in dependency order; ordinary runs select explicit suites.
const suites = [
  "foundation",
  "scopes",
  "tools",
  "exports",
  "resources",
  "artifact-scale",
  "query-cache",
  "cache-settings",
  "ranking",
  "ranking-v2",
  "scoped-browse",
  "management",
  "ranking-browse",
  "ranked-cache-lifecycle",
  "ranking-duplicates",
  "cache-inventory",
  "llm",
  "aesthetic-cache",
  "system-prompts",
  "aesthetic",
  "aesthetic-analysis",
  "aesthetic-recovery",
  "aesthetic-transport",
  "aesthetic-execution",
  "aesthetic-sampling",
  "multibooru",
  "online",
  "lake-updates",
  "lake-recovery",
  "collections",
];
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));

async function main() {
  const args = process.argv.slice(2);
  if (args[0] === "--") args.shift();
  if (!args.length || args.includes("--help") || args.includes("--list")) {
    console.log(
      "用法：pnpm test:integration <套件名...> [--dry-run] [--capacity]\n" +
        "完整验收（含千万行容量）：pnpm test:integration:all\n" +
        "可用套件：\n" +
        suites.join("\n"),
    );
    if (!args.length) process.exitCode = 2;
    return;
  }
  const all = args.includes("--all");
  const capacity = all || args.includes("--capacity");
  const flags = new Set(["--all", "--dry-run", "--capacity"]);
  const names = args.filter((arg) => !flags.has(arg));
  for (const name of names) {
    if (!suites.includes(name))
      throw new Error(`未知集成套件：${name}；用 --list 查看名称。`);
  }
  if (all && names.length) throw new Error("--all 不能同时指定套件名。");
  const selected = all ? suites : [...new Set(names)];
  if (!selected.length) throw new Error("请指定至少一个集成套件。");
  if (capacity && !selected.includes("aesthetic-recovery"))
    throw new Error("--capacity 仅用于 aesthetic-recovery。");
  const commands = selected.map((name) => ({
    name,
    script: `tooling/integration${name === "foundation" ? "" : "-" + name}.mjs`,
    args: name === "aesthetic-recovery" && capacity ? ["--capacity"] : [],
  }));
  if (args.includes("--dry-run")) {
    console.log(JSON.stringify(commands, null, 2));
    return;
  }
  // Recovery prepares and restores its own fault-enabled binary. Other suites
  // share one up-to-date build, avoiding tests against a stale executable.
  if (selected.some((name) => name !== "aesthetic-recovery")) {
    await finished(
      cargo(engineBuildArguments(engineProfile([], process.env, "debug")), {
        cwd: root,
        windowsHide: true,
      }),
    );
  }
  for (const command of commands) {
    console.log(
      `[integration] ${command.name}${command.args.length ? " (capacity)" : ""}`,
    );
    await finished(
      spawn(
        process.execPath,
        [resolve(root, command.script), ...command.args],
        {
          cwd: root,
          stdio: "inherit",
          windowsHide: true,
        },
      ),
    );
  }
}

await main().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
