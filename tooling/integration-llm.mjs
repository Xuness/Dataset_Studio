import assert from "node:assert/strict";
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { EngineFixture, sleep } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";
import { llmFixture, network } from "./llm-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(root, ".local/test-runs/integration-llm-" + Date.now());
await mkdir(run, { recursive: true });
const mock = await llmFixture(),
  engine = new EngineFixture(root, resolve(run, "state"));
const checks = [];
const { StudioClient } = await clientFixture(root, resolve(run, "sdk"));
let client;
const request = (model, overrides = {}) => ({
  model_id: model.id,
  messages: [
    { role: "user", content: [{ type: "text", text: "fixture request" }] },
  ],
  overrides,
  tools: [],
});
const createModel = (provider, protocol, id = "text-model", parameters = {}) =>
  client.llm.models.save({
    provider_id: provider.id,
    expected_revision: 0,
    config: {
      name: id,
      remote_model_id: id,
      protocol,
      enabled: true,
      parameters,
      capability_overrides: {},
    },
  });
try {
  await engine.start();
  client = new StudioClient(engine.connection);
  const saved = [];
  for (const [kind, protocol, path] of [
    ["openai", "openai_chat", "openai"],
    ["openai_compatible", "openai_responses", "compatible"],
    ["openrouter", "openai_chat", "router"],
    ["gemini", "gemini", "gemini"],
  ]) {
    const p = await client.llm.providers.save({
      expected_revision: 0,
      config: {
        name: kind,
        kind,
        base_url: mock.url + "/" + path,
        enabled: true,
        headers: {},
        network,
      },
      api_key: "fixture-secret-value",
    });
    assert.equal(p.credential_set, true);
    assert.equal(JSON.stringify(p).includes("fixture-secret-value"), false);
    const catalog = await client.llm.refreshModels(p.id, p.revision);
    assert.ok(catalog.models.length > 0);
    const m = await createModel(
      p,
      protocol,
      kind === "gemini" ? "models/text-model" : "text-model",
      { temperature: 0.6, max_output_tokens: 200 },
    );
    const m2 = await createModel(p, protocol, "other-model", {
      temperature: 0.2,
      max_output_tokens: 400,
    });
    const prepared = await client.llm.prepare(request(m, { temperature: 0 }));
    assert.equal(prepared.snapshot.parameters.temperature, 0);
    assert.equal(
      JSON.stringify(prepared).includes("fixture-secret-value"),
      false,
    );
    const result = await client.llm.generate(request(m));
    assert.equal(result.usage.input_tokens, 7);
    assert.equal(result.provider_request_id, "fixture-upstream-id");
    assert.equal(result.outputs[0].content[0].text, "你好 OK");
    const events = [];
    for await (const event of client.llm.stream(request(m))) events.push(event);
    assert.equal(events[0].type, "started");
    assert.equal(events.at(-1).type, "completed");
    assert.equal(events.at(-1).response.usage.output_tokens, 3);
    assert.equal(
      events
        .filter((e) => e.type === "delta" && e.kind === "text")
        .map((e) => e.text)
        .join(""),
      "你好 OK",
    );
    await client.llm.refreshModels(p.id, p.revision);
    assert.deepEqual(
      (await client.llm.models.list(p.id)).items.find((x) => x.id === m2.id)
        .config.parameters,
      { temperature: 0.2, max_output_tokens: 400 },
    );
    saved.push({ p, m });
    checks.push(
      kind +
        " discovery, complete/streaming inference, independent model configuration",
    );
  }
  const { p, m } = saved[0];
  const omittedStore = await client.llm.prepare(request(m, { store: null }));
  assert.equal("store" in omittedStore.snapshot.parameters, false);
  assert.equal("store" in omittedStore.native_request, false);
  const preset = await client.llm.presets.save({
    expected_revision: 0,
    config: {
      name: "低温预设",
      protocol: "openai_chat",
      parameters: { temperature: 0.1, max_output_tokens: 80 },
    },
  });
  const composed = await client.llm.prepare({
    ...request(m, { max_output_tokens: null, store: false }),
    preset_id: preset.id,
  });
  assert.equal(composed.snapshot.parameters.temperature, 0.1);
  assert.equal(composed.snapshot.parameters.store, false);
  assert.equal("max_output_tokens" in composed.snapshot.parameters, false);
  await assert.rejects(
    () => client.llm.prepare(request(m, { "openrouter.provider": {} })),
    (e) => e.code === "INVALID_INPUT",
  );
  await assert.rejects(
    () =>
      client.llm.providers.save({
        id: p.id,
        expected_revision: 0,
        config: p.config,
      }),
    (e) => e.code === "REVISION_CONFLICT",
  );
  checks.push(
    "parameter precedence, explicit omission and unsupported namespaces; optimistic revisions",
  );
  mock.state.catalogFailure = true;
  await assert.rejects(() => client.llm.refreshModels(p.id, p.revision));
  mock.state.catalogFailure = false;
  assert.ok((await client.llm.models.catalog(p.id)).catalog.models.length);
  checks.push("failed refresh retains previous catalog");
  const broken = await createModel(p, "openai_chat", "stream-break");
  const brokenEvents = [];
  for await (const e of client.llm.stream(request(broken)))
    brokenEvents.push(e);
  assert.equal(brokenEvents.at(-1).type, "failed");
  assert.equal(brokenEvents.at(-1).error.code, "LLM_STREAM_INTERRUPTED");
  assert.equal(brokenEvents.at(-1).error.outcome_unknown, true);
  const streamError = await createModel(p, "openai_chat", "stream-error");
  const errors = [];
  for await (const e of client.llm.stream(request(streamError))) errors.push(e);
  assert.equal(errors.at(-1).type, "failed");
  const error = await createModel(p, "openai_chat", "error-model");
  await assert.rejects(
    () => client.llm.generate(request(error)),
    (e) => e.code === "LLM_UPSTREAM",
  );
  assert.equal(mock.state.attempts.get("error-model"), 1);
  checks.push(
    "truncated and error streams fail explicitly; 500 is not replayed",
  );
  const slow = await createModel(p, "openai_chat", "slow-model");
  mock.state.peak = 0;
  await Promise.all(
    Array.from({ length: 5 }, () => client.llm.generate(request(slow))),
  );
  assert.ok(mock.state.peak <= 2);
  const cancel = new AbortController();
  const pending = client.llm.generate(request(slow), cancel.signal);
  await sleep(50);
  cancel.abort();
  await assert.rejects(() => pending);
  const id = crypto.randomUUID();
  await client.llm.cancel(id);
  await assert.rejects(
    () => client.llm.generate({ ...request(m), invocation_id: id }),
    (e) => e.code === "CANCELLED",
  );
  checks.push(
    "provider concurrency limit, cancellation during request and cancellation before registration",
  );
  const rateProvider = await client.llm.providers.save({
    id: p.id,
    expected_revision: p.revision,
    config: { ...p.config, network: { ...network, rate_limit_retries: 1 } },
  });
  const rate = await createModel(rateProvider, "openai_chat", "rate-model");
  await client.llm.generate(request(rate));
  assert.equal(mock.state.attempts.get("rate-model"), 2);
  assert.equal(composed.snapshot.provider_revision, 1);
  assert.equal(composed.snapshot.parameters.temperature, 0.1);
  checks.push(
    "explicit bounded 429 retry and immutable configuration snapshot",
  );
  mock.state.catalogDelay = 200;
  const refresh = client.llm.refreshModels(
    rateProvider.id,
    rateProvider.revision,
  );
  await sleep(60);
  await client.llm.providers.save({
    id: p.id,
    expected_revision: rateProvider.revision,
    config: { ...rateProvider.config, name: "changed during discovery" },
  });
  await assert.rejects(
    () => refresh,
    (e) => e.code === "REVISION_CONFLICT",
  );
  mock.state.catalogDelay = 0;
  checks.push(
    "late discovery cannot overwrite catalog after connection revision changes",
  );
  await client.llm.presets.remove(preset.id, preset.revision);
  await assert.rejects(
    () => client.llm.providers.remove(p.id, 3),
    (e) => e.code === "OBJECT_IN_USE",
  );
  client.dispose();
  await engine.stop();
  await engine.start();
  client = new StudioClient(engine.connection);
  assert.equal((await client.llm.providers.list()).items.length, 4);
  assert.equal(
    (await client.llm.models.list(p.id)).items.find((x) => x.id === m.id).config
      .parameters.temperature,
    0.6,
  );
  await client.llm.generate(request(m));
  checks.push(
    "providers, per-model settings and protected credentials survive engine restart",
  );
  const registry = resolve(run, "state/registry.sqlite");
  const db = new DatabaseSync(registry, { readOnly: true });
  assert.equal(db.prepare("PRAGMA user_version").get().user_version, 4);
  db.close();
  assert.equal(
    (await readFile(registry)).includes(Buffer.from("fixture-secret-value")),
    false,
  );
  for (const file of await readdir(resolve(run, "state/llm-credentials")))
    assert.equal(
      (await readFile(resolve(run, "state/llm-credentials", file))).includes(
        Buffer.from("fixture-secret-value"),
      ),
      false,
    );
  assert.ok(mock.state.calls.some((c) => c.geminiKey));
  checks.push(
    "credential-free DTOs and registry; encrypted Windows credential files; native authentication mapping",
  );
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify({ passed: true, checks }, null, 2),
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
    JSON.stringify({ passed: false, checks, error: String(error) }, null, 2),
  );
  throw error;
} finally {
  client?.dispose();
  await engine.stop();
  await mock.close();
}
