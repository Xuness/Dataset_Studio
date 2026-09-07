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
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { finished } from "./cargo.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const dataDir = resolve(
  process.env.STUDIO_DATA_DIR ?? resolve(root, ".local/dev"),
);
const local = resolve(root, ".local");
await mkdir(local, { recursive: true });
await mkdir(dataDir, { recursive: true });
const env = {
  ...process.env,
  STUDIO_DATA_DIR: dataDir,
  STUDIO_DEVELOPMENT: "1",
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
    const native = resolve(root, "target/debug/studio-desktop.exe");
    if (existsSync(native) && !process.argv.includes("--web")) {
      const focus = spawn(native, [], {
        env,
        stdio: "ignore",
        windowsHide: true,
      });
      focus.unref();
    }
    console.log("开发环境已在运行，请使用已有窗口或开发控制台。");
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
      await finished(
        spawn(
          process.execPath,
          [resolve(root, "tooling/prepare-sidecar.mjs")],
          { cwd: root, env, stdio: "inherit", windowsHide: true },
        ),
      );
      await finished(
        spawn(process.execPath, [resolve(root, "tooling/contracts.mjs")], {
          cwd: root,
          env,
          stdio: "inherit",
          windowsHide: true,
        }),
      );
      const bytes = await readFile(
        resolve(root, "target/debug/studio-engine.exe"),
      );
      const fingerprint = createHash("sha256").update(bytes).digest("hex");
      const folder = resolve(local, "engine-binaries", fingerprint);
      await mkdir(folder, { recursive: true });
      const binary = resolve(folder, "studio-engine.exe");
      if (!existsSync(binary))
        await copyFile(resolve(root, "target/debug/studio-engine.exe"), binary);
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
        });
        if (!response.ok)
          throw new Error("无法停止旧开发引擎，请先关闭旧版本。");
        for (let i = 0; i < 60 && (await live()); i++) await sleep(150);
        if (await live())
          throw new Error("旧引擎仍在处理请求；请稍后重试启动。");
      }
      if (!(await live())) {
        let engine;
        let launches = 0;
        for (let i = 0; i < 60 && !(await live()); i++) {
          if ((!engine || engine.exitCode !== null) && launches < 5) {
            const log = await open(resolve(dataDir, "engine.log"), "a");
            engine = spawn(binary, ["serve", "--data-dir", dataDir], {
              env,
              cwd: root,
              stdio: ["ignore", log.fd, log.fd],
              windowsHide: true,
              detached: true,
            });
            engine.unref();
            await log.close();
            launches++;
          }
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
        JSON.stringify({ fingerprint, instance_id: ready.instance_id }),
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
    if (!name || !/\.(rs|toml)$/.test(String(name))) return;
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
