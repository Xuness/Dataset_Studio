// Explicit, bounded real-image probe against a loopback mock. Never calls a commercial API.
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { createHash, randomUUID } from "node:crypto";
import { readFile, mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { EngineFixture } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";
import { network } from "./llm-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const [auditFile, appDirectory, promptName] = process.argv.slice(2);
assert.ok(
  auditFile && appDirectory && promptName,
  "Provide metadata audit, application data directory and prompt name",
);
const audit = JSON.parse(await readFile(resolve(auditFile), "utf8"));
assert.ok(audit.rows.length >= 2 && audit.rows.length <= 512);
assert.equal(new Set(audit.rows.map((r) => r.key.source_id)).size, 1);
assert.equal(audit.summary.errors, 0);
const registry = new DatabaseSync(resolve(appDirectory, "registry.sqlite"), {
  readOnly: true,
});
const source = JSON.parse(
  registry
    .prepare("SELECT json FROM source_locations WHERE id=?")
    .get(audit.rows[0].key.source_id).json,
);
const prompt = JSON.parse(
  registry
    .prepare(
      "SELECT json FROM llm_system_prompts WHERE json_extract(json,'$.config.name')=?",
    )
    .get(promptName).json,
);
registry.close();
const run = resolve(
  root,
  ".local/test-runs/aesthetic-workset-probe-" + Date.now(),
);
await mkdir(run, { recursive: true });
const engine = new EngineFixture(root, resolve(run, "state"));
const { StudioClient } = await clientFixture(root, resolve(run, "sdk"));
let client, project;
const calls = [];
const server = createServer(async (req, res) => {
  try {
    const chunks = [];
    for await (const b of req) chunks.push(b);
    const raw = Buffer.concat(chunks);
    assert.ok(raw.length <= 32 * 1024 * 1024);
    const body = JSON.parse(raw);
    const blocks = body.messages
      .filter((m) => m.role === "user")
      .flatMap((m) => m.content);
    const ids = blocks
      .filter((b) => /^img\d{2}$/.test(b.text))
      .map((b) => b.text);
    const hashes = blocks
      .filter((b) => b.type === "image_url")
      .map((b) =>
        createHash("sha256")
          .update(Buffer.from(b.image_url.url.split(",")[1], "base64"))
          .digest("hex"),
      );
    const db = new DatabaseSync(
      resolve(project.directory, "evaluation.sqlite"),
      { readOnly: true },
    );
    const batch = db
      .prepare(
        "SELECT sequence,members FROM batches WHERE state='sent' ORDER BY sequence DESC LIMIT 1",
      )
      .get();
    db.close();
    const members = JSON.parse(batch.members);
    assert.deepEqual(
      ids,
      members.map((m) => m.label),
    );
    assert.deepEqual(
      hashes,
      members.map((m) => m.image_sha256),
    );
    assert.deepEqual(
      hashes,
      members.map((m) => m.candidate.key.asset_id),
    );
    assert.ok(calls.length < 64);
    calls.push({ batch: batch.sequence, ids, hashes, bytes: raw.length });
    const content = JSON.stringify({
      schema_version: 1,
      tiers: ids.map((id) => [id]),
      elite_candidates: [],
      unjudgeable: [],
    });
    res.writeHead(200, {
      "content-type": "application/json",
      "x-request-id": "local-identity-probe",
    });
    res.end(
      JSON.stringify({
        id: "mock-" + calls.length,
        choices: [{ index: 0, finish_reason: "stop", message: { content } }],
        usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
      }),
    );
  } catch (e) {
    res.writeHead(500);
    res.end(String(e));
  }
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
try {
  await engine.start();
  client = new StudioClient(engine.connection);
  project = await client.createProject({
    name: "真实工作集本地传输验证",
    parent_directory: null,
  });
  const attached = await client.attachSource(project.id, {
    name: source.name,
    kind: source.kind,
    index_root: source.index_root,
    media_root: source.media_root,
  });
  let selection = await client.selection(project.id);
  for (let i = 0; i < audit.rows.length; i += 128) {
    selection = await client.changeSelection(project.id, {
      expected_revision: selection.revision,
      clear: i === 0,
      add: audit.rows
        .slice(i, i + 128)
        .map((r) => ({ source_id: attached.id, asset_id: r.key.asset_id })),
      remove: [],
    });
  }
  const workset = await client.createCollection(
    project.id,
    audit.collection_name + " · 本地传输验证",
  );
  const saved = await client.llm.systemPrompts.save({
    expected_revision: 0,
    config: prompt.config,
  });
  const provider = await client.llm.providers.save({
    expected_revision: 0,
    config: {
      name: "loopback mock only",
      kind: "openrouter",
      base_url: `http://127.0.0.1:${server.address().port}`,
      enabled: true,
      headers: {},
      network: { ...network, rate_limit_retries: 0 },
    },
  });
  const model = await client.llm.models.save({
    provider_id: provider.id,
    expected_revision: 0,
    config: {
      name: "local image binding probe",
      remote_model_id: "fixture",
      protocol: "openai_chat",
      enabled: true,
      parameters: {},
      capability_overrides: { input_image: "supported" },
    },
  });
  const stage = await client.aesthetic.create(project.id, {
    idempotency_key: randomUUID(),
    name: "实际图片身份及字节预算",
    collection_id: workset.id,
    model_id: model.id,
    system_prompt_id: saved.id,
    overrides: {},
    exposures: 1,
    max_calls: 64,
    concurrency: 1,
  });
  const path = `/v1/projects/${project.id}/aesthetic/stages/${stage.id}`;
  await engine.wait(
    path,
    (s) => ["ready", "needs_attention", "failed"].includes(s.state),
    180000,
  );
  assert.equal(calls.length, 0);
  await client.aesthetic.control(project.id, stage.id, "start");
  const done = await engine.wait(
    path,
    (s) => !["running", "preparing", "pausing"].includes(s.state),
    180000,
  );
  assert.equal(done.state, "completed", done.error);
  assert.equal(done.comparable, audit.rows.length);
  assert.deepEqual(
    [...new Set(calls.flatMap((c) => c.hashes))].sort(),
    audit.rows.map((r) => r.key.asset_id).sort(),
  );
  const report = {
    scope:
      "real images; loopback HTTP mock; no commercial calls; no aesthetic-quality claim",
    source_workset: audit.collection_name,
    count: audit.rows.length,
    prompt_name: promptName,
    prompt_sha256: createHash("sha256")
      .update(prompt.config.text)
      .digest("hex"),
    stage: done,
    calls,
  };
  await writeFile(resolve(run, "report.json"), JSON.stringify(report, null, 2));
  console.log(
    JSON.stringify({
      count: audit.rows.length,
      local_calls: calls.length,
      group_sizes: calls.map((c) => c.ids.length),
      max_request_bytes: Math.max(...calls.map((c) => c.bytes)),
      state: done.state,
      run,
    }),
  );
} finally {
  client?.dispose();
  await engine.stop();
  server.closeAllConnections();
  await new Promise((done) => server.close(done));
}
