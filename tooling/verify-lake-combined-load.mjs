// Explicit isolated full-index mirror only; recorded lake responses and localhost LLM.
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { execFile, spawn } from "node:child_process";
import { promisify } from "node:util";
import { open, readFile, writeFile } from "node:fs/promises";
import { resolve, basename, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import os from "node:os";
import { performance } from "node:perf_hooks";
import { DatabaseSync } from "node:sqlite";
import { EngineFixture, sleep, within } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";
import { lakeWorkerPython } from "./lake-worker-runtime.mjs";
import { network } from "./llm-fixture.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(process.argv[2]);
assert.equal(dirname(run), resolve(root, ".local/test-runs"));
assert.ok(basename(run).startsWith("astra-load-"));
const targets = JSON.parse(
  await readFile(resolve(run, "targets.json"), "utf8"),
);
assert.equal(targets.length, 3);
for (const t of targets) {
  within(run, t.media_root);
  within(run, t.index_root);
}
const python = lakeWorkerPython(root);
const engine = new EngineFixture(root, resolve(run, "engine-" + Date.now()));
const summary = {
  hardware: {
    platform: os.platform(),
    release: os.release(),
    cpu: os.cpus()[0].model,
    logical_cpus: os.cpus().length,
    ram_bytes: os.totalmem(),
  },
  cache:
    "New engine cache; OS file cache not flushed and warmed by snapshot copy",
  targets,
  engine_directory: engine.dataDir,
  phases: {},
  samples: [],
  errors: [],
  mock_calls: 0,
};
const server = createServer(async (req, res) => {
  try {
    const chunks = [];
    for await (const part of req) chunks.push(part);
    const body = JSON.parse(Buffer.concat(chunks));
    const ids = body.messages
      .filter((m) => m.role === "user")
      .flatMap((m) => m.content)
      .map((b) => b.text)
      .filter((s) => /^img\d{2}$/.test(s));
    assert.ok(ids.length > 0 && ids.length <= 16);
    summary.mock_calls++;
    await sleep(300);
    const text = JSON.stringify({
      schema_version: 1,
      tiers: [ids],
      elite_candidates: [],
      unjudgeable: [],
    });
    res.writeHead(200, {
      "content-type": "application/json",
      "x-request-id": "load-fixture",
    });
    res.end(
      JSON.stringify({
        id: "load-fixture",
        model: "fixture",
        choices: [
          { index: 0, message: { content: text }, finish_reason: "stop" },
        ],
        usage: { prompt_tokens: 10, completion_tokens: 10, total_tokens: 20 },
      }),
    );
  } catch (error) {
    res.writeHead(500);
    res.end(String(error));
  }
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
const percentile = (values, p) => {
  const sorted = [...values].sort((a, b) => a - b);
  return (
    sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * p))] ?? null
  );
};
const measure = (values) => ({
  n: values.length,
  p50: percentile(values, 0.5),
  p95: percentile(values, 0.95),
  p99: percentile(values, 0.99),
  max: Math.max(0, ...values),
});
let client,
  project,
  collection,
  fixedCollection,
  model,
  prompt,
  monitoring = false,
  monitor,
  updater;
