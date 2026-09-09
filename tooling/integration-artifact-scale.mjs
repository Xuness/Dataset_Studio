import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { DatabaseSync } from "node:sqlite";
import { randomUUID } from "node:crypto";
import { performance } from "node:perf_hooks";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { EngineFixture } from "./engine-fixture.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const runDir = resolve(
  root,
  ".local",
  "test-runs",
  "integration-artifact-scale-" + Date.now(),
);
const engine = new EngineFixture(root, resolve(runDir, "state"));
const count = 20_000,
  id = randomUUID(),
  checks = [],
  timings = {},
  samples = {};
const json = (path, value) => writeFile(path, JSON.stringify(value, null, 2));
const lake = resolve(runDir, "catalog-fixture"),
  generation = resolve(lake, "indexes/g");
await mkdir(generation, { recursive: true });
await json(resolve(lake, "CURRENT.json"), {
  library_id: id,
  index_version: 1,
  generation: "g",
});
await json(resolve(lake, "library.json"), {
  library_id: id,
  format_version: 1,
  image_format: "uncompressed-pax-tar",
});
const db = new DatabaseSync(resolve(generation, "catalog.sqlite"));
db.exec(
  "CREATE TABLE state(key TEXT PRIMARY KEY,value INTEGER);INSERT INTO state VALUES('seq',1);CREATE TABLE objects(sha256 TEXT PRIMARY KEY,pack_path TEXT,offset INTEGER,length INTEGER,stored_ext TEXT) WITHOUT ROWID;BEGIN",
);
const insert = db.prepare(
  "INSERT INTO objects VALUES(?,'unused.tar',0,?,'webp')",
);
for (let i = 0; i < count; i++) insert.run(i.toString(16).padStart(64, "0"), i);
db.exec("COMMIT");
db.close();
const run = (operator_id, parameters) => ({
  operator_id,
  operator_version: 1,
  parameters_version: 1,
  parameters,
});
async function jobDone(base, id) {
  const response = await engine.wait(
    base + "/jobs",
    (r) =>
      r.items.some(
        (j) =>
          j.id === id &&
          ["succeeded", "failed", "cancelled"].includes(j.status),
      ),
    120000,
  );
  const job = response.items.find((j) => j.id === id);
  assert.equal(job.status, "succeeded", JSON.stringify(job));
  return job;
}
process.on("exit", () => engine.child?.kill());
try {
  await engine.start();
  const p = await engine.api("/v1/projects", "POST", {
      name: "两万行标量成果验证",
    }),
    base = "/v1/projects/" + p.id;
  const source = await engine.api(base + "/sources", "POST", {
    name: "有界合成索引",
    kind: "danbooru",
    index_root: lake,
    media_root: lake,
  });
  samples.before = await engine.api("/v1/resources");
  const begin = performance.now();
  const job = await engine.api(base + "/tools/jobs", "POST", {
    idempotency_key: randomUUID(),
    scope: {
      project_id: p.id,
      target: { kind: "source", source_id: id, revision: source.revision },
    },
    run: run("core.scalar", {
      input: { kind: "stored_bytes" },
      multiplier: "3",
      addend: "-1",
    }),
  });
  const done = await jobDone(base, job.id);
  assert.equal(done.total, count);
  timings.capture_freeze_execute_publish_ms = Math.round(
    performance.now() - begin,
  );
  const artifact = await engine.api(base + "/artifacts/" + job.id);
  assert.equal(artifact.count, count);
  const first = await engine.api(
    base + "/artifacts/" + job.id + "/rows?limit=7",
  );
  assert.equal(first.items.length, 7);
  assert.ok(first.next_cursor);
  assert.deepEqual(
    first.items.map((r) => r.scalar.value),
    Array.from({ length: 7 }, (_, i) => String(i * 3 - 1)),
  );
  const second = await engine.api(
    base +
      "/artifacts/" +
      job.id +
      "/rows?limit=7&cursor=" +
      encodeURIComponent(first.next_cursor),
  );
  assert.deepEqual(
    second.items.map((r) => r.scalar.value),
    Array.from({ length: 7 }, (_, i) => String((i + 7) * 3 - 1)),
  );
  const verified = await engine.api(
    base + "/artifacts/" + job.id + "/verify",
    "POST",
  );
  assert.equal(verified.state, "ready");
  checks.push(
    "20,000 frozen scalar rows publish through the registered worker; seven-row keyset pages and file verification agree",
  );
  const started = performance.now();
  const query = await engine.api(base + "/queries", "POST", {
    name: "派生整数列尾部十项",
    spec: {
      version: 1,
      source_ids: [id],
      conditions: [
        {
          field: "project." + job.id + ".value",
          operator: "gte",
          value: { type: "integer", value: String((count - 10) * 3 - 1) },
        },
      ],
      observation_rule: "any_observation",
      order: "asset_key_asc",
    },
  });
  const result = await engine.api(
    base + "/queries/" + query.id + "/results",
    "POST",
    { expected_revision: query.revision },
  );
  const ready = await engine.wait(
    base + "/query-results/" + result.id,
    (r) => ["ready", "failed"].includes(r.state),
    60000,
  );
  assert.equal(ready.state, "ready", JSON.stringify(ready));
  assert.equal(ready.count, 10);
  const matches = await engine.api(
    base + "/query-results/" + result.id + "/assets?limit=12",
  );
  assert.deepEqual(
    matches.page.items.map((a) => a.key.asset_id),
    Array.from({ length: 10 }, (_, i) =>
      (count - 10 + i).toString(16).padStart(64, "0"),
    ),
  );
  timings.derived_query_ms = Math.round(performance.now() - started);
  const consumed = await engine.api(base + "/tools/jobs", "POST", {
    idempotency_key: randomUUID(),
    scope: {
      project_id: p.id,
      target: { kind: "query_result", result_id: result.id },
    },
    run: run("core.manifest", {
      fields: [{ kind: "artifact", artifact_id: job.id }],
    }),
  });
  await jobDone(base, consumed.id);
  await engine.expectError(
    base + "/artifacts/" + job.id + "/release",
    "POST",
    undefined,
    "ARTIFACT_IN_USE",
  );
  samples.after = await engine.api("/v1/resources");
  assert.equal(samples.after.previews.source_bytes, "0");
  checks.push(
    "derived integer query selects exactly ten of 20,000 candidates; another tool consumes the fixed artifact and protects its version",
  );
  await json(resolve(runDir, "report.json"), {
    passed: true,
    checks,
    timings,
    samples,
    rows: count,
    material: artifact.files,
    format:
      "schemaful JSONL plus SQLite typed projection; no physical columnar claim",
    boundaries:
      "Synthetic metadata catalog, no original media reads; API consumers only read bounded pages.",
    projectId: p.id,
    runDir,
  });
  console.log(
    JSON.stringify(
      { passed: true, checks, timings, report: resolve(runDir, "report.json") },
      null,
      2,
    ),
  );
} catch (error) {
  await json(resolve(runDir, "failure.json"), {
    error: String(error),
    checks,
    timings,
  });
  throw error;
} finally {
  await engine.stop();
}
