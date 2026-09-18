// Exercises the production SDK, engine, source reader, ledger and parser. All providers are local mocks.
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { randomUUID } from "node:crypto";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, writeFile, readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { EngineFixture, sleep } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";
import { network } from "./llm-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local/test-runs/integration-aesthetic-" + Date.now(),
);
await mkdir(run, { recursive: true });
await promisify(execFile)(
  "python",
  [resolve(root, "tooling/ranking-fixture.py"), resolve(run, "fixture"), "128"],
  { cwd: root, windowsHide: true },
);
const fixture = JSON.parse(
  await readFile(resolve(run, "fixture/fixture.json"), "utf8"),
);
const engine = new EngineFixture(root, resolve(run, "state"));
const { StudioClient } = await clientFixture(root, resolve(run, "sdk"));
const mock = { calls: [], mode: "valid", hold: false };
const server = createServer(async (req, res) => {
  res.on("error", () => {});
  try {
    const chunks = [];
    for await (const part of req) chunks.push(part);
    const body = JSON.parse(Buffer.concat(chunks));
    const path = new URL(req.url, "http://localhost").pathname;
    const gemini = path.includes("gemini");
    const responses = path.endsWith("responses");
    const blocks = gemini
      ? body.contents.flatMap((m) => m.parts)
      : (body.messages ?? body.input)
          .filter((m) => m.role === "user")
          .flatMap((m) => m.content);
    const ids = blocks.map((b) => b.text).filter((v) => /^img\d{2}$/.test(v));
    const images = blocks.filter((b) => b.inlineData || b.image_url);
    assert.equal(ids.length, images.length);
    assert.ok(ids.length > 0 && ids.length <= 16);
    assert.equal(new Set(ids).size, ids.length);
    const call = {
      protocol: gemini ? "gemini" : responses ? "responses" : "chat",
      ids,
      images: images.length,
      bytes: Buffer.concat(chunks).length,
      mode: mock.mode,
    };
    mock.calls.push(call);
    while (mock.hold && !res.destroyed) await sleep(25);
    if (res.destroyed) return;
    if (call.mode === "rate") {
      res.writeHead(429, { "content-type": "application/json" });
      res.end('{"error":{"message":"limited"}}');
      return;
    }
    const text =
      call.mode === "invalid"
        ? "not ranking JSON"
        : JSON.stringify({
            schema_version: 1,
            tiers: [
              ids.slice(0, Math.ceil(ids.length / 2)),
              ids.slice(Math.ceil(ids.length / 2)),
            ].filter((v) => v.length),
            elite_candidates: [ids[0]],
            unjudgeable: [],
          });
    const value = gemini
      ? {
          responseId: "gemini-result",
          modelVersion: "fixture",
          candidates: [
            { index: 0, content: { parts: [{ text }] }, finishReason: "STOP" },
          ],
          usageMetadata: {
            promptTokenCount: 13,
            candidatesTokenCount: 7,
            totalTokenCount: 20,
          },
        }
      : responses
        ? {
            id: "responses-result",
            model: "fixture",
            status: "completed",
            output: [
              { type: "message", content: [{ type: "output_text", text }] },
            ],
            usage: { input_tokens: 13, output_tokens: 7, total_tokens: 20 },
          }
        : {
            id: "chat-result",
            model: "fixture",
            choices: [
              { index: 0, message: { content: text }, finish_reason: "stop" },
            ],
            usage: {
              prompt_tokens: 13,
              completion_tokens: 7,
              total_tokens: 20,
            },
          };
    res.writeHead(200, {
      "content-type": "application/json",
      "x-request-id": "aesthetic-mock",
    });
    res.end(JSON.stringify(value));
  } catch (e) {
    res.writeHead(500);
    res.end(String(e));
  }
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
const endpoint = `http://127.0.0.1:${server.address().port}`;
let client, project;
const checks = [];
const stagePath = (id) => `/v1/projects/${project.id}/aesthetic/stages/${id}`;
async function waitStage(id, predicate) {
  return engine.wait(
    stagePath(id),
    (s) => {
      if (s.state === "failed") throw new Error(s.error);
      return predicate(s);
    },
    180000,
  );
}
async function waitCalls(count) {
  const end = Date.now() + 180000;
  while (mock.calls.length < count && Date.now() < end) await sleep(40);
  assert.ok(mock.calls.length >= count, "mock did not receive request");
}
async function workset(name, keys) {
  const selected = await client.selection(project.id);
  await client.changeSelection(project.id, {
    expected_revision: selected.revision,
    add: keys,
    remove: [],
    clear: true,
  });
  return client.createCollection(project.id, name);
}
try {
  await engine.start();
  client = new StudioClient(engine.connection);
  project = await client.createProject({
    name: "评审链路隔离测试",
    parent_directory: null,
  });
  const source = await engine.api(
    `/v1/projects/${project.id}/sources`,
    "POST",
    {
      kind: "danbooru",
      name: "合成图源",
      index_root: fixture.lake,
      media_root: fixture.lake,
    },
  );
  const assets = fixture.objects
    .filter((v) => v.number >= 32 && v.number < 96)
    .map((v) => ({
      number: v.number,
      key: { source_id: source.id, asset_id: v.sha },
    }));
  const all = await workset(
    "四种 Rating",
    assets.map((a) => a.key),
  );
  const single = await workset(
    "单 Rating 十六图",
    assets.filter((a) => a.number % 4 === 0).map((a) => a.key),
  );
  assert.equal(single.count, 16);
  const prompt = await client.llm.systemPrompts.save({
    expected_revision: 0,
    config: {
      name: "试验标准",
      description: "mock only",
      text: "评价构图、色彩和整体表达。仅提名达到顶级标准的图片。",
    },
  });
  const models = [];
  for (const [kind, protocol, path] of [
    ["openai_compatible", "openai_chat", "chat"],
    ["openai_compatible", "openai_responses", "responses"],
    ["gemini", "gemini", "gemini"],
  ]) {
    const provider = await client.llm.providers.save({
      expected_revision: 0,
      config: {
        name: path,
        kind,
        base_url: endpoint + "/" + path,
        enabled: true,
        headers: {},
        network: {
          ...network,
          request_timeout_ms: 120000,
          idle_timeout_ms: 120000,
          rate_limit_retries: 2,
        },
      },
      api_key: "fixture-secret",
    });
    models.push(
      await client.llm.models.save({
        provider_id: provider.id,
        expected_revision: 0,
        config: {
          name: path,
          remote_model_id: "fixture",
          protocol,
          enabled: true,
          parameters: {},
          capability_overrides: { input_image: "supported" },
        },
      }),
    );
  }
  const configuration = (collection = single, extra = {}) => ({
    idempotency_key: randomUUID(),
    name: "阶段",
    collection_id: collection.id,
    model_id: models[0].id,
    system_prompt_id: prompt.id,
    exposures: 1,
    max_calls: 20,
    concurrency: 1,
    overrides: {},
    ...extra,
  });
  async function create(request = configuration()) {
    const result = await client.aesthetic.create(project.id, request);
    const ready = await waitStage(
      result.id,
      (s) => s.state === "ready" || s.state === "needs_attention",
    );
    assert.equal(ready.state, "ready", ready.error);
    return ready;
  }
  const request = configuration(all, { concurrency: 2 });
  const first = await create(request);
  assert.equal(mock.calls.length, 0);
  assert.equal(
    (await client.aesthetic.create(project.id, request)).id,
    first.id,
  );
  await assert.rejects(
    () => client.aesthetic.create(project.id, { ...request, name: "变更" }),
    (e) => e.code === "IDEMPOTENCY_CONFLICT",
  );
  await client.aesthetic.control(project.id, first.id, "start");
  const complete = await waitStage(
    first.id,
    (s) => !["running", "pausing"].includes(s.state),
  );
  assert.equal(complete.state, "completed", complete.error);
  assert.equal(complete.accepted, 4);
  assert.equal(complete.protected, 4);
  assert.equal(complete.attempts, 4);
  const batches = (await client.aesthetic.batches(project.id, first.id)).items;
  assert.equal(new Set(batches.map((b) => b.rating)).size, 4);
  assert.ok(
    batches.every(
      (b) =>
        b.members.length === 16 &&
        b.members.every((m) => m.candidate.rating === b.rating),
    ),
  );
  assert.ok(
    (await client.aesthetic.candidates(project.id, first.id)).items.every(
      (c) => c.exposures === 1,
    ),
  );
  assert.equal(
    (await client.aesthetic.candidates(project.id, first.id, true)).items
      .length,
    4,
  );
  assert.ok(
    !JSON.stringify(
      await client.aesthetic.attempts(
        project.id,
        first.id,
        batches[0].sequence,
      ),
    ).includes("data:image"),
  );
  await client.aesthetic.control(project.id, first.id, "parse");
  assert.equal(mock.calls.length, 4);
  checks.push(
    "frozen workset and configuration, no paid call on create, idempotent creation, 16-image Rating isolation, ties and elite protection, local replay without another call",
  );
  for (const model of models.slice(1)) {
    const stage = await create(configuration(single, { model_id: model.id }));
    await client.aesthetic.control(project.id, stage.id, "start");
    const done = await waitStage(stage.id, (s) => s.state !== "running");
    assert.equal(done.state, "completed", done.error);
  }
  checks.push(
    "production Chat, Responses and Gemini adapters preserve 16-image mappings and feed the business validator",
  );
  const uncertainMetadata = await workset(
    "缺失和冲突的分组",
    fixture.objects
      .filter((v) => [1, 2, 14].includes(v.number))
      .map((v) => ({ source_id: source.id, asset_id: v.sha })),
  );
  const excluded = await create(configuration(uncertainMetadata));
  assert.equal(excluded.total, 3);
  assert.equal(excluded.eligible, 0);
  assert.deepEqual(
    (await client.aesthetic.candidates(project.id, excluded.id)).items
      .map((v) => v.rating)
      .sort(),
    ["conflict", "conflict", "unknown"],
  );
  const beforeExcluded = mock.calls.length;
  await client.aesthetic.control(project.id, excluded.id, "start");
  await waitStage(excluded.id, (s) => s.state === "needs_attention");
  assert.equal(mock.calls.length, beforeExcluded);
  await client.aesthetic.control(project.id, excluded.id, "cancel");
  checks.push(
    "missing and conflicting origin ratings remain visible and never purchase a comparison",
  );
  mock.mode = "invalid";
  const invalid = await create();
  await client.aesthetic.control(project.id, invalid.id, "start");
  const rejected = await waitStage(
    invalid.id,
    (s) => s.state === "needs_attention",
  );
  assert.equal(rejected.accepted, 0);
  assert.equal(rejected.invalid, 1);
  assert.ok(
    (await client.aesthetic.candidates(project.id, invalid.id)).items.every(
      (c) => c.exposures === 0,
    ),
  );
  const badBatch = (await client.aesthetic.batches(project.id, invalid.id))
    .items[0];
  assert.ok(
    (await client.aesthetic.attempts(project.id, invalid.id, badBatch.sequence))
      .items[0].receipt,
  );
  await assert.rejects(
    () =>
      client.aesthetic.retry(project.id, invalid.id, badBatch.sequence, false),
    (e) => e.code === "INVALID_INPUT",
  );
  mock.mode = "valid";
  await client.aesthetic.retry(project.id, invalid.id, badBatch.sequence, true);
  await client.aesthetic.control(project.id, invalid.id, "start");
  const fixed = await waitStage(invalid.id, (s) => s.state === "completed");
  assert.equal(fixed.accepted, 1);
  assert.equal(fixed.attempts, 2);
  assert.equal(fixed.invalid, 0);
  checks.push(
    "invalid JSON is retained without exposure, explicit charged retry retains attempt history and accepts one observation",
  );
  mock.mode = "rate";
  const limited = await create();
  const beforeLimit = mock.calls.length;
  await client.aesthetic.control(project.id, limited.id, "start");
  await waitStage(limited.id, (s) => s.state === "needs_attention");
  assert.equal(mock.calls.length, beforeLimit + 1);
  mock.mode = "valid";
  checks.push(
    "429 has one ledger attempt and no hidden provider retry despite provider retry configuration",
  );
  const budget = await create(
    configuration(all, { max_calls: 1, concurrency: 4 }),
  );
  const beforeBudget = mock.calls.length;
  await client.aesthetic.control(project.id, budget.id, "start");
  await waitStage(budget.id, (s) => s.state === "needs_attention");
  assert.equal(mock.calls.length, beforeBudget + 1);
  await assert.rejects(
    () => client.aesthetic.control(project.id, budget.id, "start"),
    (e) => e.code === "INVALID_INPUT",
  );
  checks.push("atomic call budget is respected under concurrent preparation");
  mock.hold = true;
  const interrupted = await create();
  const beforeCrash = mock.calls.length;
  await client.aesthetic.control(project.id, interrupted.id, "start");
  await waitCalls(beforeCrash + 1);
  client.dispose();
  await engine.stop(true);
  mock.hold = false;
  await engine.start();
  client = new StudioClient(engine.connection);
  await client.openProject(project.directory);
  const recovered = await client.aesthetic.stage(project.id, interrupted.id);
  assert.equal(recovered.state, "paused");
  assert.equal(recovered.unknown, 1);
  await sleep(350);
  assert.equal(mock.calls.length, beforeCrash + 1);
  const uncertain = (await client.aesthetic.batches(project.id, interrupted.id))
    .items[0];
  await client.aesthetic.retry(
    project.id,
    interrupted.id,
    uncertain.sequence,
    true,
  );
  await client.aesthetic.control(project.id, interrupted.id, "start");
  const resumed = await waitStage(
    interrupted.id,
    (s) => s.state === "completed",
  );
  assert.equal(resumed.attempts, 2);
  assert.equal(resumed.accepted, 1);
  assert.equal(resumed.unknown, 0);
  checks.push(
    "kill after upstream acceptance recovers outcome_unknown without automatic replay; explicit retry succeeds once",
  );
  mock.hold = true;
  const pausing = await create(configuration(all));
  const beforePause = mock.calls.length;
  await client.aesthetic.control(project.id, pausing.id, "start");
  await waitCalls(beforePause + 1);
  const closed = await client.closeProject(project.id);
  assert.equal(closed.state, "background");
  await client.openProject(project.directory);
  assert.equal(
    (await client.aesthetic.stage(project.id, pausing.id)).state,
    "running",
  );
  await client.aesthetic.control(project.id, pausing.id, "pause");
  assert.equal(
    (await client.aesthetic.stage(project.id, pausing.id)).state,
    "pausing",
  );
  mock.hold = false;
  const paused = await waitStage(pausing.id, (s) => s.state === "paused");
  assert.equal(paused.attempts, 1);
  assert.equal(paused.accepted, 1);
  await client.aesthetic.control(project.id, pausing.id, "cancel");
  checks.push(
    "project close keeps its active evaluation lease; pause drains the sent result without new dispatch; cancel retains evidence",
  );
  const occupied = [];
  for (let i = 0; i < 8; i++) occupied.push(await create());
  mock.hold = true;
  for (const s of occupied)
    await client.aesthetic.control(project.id, s.id, "start");
  const overflow = configuration();
  await assert.rejects(
    () => client.aesthetic.create(project.id, overflow),
    (e) => e.code === "EVALUATION_BUSY",
  );
  assert.equal(
    (await client.aesthetic.stage(project.id, overflow.idempotency_key)).state,
    "needs_attention",
  );
  await client.aesthetic.control(
    project.id,
    overflow.idempotency_key,
    "cancel",
  );
  for (const s of occupied)
    await client.aesthetic.control(project.id, s.id, "cancel");
  for (const s of occupied)
    await waitStage(s.id, (v) => v.state === "cancelled");
  mock.hold = false;
  checks.push(
    "stage admission overflow leaves a recoverable state, not an orphaned preparation; cancellation releases in-flight reservations",
  );
  const saved = await client.aesthetic.backup(project.id);
  const backup = new DatabaseSync(
    resolve(project.directory, saved.relative_path),
    { readOnly: true },
  );
  assert.equal(backup.prepare("PRAGMA quick_check").get().quick_check, "ok");
  assert.ok(backup.prepare("SELECT count(*) AS n FROM evidence").get().n >= 9);
  backup.close();
  const metrics = await client.aesthetic.metrics(project.id);
  assert.equal(metrics.active_requests, 0);
  assert.equal(metrics.reserved_request_bytes, 0);
  assert.ok(metrics.peak_request_bytes <= 512 * 1048576);
  assert.ok(metrics.peak_write_bytes <= 64 * 1048576);
  checks.push(
    "consistent WAL backup, retained paid evidence, bounded request and writer reservations, released permits",
  );
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      {
        passed: true,
        checks,
        metrics,
        calls: mock.calls,
        project: project.directory,
      },
      null,
      2,
    ),
  );
  console.log(
    JSON.stringify({
      passed: true,
      checks: checks.length,
      report: resolve(run, "report.json"),
    }),
  );
} catch (error) {
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      { passed: false, checks, error: String(error), calls: mock.calls },
      null,
      2,
    ),
  );
  throw error;
} finally {
  mock.hold = false;
  client?.dispose();
  await engine.stop();
  server.closeAllConnections();
  await new Promise((done) => server.close(done));
}