const spec = (order = "post_id_asc", conditions = []) => ({
  version: 3,
  source_ids: targets.map((t) => t.library_id),
  conditions,
  observation_rule: "current_post",
  order,
});
async function browse(seconds, name) {
  const latencies = [],
    errors = [];
  const end = Date.now() + seconds * 1000;
  await Promise.all(
    ["post_id_asc", "post_id_desc", "asset_key_asc", "asset_key_desc"].map(
      async (order) => {
        const view = await client.queries.browse(project.id, spec(order));
        let cursor;
        const seen = new Set();
        while (Date.now() < end) {
          const start = performance.now();
          try {
            const response = await client.queries.assets(project.id, view.id, {
              limit: 64,
              ...(cursor ? { cursor } : {}),
            });
            latencies.push(performance.now() - start);
            for (const item of response.page.items) {
              const key = item.key.source_id + ":" + item.key.asset_id;
              assert.ok(
                !seen.has(key),
                "Duplicate during concurrent publication",
              );
              seen.add(key);
            }
            const next = response.page.next_cursor;
            assert.ok(
              !next || next !== cursor || response.page.items.length > 0,
              "Page made no progress",
            );
            cursor = next;
            if (!cursor) break;
          } catch (error) {
            errors.push({
              code: error.code ?? "ASSERTION",
              message: String(error),
              ms: performance.now() - start,
            });
            if (error.code !== "SOURCE_BUSY") throw error;
            await sleep(50);
          }
        }
      },
    ),
  );
  summary.phases[name] = { latency_ms: measure(latencies), errors };
  console.log(JSON.stringify({ phase: name, ...summary.phases[name] }));
  assert.equal(errors.length, 0, JSON.stringify(errors));
}
async function fixed(name, bound) {
  const at = performance.now();
  const builds = [];
  for (let iteration = 0; iteration < 10; iteration++) {
    const buildAt = performance.now();
    const result = await client.queries.run(project.id, {
      ...spec("post_id_asc", [
        {
          field: "post.id",
          operator: "lte",
          value: { type: "integer", value: String(bound + iteration) },
        },
      ]),
      input_scope: {
        project_id: project.id,
        target: { kind: "workset", collection_id: fixedCollection.id },
      },
    });
    const done = await engine.wait(
      `/v1/projects/${project.id}/query-results/${result.id}`,
      (r) => !["queued", "running"].includes(r.state),
      600000,
    );
    assert.equal(done.state, "ready", JSON.stringify(done));
    builds.push({ ms: performance.now() - buildAt, count: done.count });
  }
  summary.phases[name] = {
    ms: performance.now() - at,
    input_count: fixedCollection.count,
    builds,
  };
  console.log(JSON.stringify({ phase: name, ...summary.phases[name] }));
}
async function evaluate(name) {
  const at = performance.now();
  const stage = await client.aesthetic.create(project.id, {
    idempotency_key: crypto.randomUUID(),
    name,
    collection_id: collection.id,
    model_id: model.id,
    system_prompt_id: prompt.id,
    exposures: 2,
    max_calls: 100,
    concurrency: 2,
    overrides: {},
  });
  const path = `/v1/projects/${project.id}/aesthetic/stages/${stage.id}`;
  const ready = await engine.wait(
    path,
    (s) => ["ready", "needs_attention", "failed"].includes(s.state),
    180000,
  );
  assert.equal(ready.state, "ready", JSON.stringify(ready));
  await client.aesthetic.control(project.id, stage.id, "start");
  const done = await engine.wait(
    path,
    (s) => !["running", "pausing"].includes(s.state),
    600000,
  );
  assert.equal(done.state, "completed", JSON.stringify(done));
  summary.phases[name] = {
    ms: performance.now() - at,
    attempts: done.attempts,
    accepted: done.accepted,
  };
  console.log(JSON.stringify({ phase: name, ...summary.phases[name] }));
}
async function update(number) {
  const log = await open(resolve(run, `updates-${number}.log`), "w");
  updater = spawn(
    python,
    [
      resolve(root, "tooling/lake-load-fixture.py"),
      run,
      "updates",
      String(number),
    ],
    { stdio: ["ignore", log.fd, log.fd], windowsHide: true },
  );
  await log.close();
  const child = updater;
  const code = await new Promise((done) => child.once("exit", done));
  updater = undefined;
  assert.equal(code, 0, "Recorded update load failed; inspect fixture logs");
  console.log(JSON.stringify({ phase: "updates-" + number, exit_code: code }));
}
try {
  await engine.start();
  const { StudioClient } = await clientFixture(root, resolve(run, "sdk"));
  client = new StudioClient(engine.connection);
  project = await client.createProject({
    name: "Astra full-index combined load",
    parent_directory: resolve(run, "projects"),
  });
  for (const t of targets)
    await client.attachSource(project.id, {
      kind: "auto",
      name: t.site,
      media_root: t.media_root,
      index_root: t.index_root,
    });
  const selected = await client.selection(project.id);
  await client.changeSelection(project.id, {
    expected_revision: selected.revision,
    add: targets.flatMap((t) =>
      t.sample.map((sha) => ({ source_id: t.library_id, asset_id: sha })),
    ),
    remove: [],
    clear: true,
  });
  collection = await client.createCollection(
    project.id,
    "Copied real media sample",
  );
  if (!process.argv.includes("--browse-only")) {
    const fixedKeys = [];
    for (const target of targets) {
      const pointer = JSON.parse(
        await readFile(resolve(target.index_root, "ONLINE.json"), "utf8"),
      );
      const db = new DatabaseSync(
        within(run, resolve(target.index_root, pointer.file)),
        { readOnly: true },
      );
      fixedKeys.push(
        ...db
          .prepare("SELECT sha256 FROM objects ORDER BY object_row LIMIT 1365")
          .all()
          .map((r) => ({ source_id: target.library_id, asset_id: r.sha256 })),
      );
      db.close();
    }
    for (let start = 0; start < fixedKeys.length; start += 256) {
      const selection = await client.selection(project.id);
      await client.changeSelection(project.id, {
        expected_revision: selection.revision,
        add: fixedKeys.slice(start, start + 256),
        remove: [],
        clear: start === 0,
      });
    }
    fixedCollection = await client.createCollection(
      project.id,
      "4095 member load scope",
    );
    assert.equal(fixedCollection.count, 4095);
  }
  prompt = await client.llm.systemPrompts.save({
    expected_revision: 0,
    config: {
      name: "Offline load fixture",
      description: "",
      text: "Return the specified ranking JSON.",
    },
  });
  const provider = await client.llm.providers.save({
    expected_revision: 0,
    api_key: "fixture",
    config: {
      name: "Local load mock",
      kind: "openai_compatible",
      base_url: `http://127.0.0.1:${server.address().port}`,
      enabled: true,
      headers: {},
      network: {
        ...network,
        request_timeout_ms: 120000,
        idle_timeout_ms: 120000,
      },
    },
  });
  model = await client.llm.models.save({
    provider_id: provider.id,
    expected_revision: 0,
    config: {
      name: "Fixture",
      remote_model_id: "fixture",
      protocol: "openai_chat",
      enabled: true,
      parameters: {},
      capability_overrides: { input_image: "supported" },
    },
  });
  monitoring = true;
  monitor = (async () => {
    while (monitoring) {
      summary.samples.push({
        at: Date.now(),
        value: await engine.api("/v1/resources"),
      });
      await sleep(500);
    }
  })();
  await browse(15, "browse_baseline");
  if (!process.argv.includes("--browse-only")) {
    await fixed("fixed_baseline", 200000);
    await evaluate("review_baseline");
    await update(1);
    summary.combined_started = Date.now();
    await Promise.all([
      browse(45, "browse_combined"),
      fixed("fixed_combined", 210000),
      evaluate("review_combined"),
      update(2),
    ]);
    summary.combined_finished = Date.now();
    await promisify(execFile)(
      python,
      [resolve(root, "tooling/lake-load-fixture.py"), run, "spool"],
      { windowsHide: true },
    );
  }
  summary.final_resources = await engine.api("/v1/resources");
  assert.equal(summary.final_resources.online_sqlite.protocol_errors, 0);
  summary.passed = true;
} catch (error) {
  summary.errors.push({ message: String(error), stack: error.stack });
  throw error;
} finally {
  monitoring = false;
  await monitor?.catch((e) => summary.errors.push({ message: String(e) }));
  if (updater) {
    // Fixture only. Cooperative runner deadline retains all temporary evidence.
    await new Promise((done) => updater.once("exit", done));
  }
  await engine.stop();
  await new Promise((done) => server.close(done));
  await writeFile(
    resolve(run, "load-report.json"),
    JSON.stringify(summary, null, 2),
  );
}
console.log(
  JSON.stringify(
    { run, passed: summary.passed, phases: summary.phases },
    null,
    2,
  ),
);
