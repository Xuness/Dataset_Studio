// Production SDK/engine/source/receipt paths, deterministic loopback judgments only.
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { createHash, randomUUID } from "node:crypto";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { EngineFixture, sleep } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";
import { network } from "./llm-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local/test-runs/integration-aesthetic-sampling-" + Date.now(),
);
await mkdir(run, { recursive: true });
await promisify(execFile)(
  "python",
  [resolve(root, "tooling/ranking-fixture.py"), resolve(run, "fixture"), "640"],
  { cwd: root, windowsHide: true },
);
const fixture = JSON.parse(
  await readFile(resolve(run, "fixture/fixture.json"), "utf8"),
);
const selected = fixture.objects
  .filter((v) => v.number >= 32 && v.number % 4 === 0)
  .slice(0, 145);
const qualities = new Map(selected.map((v, i) => [v.sha, i]));
const engine = new EngineFixture(root, resolve(run, "state"));
const { StudioClient } = await clientFixture(root, resolve(run, "sdk"));
const calls = [],
  checks = [],
  errors = [];
let client,
  project,
  hold = false;
const server = createServer(async (req, res) => {
  try {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    const body = JSON.parse(Buffer.concat(chunks));
    const parts = body.messages
      .filter((m) => m.role === "user")
      .flatMap((m) => m.content);
    const images = [];
    for (let i = 0; i < parts.length; i++)
      if (/^img\d{2}$/.test(parts[i].text)) {
        assert.ok(parts[i + 1]?.image_url);
        const hash = createHash("sha256")
          .update(
            Buffer.from(parts[i + 1].image_url.url.split(",")[1], "base64"),
          )
          .digest("hex");
        assert.ok(qualities.has(hash));
        images.push({ id: parts[i].text, hash, quality: qualities.get(hash) });
      }
    assert.ok(images.length >= 2 && images.length <= 16);
    const db = new DatabaseSync(
      resolve(project.directory, "evaluation.sqlite"),
      { readOnly: true },
    );
    const sent = db
      .prepare(
        "SELECT sequence,stage_id,members,sampling FROM batches WHERE state='sent' ORDER BY sequence LIMIT 32",
      )
      .all();
    db.close();
    const batch = sent.find(
      (v) =>
        JSON.stringify(JSON.parse(v.members).map((m) => m.image_sha256)) ===
        JSON.stringify(images.map((v) => v.hash)),
    );
    assert.ok(batch, "wire image identities must match one frozen batch");
    const sampling = JSON.parse(batch.sampling) ?? { round: 0 };
    calls.push({
      batch: batch.sequence,
      stage: batch.stage_id,
      round: sampling.round,
      ordinals: JSON.parse(batch.members).map((m) => m.candidate.ordinal),
    });
    while (hold) await sleep(20);
    // Variable latency, independently of dispatch order.
    await sleep((17 - (images[0].quality % 17)) * 3);
    const text = JSON.stringify({
      schema_version: 1,
      tiers: images.sort((a, b) => b.quality - a.quality).map((v) => [v.id]),
      elite_candidates: [],
      unjudgeable: [],
    });
    res.writeHead(200, {
      "content-type": "application/json",
      "x-request-id": "sampling-fixture",
    });
    res.end(
      JSON.stringify({
        id: "sampling",
        model: "fixture",
        choices: [
          { index: 0, finish_reason: "stop", message: { content: text } },
        ],
        usage: { prompt_tokens: 20, completion_tokens: 10, total_tokens: 30 },
      }),
    );
  } catch (error) {
    errors.push(String(error));
    res.writeHead(500);
    res.end(String(error));
  }
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
const path = (id) => `/v1/projects/${project.id}/aesthetic/stages/${id}`;
const wait = (id, p) => engine.wait(path(id), p, 180000);
async function fit(stage) {
  const job = await client.aesthetic.analysis.create(project.id, {
    idempotency_key: randomUUID(),
    name: "sampling validation",
    spec: {
      kind: "fit",
      config: {
        stage_id: stage,
        estimator: {
          kind: "davidson_v1",
          iterations: 128,
          regularization: 0.1,
          tie_strength: 1,
        },
        stability_seed: 17,
      },
      experiment_id: null,
      variant: null,
    },
  });
  return engine.wait(
    `/v1/projects/${project.id}/aesthetic/analysis/jobs/${job.id}`,
    (v) => v.state === "completed",
    180000,
  );
}
try {
  await engine.start();
  client = new StudioClient(engine.connection);
  project = await client.createProject({
    name: "145图轮次采样验收",
    parent_directory: null,
  });
  const source = await engine.api(
    `/v1/projects/${project.id}/sources`,
    "POST",
    {
      kind: "danbooru",
      name: "fixture",
      index_root: fixture.lake,
      media_root: fixture.lake,
    },
  );
  const selection = await client.selection(project.id);
  await client.changeSelection(project.id, {
    expected_revision: selection.revision,
    add: selected.map((v) => ({ source_id: source.id, asset_id: v.sha })),
    remove: [],
    clear: true,
  });
  const collection = await client.createCollection(
    project.id,
    "145同Rating图片",
  );
  assert.equal(collection.count, 145);
  const prompt = await client.llm.systemPrompts.save({
    expected_revision: 0,
    config: { name: "fixture", description: "loopback only", text: "fixture" },
  });
  const provider = await client.llm.providers.save({
    expected_revision: 0,
    config: {
      name: "fixture",
      kind: "openai_compatible",
      base_url: `http://127.0.0.1:${server.address().port}`,
      enabled: true,
      headers: {},
      network,
    },
    api_key: "fixture",
  });
  const model = await client.llm.models.save({
    provider_id: provider.id,
    expected_revision: 0,
    config: {
      name: "fixture",
      remote_model_id: "fixture",
      protocol: "openai_chat",
      enabled: true,
      parameters: {},
      capability_overrides: { input_image: "supported" },
    },
  });
  const request = {
    idempotency_key: randomUUID(),
    name: "均衡四次",
    collection_id: collection.id,
    model_id: model.id,
    system_prompt_id: prompt.id,
    exposures: 4,
    max_calls: 48,
    concurrency: 16,
    max_request_mib: 32,
    overrides: {},
    sampling: {
      mode: "balanced",
      min_exposures: 4,
      max_exposures: 6,
      rank_tolerance: 0.1,
      seed: 17,
    },
  };
  const preflight = await client.aesthetic.preflight(project.id, request);
  assert.equal(preflight.minimum_calls_lower_bound, 37);
  const stage = await client.aesthetic.create(project.id, request);
  await wait(stage.id, (v) => v.state === "ready");
  assert.equal(calls.length, 0);
  hold = true;
  await client.aesthetic.control(project.id, stage.id, "start");
  const deadline = Date.now() + 30000;
  while (!calls.length && Date.now() < deadline) await sleep(20);
  assert.ok(calls.length);
  await client.aesthetic.control(project.id, stage.id, "pause");
  hold = false;
  await wait(stage.id, (v) => v.state === "paused");
  const paid = calls.length;
  client.dispose();
  await engine.stop();
  await engine.start();
  client = new StudioClient(engine.connection);
  await client.openProject(project.directory);
  await sleep(150);
  assert.equal(calls.length, paid);
  assert.equal(
    (await client.aesthetic.stage(project.id, stage.id)).sampling.round,
    1,
  );
  await client.aesthetic.control(project.id, stage.id, "start");
  const finished = await wait(stage.id, (v) =>
    ["completed", "needs_attention"].includes(v.state),
  );
  assert.equal(finished.state, "completed", finished.error);
  assert.equal(finished.sampling.components, 1);
  assert.equal(finished.sampling.covered, 145);
  for (const round of new Set(
    calls.filter((c) => c.stage === stage.id).map((c) => c.round),
  )) {
    const members = calls
      .filter((c) => c.stage === stage.id && c.round === round)
      .flatMap((c) => c.ordinals);
    assert.equal(new Set(members).size, members.length);
  }
  assert.ok(finished.attempts <= 48);
  checks.push(
    "145 images, concurrency 16, immutable disjoint rounds, pause/restart and connected coverage",
  );
  const snapshot = await fit(stage.id);
  assert.equal(snapshot.result.validity.groups[0].ranking_scope, "rating");
  assert.equal(snapshot.result.groups[0].compared, 145);
  const diagnostic = await client.aesthetic.samplingDiagnostic(
    project.id,
    stage.id,
    0,
  );
  assert.ok(diagnostic.distinct_opponents >= 24);
  checks.push(
    "versioned validity and per-candidate sampling evidence are available through the public SDK",
  );
  const supplemental = {
    idempotency_key: randomUUID(),
    additional_calls: 1,
    policy: {
      mode: "adaptive",
      min_exposures: 4,
      max_exposures: 8,
      rank_tolerance: 0.08,
      seed: 19,
    },
  };
  const beforeCalls = calls.length;
  const configured = await client.aesthetic.configureSampling(
    project.id,
    stage.id,
    supplemental,
  );
  await client.aesthetic.configureSampling(project.id, stage.id, supplemental);
  assert.equal(configured.sampling.call_limit, finished.attempts + 1);
  assert.equal(calls.length, beforeCalls);
  assert.equal(configured.config_hash, finished.config_hash);
  await assert.rejects(() =>
    client.aesthetic.configureSampling(project.id, stage.id, {
      ...supplemental,
      additional_calls: 2,
    }),
  );
  await client.aesthetic.control(project.id, stage.id, "start");
  const limited = await wait(stage.id, (v) => v.state === "needs_attention");
  assert.equal(limited.attempts, finished.attempts + 1);
  assert.equal(limited.sampling.reason, "call_budget");
  const callsAtLimit = calls.length;
  await client.aesthetic.control(project.id, stage.id, "start");
  const rechecked = await wait(stage.id, (v) => v.state === "needs_attention");
  assert.equal(rechecked.attempts, limited.attempts);
  assert.equal(calls.length, callsAtLimit);

  assert.equal(
    (await client.aesthetic.analysis.job(project.id, snapshot.id)).input
      .evidence_watermark,
    snapshot.input.evidence_watermark,
  );
  checks.push(
    "supplement configuration is idempotent and makes no API calls; exhausted adaptive budget remains unresolved",
  );
  const comparisons = [];
  async function measure(label, stage, job) {
    let after;
    const rows = [];
    do {
      const page = await client.aesthetic.analysis.rows(project.id, job.id, {
        after,
        limit: 64,
      });
      rows.push(...page.items);
      after = page.next_cursor;
    } while (after);
    const connected = job.result.groups[0].fully_connected;
    comparisons.push({
      label,
      call_cap: 48,
      calls: stage.attempts,
      components: job.result.groups[0].components,
      min_exposures: Math.min(...rows.map((r) => r.exposures)),
      max_exposures: Math.max(...rows.map((r) => r.exposures)),
      mean_percentile_error: connected
        ? rows.reduce(
            (sum, r) =>
              sum +
              Math.abs(
                r.percentile - (144 - qualities.get(r.key.asset_id)) / 144,
              ),
            0,
          ) / rows.length
        : null,
    });
  }
  await measure("balanced", finished, snapshot);
  for (const mode of ["legacy", "adaptive"]) {
    const req = {
      ...request,
      idempotency_key: randomUUID(),
      name: mode,
      exposures: mode === "adaptive" ? 2 : 4,
    };
    if (mode === "legacy") delete req.sampling;
    else
      req.sampling = {
        mode: "adaptive",
        min_exposures: 2,
        max_exposures: 8,
        rank_tolerance: 0.1,
        seed: 17,
      };
    const s = await client.aesthetic.create(project.id, req);
    await wait(s.id, (v) => v.state === "ready");
    await client.aesthetic.control(project.id, s.id, "start");
    const done = await wait(s.id, (v) =>
      ["completed", "needs_attention"].includes(v.state),
    );
    assert.ok(done.attempts <= 48);
    const ranking = await fit(s.id);
    await measure(mode, done, ranking);
  }
  checks.push(
    "legacy, balanced and adaptive sampling compared with the same 48-call cap and a deterministic synthetic quality reference",
  );
  const db = new DatabaseSync(resolve(project.directory, "evaluation.sqlite"), {
    readOnly: true,
  });
  const raw = db.prepare("SELECT COUNT(*) n FROM raw_receipts").get().n;
  assert.equal(raw, calls.length);
  assert.equal(db.prepare("PRAGMA quick_check").get().quick_check, "ok");
  db.close();
  assert.deepEqual(errors, []);
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      {
        passed: true,
        checks,
        providerCalls: calls.length,
        comparisons,
        calls,
        stage: finished,
        snapshot,
        limited,
        project: { id: project.id, directory: project.directory },
      },
      null,
      2,
    ),
  );
  console.log(
    JSON.stringify({ passed: checks.length, providerCalls: calls.length, run }),
  );
} finally {
  hold = false;
  client?.dispose();
  await engine.stop();
  await new Promise((done) => server.close(done));
}
