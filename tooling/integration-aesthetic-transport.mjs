// Exercises the production SDK, engine, source reader, ledger and parser. All providers are local mocks.
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { randomUUID, createHash } from "node:crypto";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, writeFile, readFile, open } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { EngineFixture, sleep } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";
import { network } from "./llm-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local/test-runs/integration-aesthetic-transport-" + Date.now(),
);
await mkdir(run, { recursive: true });
await promisify(execFile)(
  "python",
  [
    resolve(root, "tooling/ranking-fixture.py"),
    resolve(run, "fixture"),
    "128",
    "--aesthetic-large",
  ],
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
    if (project) {
      const db = new DatabaseSync(
        resolve(project.directory, "evaluation.sqlite"),
        { readOnly: true },
      );
      const row = db
        .prepare(
          "SELECT members FROM batches WHERE state='sent' ORDER BY sequence DESC LIMIT 1",
        )
        .get();
      db.close();
      assert.ok(row);
      const members = JSON.parse(row.members);
      assert.deepEqual(
        ids,
        members.map((m) => m.label),
      );
      const hashes = images.map((i) =>
        createHash("sha256")
          .update(
            Buffer.from(
              i.inlineData?.data ?? i.image_url.url.split(",")[1],
              "base64",
            ),
          )
          .digest("hex"),
      );
      assert.deepEqual(
        hashes,
        members.map((m) => m.image_sha256),
      );
      assert.deepEqual(
        hashes,
        members.map((m) => m.candidate.key.asset_id),
      );
      assert.ok(Buffer.concat(chunks).length <= 48 * 1024 * 1024);
    }
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
    if (call.mode === "raw-invalid") {
      res.writeHead(200, {
        "content-type": "application/json",
        "x-request-id": "raw-invalid",
        "set-cookie": "must-not-persist",
      });
      res.end("{broken-json");
      return;
    }
    if (call.mode === "oversize") {
      res.writeHead(200, { "content-type": "application/json" });
      res.end("x".repeat(17 * 1024 * 1024));
      return;
    }
    if (call.mode === "partial") {
      res.writeHead(200, {
        "content-type": "application/json",
        "x-request-id": "partial",
      });
      res.write('{"choices":[');
      setTimeout(() => res.destroy(), 100);
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
  await workset(
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
    max_request_mib: 12,
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

  const normal = await create();
  await client.aesthetic.control(project.id, normal.id, "start");
  const completed = await waitStage(normal.id, (s) =>
    ["completed", "needs_attention"].includes(s.state),
  );
  assert.equal(completed.state, "completed", completed.error);
  assert.equal(completed.attempts, 2);
  assert.equal(completed.accepted, 2);
  const batches = (await client.aesthetic.batches(project.id, normal.id)).items;
  assert.equal(batches.filter((b) => b.state === "superseded").length, 1);
  const accepted = batches.filter((b) => b.state === "accepted");
  assert.ok(accepted.every((b) => b.members.length === 8));
  checks.push(
    "native-body byte budget splits 16 large images into audited replacements; each wire ID and SHA-256 matches the ledger",
  );
  for (const batch of accepted) {
    const a = (
      await client.aesthetic.attempts(project.id, normal.id, batch.sequence)
    ).items[0];
    assert.ok(a.raw_receipt.complete);
    assert.equal(a.raw_receipt.http_status, 200);
  }
  const before = mock.calls.length;
  await client.aesthetic.reparse(project.id, normal.id, accepted[0].sequence);
  assert.equal(mock.calls.length, before);
  checks.push(
    "raw HTTP receipts saved; accepted evidence is immutable and local reparse never sends a request",
  );
  for (const mode of ["raw-invalid", "partial", "oversize"]) {
    mock.mode = mode;
    const s = await create(configuration(single, { max_calls: 1 }));
    await client.aesthetic.control(project.id, s.id, "start");
    await waitStage(s.id, (s) => s.state === "needs_attention");
    const bs = (await client.aesthetic.batches(project.id, s.id)).items;
    const attempted = bs.find((b) => b.attempt_id);
    assert.ok(attempted);
    const a = (
      await client.aesthetic.attempts(project.id, s.id, attempted.sequence)
    ).items[0];
    assert.ok(a.raw_receipt);
    assert.equal(a.raw_receipt.complete, mode === "raw-invalid");
    assert.equal(a.raw_receipt.http_status, 200);
    if (mode === "oversize") assert.equal(a.raw_receipt.bytes, 16 * 1048576);
    const count = mock.calls.length;
    await assert.rejects(
      () => client.aesthetic.reparse(project.id, s.id, attempted.sequence),
      (e) => e.code === "EVALUATION_REPARSE_FAILED",
    );
    assert.equal(mock.calls.length, count);
  }
  mock.mode = "valid";
  const wideRequest = configuration();
  delete wideRequest.max_request_mib;
  const wide = await create(wideRequest);
  assert.equal(wide.config.max_request_bytes, 32 * 1048576);
  await client.aesthetic.control(project.id, wide.id, "start");
  const wideDone = await waitStage(wide.id, (s) =>
    ["completed", "needs_attention"].includes(s.state),
  );
  assert.equal(wideDone.state, "completed", wideDone.error);
  assert.equal(wideDone.attempts, 1);
  assert.equal(mock.calls.at(-1).ids.length, 16);
  const wide48 = await create(configuration(single, { max_request_mib: 48 }));
  assert.equal(wide48.config.max_request_bytes, 48 * 1048576);
  await assert.rejects(
    () =>
      client.aesthetic.preflight(
        project.id,
        configuration(single, { max_request_mib: 49 }),
      ),
    (e) => e.code === "INVALID_INPUT",
  );
  assert.equal(
    (await client.aesthetic.stage(project.id, normal.id)).config
      .max_request_bytes,
    12 * 1048576,
  );
  checks.push(
    "new default 32 MiB keeps all 16 large images together; 48 MiB can be frozen; old 12 MiB stage remains unchanged",
  );
  const db = new DatabaseSync(resolve(project.directory, "evaluation.sqlite"), {
    readOnly: true,
  });
  for (const r of db
    .prepare("SELECT metadata,body,sha256 FROM raw_receipts")
    .all()) {
    assert.equal(createHash("sha256").update(r.body).digest("hex"), r.sha256);
    assert.ok(!r.metadata.includes("must-not-persist"));
    assert.ok(!r.metadata.includes("fixture-secret"));
  }
  db.close();
  checks.push(
    "malformed and partial HTTP responses retained with completeness and safe headers; reparse failure incurs no calls",
  );
  const originalCalls = mock.calls.length;
  const targetImage = fixture.objects.find((v) => v.number === 32);
  const archive = await open(resolve(fixture.lake, "packs/fixture.tar"), "r+");
  const firstByte = Buffer.alloc(1);
  await archive.read(firstByte, 0, 1, targetImage.offset);
  try {
    await archive.write(
      Buffer.from([firstByte[0] ^ 1]),
      0,
      1,
      targetImage.offset,
    );
    const damaged = await create();
    const candidates = (
      await client.aesthetic.candidates(project.id, damaged.id)
    ).items;
    assert.equal(
      candidates.filter((c) => c.disposition === "needs_review").length,
      1,
    );
    assert.equal(
      candidates.find((c) => c.disposition === "needs_review").key.asset_id,
      targetImage.sha,
    );
    assert.equal(mock.calls.length, originalCalls);
  } finally {
    await archive.write(firstByte, 0, 1, targetImage.offset);
    await archive.close();
  }
  checks.push(
    "creation verifies physical image bytes; one corrupted candidate is isolated before any model call",
  );
  const job = await client.submitJob(project.id, {
    idempotency_key: randomUUID(),
    scope: {
      project_id: project.id,
      target: { kind: "workset", collection_id: single.id },
    },
  });
  await engine.wait(
    `/v1/projects/${project.id}/jobs`,
    (v) => v.items.some((j) => j.id === job.id && j.status === "succeeded"),
    30000,
  );
  const pack = await client.aesthetic.recoveryPackage(project.id);
  const packageDirectory = resolve(project.directory, pack.relative_path);
  const manifest = JSON.parse(
    await readFile(resolve(packageDirectory, "recovery.json"), "utf8"),
  );
  assert.equal(manifest.evaluation_schema, 7);
  assert.ok(manifest.files.some((f) => f.path === "project.sqlite"));
  assert.ok(manifest.files.some((f) => f.path === "evaluation.sqlite"));
  const target = resolve(run, "restored-project");
  await assert.rejects(
    () => client.aesthetic.restore(packageDirectory, target),
    (e) => e.code === "PROJECT_BUSY",
  );
  await client.closeProject(project.id);
  await engine.stop();
  await engine.start();
  client = new StudioClient(engine.connection);
  const artifact = manifest.files.find((f) => f.path.startsWith("artifacts/"));
  assert.ok(artifact);
  const artifactPath = resolve(packageDirectory, artifact.path);
  const artifactBytes = await readFile(artifactPath);
  try {
    await writeFile(artifactPath, "tampered");
    await assert.rejects(
      () => client.aesthetic.restore(packageDirectory, target + "-rejected"),
      (e) => e.code === "BACKUP_INVALID",
    );
  } finally {
    await writeFile(artifactPath, artifactBytes);
  }
  await client.aesthetic.restore(packageDirectory, target);
  const restored = await client.openProject(target);
  assert.equal(restored.id, project.id);
  const savedStage = await client.aesthetic.stage(restored.id, normal.id);
  assert.equal(savedStage.accepted, 2);
  assert.equal(mock.calls.length, 6);
  checks.push(
    "project package verifies two database generations, file hashes and references; restoration preserves paid evidence without dispatch",
  );
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify({ checks, calls: mock.calls }, null, 2),
  );
  console.log("Aesthetic transport integration:", checks.length, "passed", run);
} finally {
  client?.dispose();
  await engine.stop();
  server.closeAllConnections();
  await new Promise((done) => server.close(done));
}
