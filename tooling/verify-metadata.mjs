// Opt-in, bounded real-lake API verification. The lake stays read-only; project
// writes and reports live in this run's isolated .local directory.
import assert from "node:assert/strict";
import { spawn, execFile } from "node:child_process";
import { mkdir, open, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs, promisify } from "node:util";
import { performance } from "node:perf_hooks";
const { values } = parseArgs({
  options: {
    "index-root": { type: "string" },
    "media-root": { type: "string" },
    asset: { type: "string", multiple: true },
  },
});
assert.ok(
  values["index-root"] && values["media-root"] && values.asset?.length,
  "Provide --index-root, --media-root and 1–8 --asset SHA256 values",
);
assert.ok(
  values.asset.length <= 8 &&
    values.asset.every((id) => /^[0-9a-f]{64}$/.test(id)),
);
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(root, ".local", "metadata-verification-" + Date.now());
const state = resolve(run, "state");
await mkdir(state, { recursive: true });
const log = await open(resolve(run, "engine.log"), "w");
const child = spawn(
  resolve(root, "target/debug/studio-engine.exe"),
  ["serve", "--data-dir", state],
  { stdio: ["ignore", log.fd, log.fd], windowsHide: true },
);
await log.close();
let connection;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function api(path, body) {
  const response = await fetch(connection.endpoint + path, {
    method: body ? "POST" : "GET",
    headers: {
      Authorization: "Bearer " + connection.token,
      ...(body ? { "Content-Type": "application/json" } : {}),
    },
    ...(body ? { body: JSON.stringify(body) } : {}),
    signal: AbortSignal.timeout(12_000),
  });
  const json = await response.json();
  assert.equal(response.status, 200, JSON.stringify(json));
  return json;
}
try {
  for (let i = 0; i < 80; i++) {
    try {
      const c = JSON.parse(
        await readFile(resolve(state, "engine.json"), "utf8"),
      );
      if (c.pid === child.pid) {
        connection = c;
        await api("/v1/health");
        break;
      }
    } catch {
      /* startup */
    }
    if (child.exitCode !== null)
      throw new Error("Engine exited; inspect " + resolve(run, "engine.log"));
    await sleep(100);
  }
  assert.ok(connection);
  const project = await api("/v1/projects", {
    name: "真实元数据验证",
    parent_directory: null,
  });
  const base = "/v1/projects/" + project.id;
  const source = await api(base + "/sources", {
    name: "只读 Danbooru",
    kind: "danbooru",
    index_root: values["index-root"],
    media_root: values["media-root"],
  });
  const before = await api(base + "/selection");
  const revision = (await api(base)).revision;
  const samples = [];
  for (const asset of values.asset) {
    const path = base + "/sources/" + source.id + "/assets/" + asset;
    for (let pass = 0; pass < 2; pass++) {
      let start = performance.now();
      const metadata = await api(path + "/metadata");
      const metadataMs = performance.now() - start;
      assert.equal(metadata.object.key.asset_id, asset);
      assert.equal(metadata.stored_width, null);
      assert.equal(
        metadata.version.catalog_sequence,
        metadata.version.analysis_sequence,
      );
      const summary = {
        asset,
        pass,
        metadataMs,
        version: metadata.version,
        recordsOnPage: metadata.records.length,
        moreRecords: !!metadata.next_cursor,
      };
      if (metadata.records[0]) {
        const record = metadata.records[0];
        const observationsPath =
          path + "/records/" + record.record_id + "/observations";
        const query = "?version=" + encodeURIComponent(metadata.version.token);
        start = performance.now();
        const observations = await api(observationsPath + query);
        summary.observationsMs = performance.now() - start;
        summary.observationsOnPage = observations.items.length;
        const observation =
          observations.items.find((o) => o.relation === "asset_origin") ??
          observations.items[0];
        if (observation) {
          start = performance.now();
          const raw = await api(
            observationsPath +
              "/" +
              observation.observation_id +
              "/raw" +
              query,
          );
          summary.rawMs = performance.now() - start;
          summary.observationId = observation.observation_id;
          summary.postId = observation.post_id;
          summary.fields = Object.fromEntries(
            observation.fields
              .filter((f) =>
                ["source_width", "source_height", "rating"].includes(f.name),
              )
              .map((f) => [f.name, f.value]),
          );
          summary.raw = {
            status: raw.status,
            bytes: raw.bytes,
            format: raw.format,
          };
        }
      }
      samples.push(summary);
    }
  }
  assert.deepEqual(await api(base + "/selection"), before);
  assert.equal((await api(base)).revision, revision);
  let memory = null;
  if (process.platform === "win32") {
    const result = await promisify(execFile)(
      "pwsh.exe",
      [
        "-NoProfile",
        "-Command",
        "Get-Process -Id " +
          child.pid +
          " | Select-Object WorkingSet64,PrivateMemorySize64,PeakWorkingSet64 | ConvertTo-Json -Compress",
      ],
      { windowsHide: true },
    );
    memory = JSON.parse(result.stdout);
  }
  const report = {
    verifiedAt: new Date().toISOString(),
    projectId: project.id,
    memory,
    samples,
    selectionUnchanged: true,
    boundary:
      "Per-request read transactions and matched watermarks; first API pass is not an OS-cold-cache test; bounded samples are not a full-lake performance qualification.",
  };
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(report, null, 2) + "\n",
  );
  console.log(
    JSON.stringify({
      report: resolve(run, "report.json"),
      samples: samples.length,
      metadataMs: samples.map((s) => Math.round(s.metadataMs * 10) / 10),
      memory,
    }),
  );
} finally {
  if (child.exitCode === null) {
    const exited = new Promise((r) => child.once("exit", r));
    child.kill();
    await exited;
  }
}
