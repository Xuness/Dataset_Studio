// Two loopback model calls; never opens a real project or contacts a model provider.
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { EngineFixture } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";
import { network } from "./llm-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local/test-runs/integration-aesthetic-cache-" + Date.now(),
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
const calls = [],
  errors = [];
const server = createServer(async (request, response) => {
  try {
    const parts = [];
    for await (const part of request) parts.push(part);
    const body = JSON.parse(Buffer.concat(parts));
    const labels = body.messages
      .filter((m) => m.role === "user")
      .flatMap((m) => m.content)
      .map((p) => p.text)
      .filter((t) => /^img\d{2}$/.test(t));
    assert.equal(labels.length, 16);
    assert.equal(body.service_tier, "flex");
    assert.deepEqual(body.provider.only, ["google-ai-studio/flex"]);
    assert.equal(body.messages[0].content[0].cache_control.type, "ephemeral");
    calls.push({ session_id: body.session_id, labels: labels.length });
    response.writeHead(200, { "content-type": "application/json" });
    response.end(
      JSON.stringify({
        id: `mock-${calls.length}`,
        model: body.model,
        provider: "Google AI Studio",
        service_tier: "flex",
        choices: [
          {
            index: 0,
            finish_reason: "stop",
            message: {
              content: JSON.stringify({
                schema_version: 1,
                tiers: [labels],
                elite_candidates: [],
                unjudgeable: [],
              }),
            },
          },
        ],
        usage: {
          prompt_tokens: 100,
          completion_tokens: 10,
          prompt_tokens_details: {
            cached_tokens: calls.length === 1 ? 0 : 80,
            cache_write_tokens: calls.length === 1 ? 80 : 0,
          },
          cost: 0.001,
        },
      }),
    );
  } catch (error) {
    errors.push(String(error));
    response.writeHead(500);
    response.end("fixture failure");
  }
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
let client;
const checks = [];
try {
  await engine.start();
  client = new StudioClient(engine.connection);
  const project = await client.createProject({
    name: "缓存链路小样本",
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
  const selection = await client.selection(project.id);
  await client.changeSelection(project.id, {
    expected_revision: selection.revision,
    clear: true,
    remove: [],
    add: fixture.objects
      .filter((v) => v.number >= 32 && v.number < 96 && v.number % 4 === 0)
      .map((v) => ({ source_id: source.id, asset_id: v.sha })),
  });
  const collection = await client.createCollection(project.id, "十六图");
  assert.equal(collection.count, 16);
  const provider = await client.llm.providers.save({
    expected_revision: 0,
    config: {
      name: "Mock OpenRouter",
      kind: "openrouter",
      base_url: `http://127.0.0.1:${server.address().port}/v1`,
      enabled: true,
      headers: {},
      network,
    },
    api_key: "fixture-only",
  });
  let model = await client.llm.models.save({
    provider_id: provider.id,
    expected_revision: 0,
    config: {
      name: "Mock Gemini",
      remote_model_id: "google/gemini-cache-fixture",
      protocol: "openai_chat",
      enabled: true,
      parameters: {},
      capability_overrides: { input_image: "supported" },
    },
  });
  const prompt = await client.llm.systemPrompts.save({
    expected_revision: 0,
    config: {
      name: "固定评审标准",
      text: "Evaluate composition and color.",
      description: "mock",
    },
  });
  const policy = {
    stream: false,
    concurrency: 1,
    connect_timeout_ms: 3000,
    first_response_timeout_ms: 10000,
    idle_timeout_ms: 3000,
    request_timeout_ms: 10000,
    batch_timeout_ms: 15000,
    max_retries: 0,
    retry_unknown: false,
    exhausted: "pause",
  };
  async function create() {
    const stage = await client.aesthetic.create(project.id, {
      idempotency_key: crypto.randomUUID(),
      name: "缓存测试",
      collection_id: collection.id,
      model_id: model.id,
      system_prompt_id: prompt.id,
      overrides: {},
      exposures: 2,
      max_calls: 2,
      concurrency: 1,
      execution_policy: policy,
      budget_mode: "complete",
    });
    return engine.wait(
      `/v1/projects/${project.id}/aesthetic/stages/${stage.id}`,
      (s) => {
        assert.notEqual(s.state, "failed", s.error);
        return s.state === "ready";
      },
      30000,
    );
  }
  const legacy = await create();
  model = await client.llm.models.save({
    id: model.id,
    provider_id: provider.id,
    expected_revision: model.revision,
    config: {
      ...model.config,
      parameters: {
        service_tier: "flex",
        "openrouter.provider": { only: ["google-ai-studio/flex"] },
        "openrouter.cache_strategy": "system",
        "openrouter.cache_affinity": true,
      },
    },
  });
  const rebound = await client.aesthetic.configureExecution(
    project.id,
    legacy.id,
    {
      idempotency_key: crypto.randomUUID(),
      expected_revision: legacy.execution_settings.revision,
      policy,
    },
  );
  assert.deepEqual(rebound.config.model.parameters, {});
  assert.equal(rebound.execution_settings.model_revision, model.revision);
  checks.push(
    "new model cache defaults do not change a frozen stage during rebind",
  );
  const stage = await create();
  const other = await create();
  assert.equal(
    stage.config.model.parameters["openrouter.session_id"],
    `aesthetic-${stage.id}`,
  );
  assert.notEqual(
    stage.config.model.parameters["openrouter.session_id"],
    other.config.model.parameters["openrouter.session_id"],
  );
  await client.aesthetic.control(project.id, stage.id, "start");
  const completed = await engine.wait(
    `/v1/projects/${project.id}/aesthetic/stages/${stage.id}`,
    (s) => {
      if (["failed", "needs_attention"].includes(s.state))
        throw new Error(s.error ?? s.state);
      return s.state === "completed";
    },
    30000,
  );
  assert.equal(calls.length, 2);
  assert.ok(calls.every((c) => c.session_id === `aesthetic-${stage.id}`));
  checks.push(
    "two image batches share stage affinity; a separate stage has its own session",
  );
  assert.equal(completed.accepted, 2);
  assert.deepEqual(completed.usage_summary, {
    recorded_requests: 2,
    cache_observed_requests: 2,
    cache_hit_requests: 1,
    cached_input_tokens: 80,
    cache_observed_input_tokens: 200,
    cache_write_observed_requests: 2,
    cache_write_tokens: 80,
    cost_observed_requests: 2,
    cost_usd: 0.002,
  });
  const batches = await client.aesthetic.batches(project.id, stage.id);
  const attempts = await client.aesthetic.attempts(
    project.id,
    stage.id,
    batches.items[0].sequence,
  );
  assert.equal(attempts.items[0].receipt.usage.service_tier, "flex");
  checks.push(
    "receipt route and incremental cache/cost totals survive the full paid-ledger path",
  );
  assert.deepEqual(errors, []);
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify({ passed: true, checks, mock_calls: calls.length }, null, 2),
  );
  console.log(
    JSON.stringify({
      passed: true,
      checks: checks.length,
      mock_calls: calls.length,
      report: resolve(run, "report.json"),
    }),
  );
} finally {
  client?.dispose();
  await engine.stop();
  server.closeAllConnections();
  await new Promise((done) => server.close(done));
}
