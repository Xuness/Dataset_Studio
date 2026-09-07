import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";
import { setImmediate } from "node:timers";
const { DOMException, Response, Blob } = globalThis;
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const output = resolve(root, ".local/client-media");
await mkdir(output, { recursive: true });
const code = ts.transpileModule(
  await readFile(resolve(root, "packages/client/src/media.ts"), "utf8"),
  {
    compilerOptions: {
      target: ts.ScriptTarget.ES2022,
      module: ts.ModuleKind.ESNext,
    },
  },
).outputText;
const modulePath = resolve(output, "media.mjs");
await writeFile(modulePath, code);
const { MediaClient } = await import(
  pathToFileURL(modulePath).href + "?" + Date.now()
);
const realFetch = globalThis.fetch;
const calls = [],
  cancellations = [],
  unhandled = [];
const unhandledListener = (error) => unhandled.push(error);
process.on("unhandledRejection", unhandledListener);
let ignoreAbort = false,
  holdCancel = null;
globalThis.fetch = (url, init) =>
  new Promise((resolve, reject) => {
    const request = { url, init, resolve, reject, aborts: 0 };
    calls.push(request);
    init.signal.addEventListener("abort", () => {
      request.aborts++;
      if (!ignoreAbort) reject(new DOMException("cancelled", "AbortError"));
    });
  });
const client = new MediaClient(
  { endpoint: "http://127.0.0.1:1", token: "fixture" },
  async (path, init) => {
    cancellations.push({ path, init });
    if (holdCancel) await holdCancel;
    return { ok: true };
  },
);
const asset = {
  key: { source_id: crypto.randomUUID(), asset_id: "sample-0001" },
  name: "fixture",
  bytes: "0",
  extension: "png",
  source_name: "fixture",
  selected: false,
};
const p1 = crypto.randomUUID(),
  p2 = crypto.randomUUID();
const respond = (call, offline = false) =>
  call.resolve(
    new Response(new Blob(["preview"]), {
      headers: {
        "content-type": "image/jpeg",
        "x-studio-freshness": offline ? "offline_cached" : "verified",
        "x-studio-verified-ms": "1234",
      },
    }),
  );
const tick = () => new Promise((done) => setImmediate(done));
const checks = [];
try {
  const a = new AbortController(),
    b = new AbortController();
  const first = client.acquire(p1, asset, 360, { signal: a.signal });
  const second = client.acquire(p1, asset, 360, { signal: b.signal });
  assert.equal(calls.length, 1);
  const firstRejected = assert.rejects(first, { name: "AbortError" });
  a.abort();
  await firstRejected;
  assert.equal(calls[0].aborts, 0);
  assert.equal(cancellations.length, 0);
  respond(calls[0], true);
  const image = await second;
  assert.equal(image.offline, true);
  assert.equal(image.verifiedMs, 1234);
  image.release();
  const memoryAbort = new AbortController();
  const memory = await client.acquire(p1, asset, 360, {
    signal: memoryAbort.signal,
  });
  assert.equal(memory.url, image.url);
  memoryAbort.abort();
  memory.release();
  checks.push(
    "same-priority consumers merge; one cancellation preserves the other; memory handle abort is safe",
  );

  let releaseCancel;
  holdCancel = new Promise((done) => {
    releaseCancel = done;
  });
  const stop = new AbortController();
  const pending = client.acquire(p1, asset, 720, { signal: stop.signal });
  const stopped = assert.rejects(pending, { name: "AbortError" });
  stop.abort();
  await stopped;
  assert.equal(calls[1].aborts, 1);
  assert.equal(cancellations.length, 1);
  assert.match(cancellations[0].path, /\/read-requests\/[0-9a-f-]+\/cancel$/);
  let closed = false;
  const clearing = client.clear(p1).then(() => {
    closed = true;
  });
  await tick();
  assert.equal(closed, false);
  releaseCancel();
  await clearing;
  holdCancel = null;
  checks.push(
    "last subscriber aborts transport and sends one explicit cancellation; closing waits for its acknowledgement",
  );

  const prefetch = client.acquire(p1, asset, 400, { priority: "prefetch" });
  const visible = client.acquire(p1, asset, 400, { priority: "interactive" });
  assert.equal(calls.length, 4);
  assert.match(calls[2].url, /priority=prefetch/);
  assert.match(calls[3].url, /priority=interactive/);
  respond(calls[2]);
  respond(calls[3]);
  const [prefetched, shown] = await Promise.all([prefetch, visible]);
  assert.equal(prefetched.url, shown.url);
  prefetched.release();
  shown.release();
  const crossProject = client.acquire(p2, asset, 400);
  assert.equal(calls.length, 5);
  respond(calls[4]);
  (await crossProject).release();
  checks.push(
    "foreground attaches separately to promote shared server work; object URL is reused; project cache keys remain isolated",
  );

  ignoreAbort = true;
  const lateAbort = new AbortController();
  const late = client.acquire(p1, asset, 900, { signal: lateAbort.signal });
  const lateRejected = assert.rejects(late, { name: "AbortError" });
  lateAbort.abort();
  await lateRejected;
  respond(calls[5]);
  await tick();
  const retried = client.acquire(p1, asset, 900);
  assert.equal(calls.length, 7);
  respond(calls[6]);
  (await retried).release();
  await tick();
  assert.deepEqual(unhandled, []);
  checks.push(
    "late transport completion after cancellation never populates the preview cache",
  );
  await writeFile(
    resolve(output, "report.json"),
    JSON.stringify({ passed: true, checks }, null, 2),
  );
  console.log(
    JSON.stringify(
      { passed: true, checks, report: resolve(output, "report.json") },
      null,
      2,
    ),
  );
} finally {
  client.dispose();
  await tick();
  globalThis.fetch = realFetch;
  process.off("unhandledRejection", unhandledListener);
}
