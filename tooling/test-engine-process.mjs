import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { existsSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { processExists, waitForEngineExit } from "./engine-process.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local/test-runs",
  "engine-process-test-" + Date.now(),
);
await mkdir(run, { recursive: true });
const checks = [];
let child;
try {
  assert.equal(processExists(process.pid), true);
  for (const pid of [undefined, null, -1, 0, 1.5])
    assert.equal(processExists(pid), false);
  checks.push(
    "Process checks reject invalid PIDs without signalling a process",
  );

  let elapsed = 0;
  let launches = 0;
  const notices = [];
  await waitForEngineExit(42, {
    exists: () => elapsed < 25000,
    now: () => elapsed,
    sleep: async (ms) => {
      assert.equal(
        launches,
        0,
        "No replacement while shutdown still owns its lock",
      );
      elapsed += ms;
    },
    onWaiting: (ms) => notices.push(ms),
  });
  launches++;
  assert.equal(elapsed, 25000);
  assert.equal(launches, 1);
  assert.deepEqual(notices, [0, 5000, 10000, 15000, 20000]);
  checks.push(
    "A 25-second shutdown exceeds the former startup retry window without launching competing engines",
  );

  elapsed = 0;
  await assert.rejects(
    waitForEngineExit(42, {
      exists: () => true,
      timeoutMs: 1000,
      now: () => elapsed,
      sleep: async (ms) => {
        elapsed += ms;
      },
    }),
    /PID 42.*仍在退出收尾/,
  );
  assert.equal(elapsed, 1000);
  await waitForEngineExit(42, {
    exists: () => false,
    sleep: async () => assert.fail("An exited process needs no delay"),
  });
  checks.push(
    "Stuck exits fail with a bounded, specific error; dead PIDs return immediately",
  );

  const marker = resolve(run, "owned.lock");
  child = spawn(
    process.execPath,
    [
      "--input-type=module",
      "-e",
      `
import { createServer } from 'node:http';
import { writeFileSync, unlinkSync } from 'node:fs';
const marker = process.argv[1];
writeFileSync(marker, String(process.pid), { flag: 'wx' });
const server = createServer((request, response) => {
  response.end('ok');
  if (request.url === '/shutdown') {
    server.close(() => {
      process.send({ closed: true });
      setTimeout(() => {
        unlinkSync(marker);
        process.exit(0);
      }, 1500);
    });
  }
});
server.listen(0, '127.0.0.1', () => process.send({
  endpoint: 'http://127.0.0.1:' + server.address().port,
}));
`,
      marker,
    ],
    {
      cwd: run,
      windowsHide: true,
      stdio: ["ignore", "ignore", "inherit", "ipc"],
    },
  );
  const exited = once(child, "exit");
  const [{ endpoint }] = await once(child, "message");
  const closed = once(child, "message");
  await (await fetch(endpoint + "/shutdown", { method: "POST" })).text();
  assert.equal((await closed)[0].closed, true);
  assert.equal(processExists(child.pid), true);
  assert.equal(existsSync(marker), true);
  await assert.rejects(fetch(endpoint, { signal: AbortSignal.timeout(250) }));
  await waitForEngineExit(child.pid, { timeoutMs: 10000 });
  assert.equal(processExists(child.pid), false);
  assert.equal(existsSync(marker), false);
  assert.equal((await exited)[0], 0);
  checks.push(
    "A real child closes HTTP before delayed cleanup; shutdown waits until its process exits and the ownership marker is released",
  );

  const report = resolve(run, "report.json");
  await writeFile(report, JSON.stringify({ passed: true, checks }, null, 2));
  console.log(JSON.stringify({ passed: true, checks, report }, null, 2));
} finally {
  if (child && child.exitCode === null) child.kill();
}
