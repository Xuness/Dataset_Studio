// Exercise the public source launcher and actual application in an isolated desktop.
import assert from "node:assert/strict";
import { spawn, execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, open, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { sleep } from "./engine-fixture.mjs";

assert.equal(process.platform, "linux");
assert.equal(
  process.env.STUDIO_LINUX_TEST_SESSION,
  "1",
  "Use tooling/linux-test-session.sh",
);
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(root, ".local/test-runs", "linux-dev-" + Date.now());
const data = resolve(run, "state");
await mkdir(data, { recursive: true });
const execute = promisify(execFile);
const log = await open(resolve(run, "startup.log"), "a");
const child = spawn(
  process.execPath,
  [resolve(root, "tooling/dev.mjs"), "--engine-profile=debug"],
  {
    cwd: root,
    stdio: ["ignore", log.fd, log.fd],
    env: { ...process.env, STUDIO_DATA_DIR: data },
  },
);
await log.close();
let connection, window;
try {
  for (let i = 0; i < 1200; i++) {
    if (child.exitCode !== null)
      throw new Error("Source launcher exited; inspect startup.log");
    try {
      connection = JSON.parse(
        await readFile(resolve(data, "engine.json"), "utf8"),
      );
      const health = await fetch(connection.endpoint + "/v1/health", {
        headers: { authorization: "Bearer " + connection.token },
        signal: AbortSignal.timeout(500),
      }).then((r) => r.json());
      assert.equal(health.instance_id, connection.instance_id);
      const workspace = await fetch(
        "http://127.0.0.1:1420/__studio/workspace",
        { signal: AbortSignal.timeout(500) },
      ).then((r) => r.json());
      assert.equal(workspace.workspace, root);
      const { stdout } = await execute("xdotool", [
        "search",
        "--onlyvisible",
        "--name",
        "^Dataset Studio$",
      ]);
      window = stdout.trim().split("\n").at(-1);
      if (window) break;
    } catch {
      /* Await this source launch, including incremental compilation. */
    }
    await sleep(100);
  }
  assert.ok(window, "Actual application window did not appear");
  const resources = await fetch(connection.endpoint + "/v1/resources", {
    headers: { authorization: "Bearer " + connection.token },
  }).then((r) => r.json());
  assert.ok(
    Number(resources.process_memory?.resident_bytes) > 0,
    "Linux process memory is missing",
  );
  await sleep(1500);
  await execute("import", ["-window", "root", resolve(run, "application.png")]);
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      {
        source_launcher: true,
        authenticated_engine: true,
        native_window: true,
        process_memory: true,
      },
      null,
      2,
    ),
  );
  await execute("xdotool", ["windowactivate", "--sync", window]);
  await execute("xdotool", ["key", "--clearmodifiers", "alt+F4"]);
  window = undefined;
  console.log(JSON.stringify({ run, source_desktop: true }));
} finally {
  if (window) await execute("xdotool", ["windowclose", window]).catch(() => {});
  for (let i = 0; i < 100 && child.exitCode === null; i++) await sleep(100);
  if (child.exitCode === null) child.kill();
  if (connection)
    await fetch(connection.endpoint + "/v1/shutdown", {
      method: "POST",
      headers: { authorization: "Bearer " + connection.token },
      signal: AbortSignal.timeout(10000),
    }).catch(() => {});
}
