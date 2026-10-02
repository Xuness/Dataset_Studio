import {
  mkdir,
  readFile,
  writeFile,
  open,
  copyFile,
  unlink,
} from "node:fs/promises";
import { watch, existsSync } from "node:fs";
import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { fileURLToPath } from "node:url";
import { finished } from "./cargo.mjs";
import { engineProfile, engineExecutable } from "./engine-profile.mjs";
import { engineWaitTimeoutMs, waitForEngineExit } from "./engine-process.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const dataDir = resolve(
  process.env.STUDIO_DATA_DIR ?? resolve(root, ".local/dev"),
);
const local = resolve(root, ".local");
const profile = engineProfile();
let prebuilt = process.argv
  .find((arg) => arg.startsWith("--prebuilt-engine="))
  ?.slice("--prebuilt-engine=".length);
if (prebuilt && !existsSync(resolve(prebuilt)))
  throw new Error("指定的已验证引擎不存在。");
await mkdir(local, { recursive: true });
await mkdir(dataDir, { recursive: true });
const env = {
  ...process.env,
  STUDIO_DATA_DIR: dataDir,
  STUDIO_DEVELOPMENT: "1",
  STUDIO_ENGINE_PROFILE: profile,
  STUDIO_DUCKDB_DLL:
    process.env.STUDIO_DUCKDB_DLL ?? resolve(root, "vendor/duckdb/duckdb.dll"),
};
if (process.platform === "win32" && !env.INCLUDE) {
  delete env.CC;
  delete env.CXX;
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const lockPath = resolve(local, "development-session.json");
async function json(path) {
  try {
    return JSON.parse(await readFile(path, "utf8"));
  } catch {
    return null;
  }
}
async function live() {
  const connection = await json(resolve(dataDir, "engine.json"));
  if (!connection || !/^http:\/\/127\.0\.0\.1:\d+$/.test(connection.endpoint))
    return null;
  try {
    const response = await fetch(connection.endpoint + "/v1/health", {
      headers: { Authorization: "Bearer " + connection.token },
      signal: AbortSignal.timeout(800),
    });
    const health = await response.json();
    return response.ok &&
      health.instance_id === connection.instance_id &&
      health.api_version === 1
      ? connection
      : null;
  } catch {
    return null;
  }
}
const prior = await json(lockPath);
if (prior?.workspace === root && Number.isInteger(prior.pid)) {
  let running = false;
  try {
    process.kill(prior.pid, 0);
    running = true;
  } catch {
    /* Stale development session. */
  }
  if (running) {
    if (process.argv.includes("--web")) {
      console.log("已有开发环境，浏览器入口：http://127.0.0.1:1420");
    } else {
      const native = resolve(root, "target/debug/studio-desktop.exe");
      if (!existsSync(native))
        throw new Error(
          "未找到桌面程序。请先运行 node tooling/cargo-run.mjs build -p studio-desktop，再重新启动。",
        );
      const logDirectory = resolve(local, "logs");
      await mkdir(logDirectory, { recursive: true });
      const log = await open(resolve(logDirectory, "desktop-startup.log"), "a");
      try {
        const focus = spawn(native, [], {
          cwd: root,
          env,
          stdio: ["ignore", log.fd, log.fd],
          windowsHide: true,
          // Debug desktop builds use the console subsystem on Windows. Keep
          // them independent of the short-lived double-click launcher console.
          detached: true,
        });
        await once(focus, "spawn");
        focus.unref();
      } finally {
        await log.close();
      }
      console.log("已复用开发环境，正在打开桌面窗口。");
    }
    process.exit(0);
  }
}
await writeFile(
  lockPath,
  JSON.stringify({ workspace: root, pid: process.pid }),
);
let stopped = false;
let dirty = false;
let building = false;
let desktop;
let vite;
let debounce;
const watchers = [];
async function buildAndConnect() {
  if (building) {
    dirty = true;
    return;
  }
  building = true;
  try {
    do {
      dirty = false;
      console.log("检查本机引擎与接口契约…");
      const supplied = prebuilt ? resolve(prebuilt) : null;
      prebuilt = undefined;
      if (!supplied)
        await finished(
          spawn(
            process.execPath,
            [
              resolve(root, "tooling/prepare-sidecar.mjs"),
              `--engine-profile=${profile}`,
            ],
            { cwd: root, env, stdio: "inherit", windowsHide: true },
          ),
        );
      await finished(
        spawn(
          process.execPath,
          [
            resolve(root, "tooling/contracts.mjs"),
            ...(supplied ? [`--engine=${supplied}`, "--check"] : []),
          ],
          {
            cwd: root,
            env,
            stdio: "inherit",
            windowsHide: true,
          },
        ),
      );
      const executable = supplied ?? engineExecutable(root, profile);
      const bytes = await readFile(executable);
      const fingerprint = createHash("sha256").update(bytes).digest("hex");
      const folder = resolve(local, "engine-binaries", fingerprint);
      await mkdir(folder, { recursive: true });
      const binary = resolve(folder, "studio-engine.exe");
      if (!existsSync(binary)) await copyFile(executable, binary);
      env.STUDIO_ENGINE_PATH = binary;
      const current = await live();
      const built = await json(resolve(dataDir, "development-build.json"));
      if (
        current &&
        (built?.fingerprint !== fingerprint ||
          built?.instance_id !== current.instance_id)
      ) {
        console.log("后端已更新，正在保存检查点并重启开发引擎…");
        const response = await fetch(current.endpoint + "/v1/shutdown", {
          method: "POST",
          headers: { Authorization: "Bearer " + current.token },
          signal: AbortSignal.timeout(10000),
        });
        if (!response.ok)
          throw new Error("无法停止旧开发引擎，请先关闭旧版本。");
        // HTTP can stop before workers, SQLite checkpoints and the directory
        // lock are released. Do not spend startup retries racing that cleanup.
        await waitForEngineExit(current.pid, {
          onWaiting: (elapsed) =>
            console.log(
              `等待旧引擎完成退出收尾… ${Math.floor(elapsed / 1000)} 秒`,
            ),
        });
      }
      if (!(await live())) {
        let engine;
        let launches = 0;
        const deadline = performance.now() + engineWaitTimeoutMs;
        let nextLaunch = 0;
        while (performance.now() < deadline && !(await live())) {
          if (
            (!engine || engine.exitCode !== null) &&
            launches < 5 &&
            performance.now() >= nextLaunch
          ) {
            const log = await open(resolve(dataDir, "engine.log"), "a");
            engine = spawn(binary, ["serve", "--data-dir", dataDir], {
              env,
              cwd: root,
              stdio: ["ignore", log.fd, log.fd],
              windowsHide: true,
              detached: true,
            });
            try {
              await once(engine, "spawn");
              engine.unref();
            } finally {
              await log.close();
            }
            launches++;
            nextLaunch = performance.now() + 1000;
          }
          if (engine?.exitCode != null && launches >= 5) break;
          await sleep(200);
        }
      }
      const ready = await live();
      if (!ready)
        throw new Error(
          "本机引擎启动失败，请查看 " + resolve(dataDir, "engine.log"),
        );
      await writeFile(
        resolve(dataDir, "development-build.json"),
        JSON.stringify({
          fingerprint,
          instance_id: ready.instance_id,
          profile,
        }),
      );
      console.log("本机引擎就绪。");
    } while (dirty && !stopped);
  } finally {
    building = false;
  }
}
async function stop() {
  if (stopped) return;
  stopped = true;
  clearTimeout(debounce);
  for (const watcher of watchers) watcher.close();
  desktop?.kill();
  vite?.kill();
  const owned = await json(lockPath);
  if (owned?.pid === process.pid) await unlink(lockPath).catch(() => {});
}
try {
  await buildAndConnect();
  let existing = false;
  try {
    const response = await fetch("http://127.0.0.1:1420/__studio/workspace", {
      signal: AbortSignal.timeout(700),
    });
    const info = await response.json();
    if (info.workspace !== root) throw new Error("端口 1420 被其他项目占用。");
    existing = true;
  } catch (error) {
    if (error instanceof Error && error.message.includes("其他项目"))
      throw error;
  }
  if (!existing) {
    vite = spawn(
      process.execPath,
      [
        resolve(root, "apps/desktop/node_modules/vite/bin/vite.js"),
        "--host",
        "127.0.0.1",
      ],
      {
        cwd: resolve(root, "apps/desktop"),
        env,
        stdio: "inherit",
        windowsHide: true,
      },
    );
  }
  for (let i = 0; i < 60; i++) {
    try {
      const response = await fetch("http://127.0.0.1:1420/__studio/workspace");
      const info = await response.json();
      if (info.workspace === root) break;
    } catch {
      /* Wait for Vite. */
    }
    if (i === 59) throw new Error("开发前端启动失败。");
    await sleep(150);
  }
  function changed(_event, name) {
    if (!name || !/\.(rs|toml|py|sql)$/.test(String(name))) return;
    clearTimeout(debounce);
    debounce = setTimeout(
      () =>
        void buildAndConnect().catch((error) =>
          console.error("后端更新失败：", error.message),
        ),
      350,
    );
  }
  watchers.push(
    watch(resolve(root, "crates"), { recursive: true }, changed),
    watch(
      resolve(root, "services/lake-worker/src"),
      { recursive: true },
      (event, name) => {
        if (name && /\.(py|sql)$/.test(String(name))) changed(event, name);
      },
    ),
    watch(resolve(root, "services/lake-worker"), (event, name) => {
      if (name === "worker.py" || name === "pyproject.toml")
        changed(event, name);
    }),
    watch(root, (event, name) => {
      if (name === "Cargo.toml") changed(event, name);
    }),
  );
  if (!process.argv.includes("--web")) {
    desktop = spawn(
      process.execPath,
      [
        resolve(root, "apps/desktop/node_modules/@tauri-apps/cli/tauri.js"),
        "dev",
      ],
      {
        cwd: resolve(root, "apps/desktop"),
        env,
        stdio: "inherit",
        windowsHide: true,
      },
    );
    console.log(
      "开发窗口启动中。前端热更新；Rust 后端修改后自动重启并恢复任务。",
    );
  } else console.log("浏览器开发入口：http://127.0.0.1:1420");
  process.on("SIGINT", () => void stop());
  process.on("SIGTERM", () => void stop());
  if (desktop ?? vite) await finished(desktop ?? vite);
  else await new Promise((resolve) => process.once("SIGINT", resolve));
} finally {
  await stop();
}
