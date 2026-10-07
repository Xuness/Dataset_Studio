import { pythonCommand } from "./platform.mjs";
// All network calls target this loopback mock; fixtures never open a real project.
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { randomUUID } from "node:crypto";
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
  ".local/test-runs/integration-aesthetic-execution-" + Date.now(),
);
await mkdir(run, { recursive: true });
await promisify(execFile)(
  pythonCommand(),
  [resolve(root, "tooling/ranking-fixture.py"), resolve(run, "fixture"), "320"],
  { cwd: root, windowsHide: true },
);
const fixture = JSON.parse(
  await readFile(resolve(run, "fixture/fixture.json"), "utf8"),
);
const engine = new EngineFixture(root, resolve(run, "state"));
const { StudioClient } = await clientFixture(root, resolve(run, "sdk"));
const mock = {
  mode: "valid",
  ordinal: 0,
  calls: [],
  release: false,
  errors: [],
};
function mode(value) {
  mock.mode = value;
  mock.ordinal = 0;
}
const server = createServer(async (req, res) => {
  res.on("error", () => {});
  try {
    const chunks = [];
    for await (const b of req) chunks.push(b);
    const body = JSON.parse(Buffer.concat(chunks));
    const protocol = body.contents
      ? "gemini"
      : body.input
        ? "responses"
        : "chat";
    const streamed = body.stream || req.url.includes(":streamGenerateContent");
    const blocks =
      protocol === "gemini"
        ? body.contents.flatMap((m) => m.parts)
        : (body.messages ?? body.input)
            .filter((m) => m.role === "user")
            .flatMap((m) => m.content);
    const ids = blocks.map((b) => b.text).filter((v) => /^img\d{2}$/.test(v));
    assert.ok(ids.length >= 2 && ids.length <= 16);
    const current = mock.mode,
      ordinal = ++mock.ordinal;
    mock.calls.push({
      mode: current,
      ordinal,
      stream: !!streamed,
      protocol,
      at: Date.now(),
      count: ids.length,
    });
    if (current === "once-rate" && ordinal === 1) {
      res.writeHead(429, {
        "content-type": "application/json",
        "retry-after": "1",
      });
      res.end('{"error":{"message":"rate limited"}}');
      return;
    }
    if (current === "reject") {
      res.writeHead(401, { "content-type": "application/json" });
      res.end('{"error":{"message":"authentication"}}');
      return;
    }
    if (current === "slow-first") await sleep(700);
    if (res.destroyed) return;
    const abstainCount =
      current === "once-empty" && ordinal === 1
        ? ids.length
        : current === "once-single" && ordinal === 1
          ? ids.length - 1
          : (current === "once-abstain" && ordinal === 1) ||
              (current === "after-exposure" && ordinal === 2)
            ? 1
            : 0;
    const judged = ids.slice(abstainCount);
    const text = JSON.stringify({
      schema_version: 1,
      tiers: [judged.slice(0, 4), judged.slice(4)].filter((t) => t.length),
      elite_candidates: judged.length ? [judged.at(-1)] : [],
      unjudgeable: ids.slice(0, abstainCount).map((id) => ({
        id,
        reason: "transient model abstention",
      })),
    });
    if (!streamed) {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(
        JSON.stringify({
          id: "json-result",
          choices: [
            { index: 0, message: { content: text }, finish_reason: "stop" },
          ],
          usage: { prompt_tokens: 13, completion_tokens: 7 },
        }),
      );
      return;
    }
    if (protocol === "chat")
      assert.equal(body.stream_options.include_usage, true);
    res.writeHead(200, {
      "content-type": "text/event-stream",
      "x-generation-id": "fixture-generation",
      "set-cookie": "do-not-persist",
    });
    const event = (value) => res.write(`data: ${JSON.stringify(value)}\n\n`);
    res.write(": OPENROUTER PROCESSING\n\n");
    if (protocol === "responses") {
      event({ type: "response.output_text.delta", delta: text.slice(0, 20) });
      await sleep(80);
      event({
        type: "response.completed",
        response: {
          id: "responses-result",
          model: "fixture",
          status: "completed",
          output: [
            { type: "message", content: [{ type: "output_text", text }] },
          ],
          usage: { input_tokens: 13, output_tokens: 7, total_tokens: 20 },
        },
      });
      res.end();
      return;
    }
    if (protocol === "gemini") {
      const cut = Math.floor(text.length / 2);
      event({
        responseId: "gemini-result",
        candidates: [
          { index: 0, content: { parts: [{ text: text.slice(0, cut) }] } },
        ],
      });
      await sleep(60);
      event({
        candidates: [
          {
            index: 0,
            content: { parts: [{ text: text.slice(cut) }] },
            finishReason: "STOP",
          },
        ],
      });
      await sleep(100);
      event({
        usageMetadata: {
          promptTokenCount: 13,
          candidatesTokenCount: 7,
          totalTokenCount: 20,
        },
      });
      res.end();
      return;
    }
    if (current === "heartbeat-total") {
      for (let n = 0; n < 80 && !res.destroyed; n++) {
        await sleep(50);
        if (!res.destroyed) res.write(": keepalive\n\n");
      }
      if (!res.destroyed) res.end();
      return;
    }
    if (
      current === "partial" ||
      (["once-drop", "first-drop"].includes(current) && ordinal === 1)
    ) {
      event({
        id: "partial",
        choices: [{ index: 0, delta: { content: "{" } }],
      });
      if (current === "partial") await sleep(1200);
      else await sleep(40);
      res.destroy();
      return;
    }
    if (current === "queue" || current === "hold") {
      const started = Date.now();
      while (
        !res.destroyed &&
        (current === "queue" ? Date.now() - started < 1000 : !mock.release)
      ) {
        await sleep(100);
        if (!res.destroyed) res.write(": keepalive\n\n");
      }
      if (res.destroyed) return;
    }
    event({
      id: "stream-result",
      choices: [{ index: 0, delta: { reasoning: "fixture progress" } }],
    });
    const cut = Math.floor(text.length / 2);
    event({
      id: "stream-result",
      choices: [{ index: 0, delta: { content: text.slice(0, cut) } }],
    });
    await sleep(60);
    if (res.destroyed) return;
    event({
      choices: [
        {
          index: 0,
          delta: { content: text.slice(cut) },
          finish_reason: "stop",
        },
      ],
    });
    // Usage is deliberately delayed after finish_reason, and must still be retained.
    await sleep(100);
    if (res.destroyed) return;
    event({
      choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
      usage: { prompt_tokens: 13, completion_tokens: 7, total_tokens: 20 },
    });
    if (current !== "no-terminal") res.write("data: [DONE]\n\n");
    res.end();
  } catch (error) {
    mock.errors.push(String(error));
    if (!res.headersSent) res.writeHead(500);
    if (!res.destroyed) res.end(String(error));
  }
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
const endpoint = `http://127.0.0.1:${server.address().port}`;
const policy = {
  stream: true,
  concurrency: 1,
  connect_timeout_ms: 1000,
  first_response_timeout_ms: 2000,
  idle_timeout_ms: 600,
  request_timeout_ms: 4000,
  batch_timeout_ms: 12000,
  max_retries: 0,
  retry_unknown: false,
  exhausted: "pause",
};
let client, project, provider, model, prompt, single, many;
const checks = [],
  cases = {};
const path = (id) => `/v1/projects/${project.id}/aesthetic/stages/${id}`;
const waitStage = (id, predicate) => engine.wait(path(id), predicate, 60000);
const stop = (id) =>
  waitStage(id, (s) =>
    [
      "completed",
      "completed_with_exclusions",
      "needs_attention",
      "paused",
      "failed",
    ].includes(s.state),
  );
async function until(predicate) {
  const end = Date.now() + 15000;
  while (!(await predicate()) && Date.now() < end) await sleep(30);
  assert.ok(await predicate());
}
function ledger(query, ...args) {
  const db = new DatabaseSync(resolve(project.directory, "evaluation.sqlite"), {
    readOnly: true,
  });
  try {
    return db.prepare(query).all(...args);
  } finally {
    db.close();
  }
}
async function workset(name, keys) {
  const selection = await client.selection(project.id);
  await client.changeSelection(project.id, {
    expected_revision: selection.revision,
    clear: true,
    add: keys,
    remove: [],
  });
  return client.createCollection(project.id, name);
}
function configuration(collection = single, overrides = {}, extra = {}) {
  const execution_policy = { ...policy, ...overrides };
  return {
    idempotency_key: randomUUID(),
    name: "执行与恢复 " + mock.mode,
    collection_id: collection.id,
    model_id: model.id,
    system_prompt_id: prompt.id,
    exposures: 1,
    max_calls: 20,
    concurrency: execution_policy.concurrency,
    overrides: {},
    execution_policy,
    budget_mode: "complete",
    sampling: {
      mode: "balanced",
      min_exposures: 1,
      max_exposures: 1,
      rank_tolerance: 0.1,
      seed: 17,
    },
    ...extra,
  };
}
async function create(collection = single, overrides = {}, extra = {}) {
  const s = await client.aesthetic.create(
    project.id,
    configuration(collection, overrides, extra),
  );
  const ready = await waitStage(s.id, (v) =>
    ["ready", "needs_attention", "failed"].includes(v.state),
  );
  assert.equal(ready.state, "ready", ready.error);
  return ready;
}
async function runStage(collection = single, overrides = {}, extra = {}) {
  const s = await create(collection, overrides, extra);
  await client.aesthetic.control(project.id, s.id, "start");
  return stop(s.id);
}
async function fit(stageId) {
  const job = await client.aesthetic.analysis.create(project.id, {
    idempotency_key: randomUUID(),
    name: "恢复验证快照",
    spec: {
      kind: "fit",
      config: {
        stage_id: stageId,
        estimator: {
          kind: "davidson_v2",
          iterations: 128,
          regularization: 0.001,
          tie_strength: 0.1,
        },
        stability_seed: null,
      },
      experiment_id: null,
      variant: null,
    },
  });
  return engine.wait(
    `/v1/projects/${project.id}/aesthetic/analysis/jobs/${job.id}`,
    (s) => s.state === "completed",
    60000,
  );
}
try {
  await engine.start();
  client = new StudioClient(engine.connection);
  project = await client.createProject({
    name: "美学执行恢复隔离测试",
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
    .filter((v) => v.number >= 32 && v.number < 288 && v.number % 4 === 0)
    .map((v) => ({ source_id: source.id, asset_id: v.sha }));
  single = await workset("十六图", assets.slice(0, 16));
  many = await workset("六十四图", assets);
  assert.equal(many.count, 64);
  prompt = await client.llm.systemPrompts.save({
    expected_revision: 0,
    config: {
      name: "模拟审美标准",
      description: "loopback only",
      text: "比较合成图片，允许同梯队。",
    },
  });
  provider = await client.llm.providers.save({
    expected_revision: 0,
    api_key: "fixture-secret",
    config: {
      name: "流式模拟供应商",
      kind: "openai_compatible",
      base_url: endpoint,
      enabled: true,
      headers: {},
      network: {
        ...network,
        idle_timeout_ms: 200,
        request_timeout_ms: 3000,
        max_concurrency: 1,
        rate_limit_retries: 2,
      },
    },
  });
  model = await client.llm.models.save({
    provider_id: provider.id,
    expected_revision: 0,
    config: {
      name: "模拟多模态模型",
      remote_model_id: "fixture",
      protocol: "openai_chat",
      enabled: true,
      parameters: {},
      capability_overrides: { input_image: "supported" },
    },
  });

  const insufficient = configuration(
    many,
    {},
    {
      exposures: 2,
      max_calls: 1,
      sampling: {
        mode: "balanced",
        min_exposures: 2,
        max_exposures: 2,
        rank_tolerance: 0.1,
        seed: 17,
      },
    },
  );
  const rejected = await client.aesthetic.preflight(project.id, insufficient);
  assert.equal(rejected.admitted, false);
  assert.equal(rejected.minimum_calls_lower_bound, 8);
  assert.equal(
    (
      await client.aesthetic.preflight(project.id, {
        ...insufficient,
        budget_mode: "trial",
      })
    ).admitted,
    true,
  );
  await assert.rejects(
    () => client.aesthetic.create(project.id, insufficient),
    (e) => e.code === "EVALUATION_BUDGET_INSUFFICIENT",
  );
  checks.push(
    "complete-budget admission rejects insufficient exposure budget; an explicit trial remains possible",
  );

  mode("valid");
  const valid = await runStage();
  cases.valid = valid.id;
  assert.equal(valid.state, "completed");
  assert.equal(valid.input_tokens, 13);
  assert.equal(valid.output_tokens, 7);
  const firstBatch = (await client.aesthetic.batches(project.id, valid.id))
    .items[0];
  const raw = ledger(
    "SELECT r.metadata,r.body FROM raw_receipts r JOIN attempts a ON a.id=r.attempt_id WHERE a.batch=?",
    firstBatch.sequence,
  )[0];
  const metadata = JSON.parse(raw.metadata);
  assert.equal(metadata.adapter_version, "native_sse_v1");
  assert.equal(metadata.complete, true);
  assert.equal(metadata.headers["set-cookie"], undefined);
  assert.equal(metadata.provider_request_id, "fixture-generation");
  assert.ok(Buffer.from(raw.body).toString().includes("[DONE]"));
  assert.ok(
    ledger(
      "SELECT adapter_version FROM receipt_parses WHERE attempt_id=?",
      firstBatch.attempt_id,
    ).every((r) => r.adapter_version === "native_sse_v1"),
  );
  const beforeParse = mock.calls.length;
  await client.aesthetic.reparse(project.id, valid.id, firstBatch.sequence);
  assert.equal(mock.calls.length, beforeParse);
  checks.push(
    "streaming preserves raw SSE, terminal marker, delayed usage and request ID; local replay purchases no call",
  );

  mode("valid");
  const jsonMode = await runStage(single, { stream: false });
  assert.equal(jsonMode.state, "completed");
  assert.equal(
    ledger(
      "SELECT json_extract(r.metadata,'$.adapter_version') adapter FROM raw_receipts r JOIN attempts a ON a.id=r.attempt_id JOIN batches b ON b.sequence=a.batch WHERE b.stage_id=?",
      jsonMode.id,
    )[0].adapter,
    "native_json_v1",
  );
  checks.push(
    "the non-streaming option retains complete JSON receipts and uses the same explicit execution policy",
  );
  for (const protocol of ["openai_responses", "gemini"]) {
    const connection =
      protocol === "gemini"
        ? await client.llm.providers.save({
            expected_revision: 0,
            api_key: "fixture-secret",
            config: {
              ...provider.config,
              name: "Gemini native fixture",
              kind: "gemini",
            },
          })
        : provider;
    const native = await client.llm.models.save({
      provider_id: connection.id,
      expected_revision: 0,
      config: { ...model.config, name: "Native " + protocol, protocol },
    });
    const completed = await runStage(single, {}, { model_id: native.id });
    assert.equal(completed.state, "completed");
    assert.equal(completed.input_tokens, 13);
    assert.equal(completed.output_tokens, 7);
  }
  checks.push(
    "recorded Responses and Gemini SSE preserve protocol completion and final usage metadata",
  );
  mode("slow-first");
  const delayed = await runStage(single, {
    first_response_timeout_ms: 1500,
    idle_timeout_ms: 250,
    request_timeout_ms: 3000,
  });
  assert.equal(delayed.state, "completed");
  checks.push(
    "first response may exceed both provider and stream idle limits without timing out",
  );
  mode("slow-first");
  const noRaw = await runStage(
    single,
    {
      first_response_timeout_ms: 200,
      idle_timeout_ms: 100,
      request_timeout_ms: 500,
    },
    { name: "首包超时无回执" },
  );
  cases.noRaw = noRaw.id;
  assert.equal(noRaw.unknown, 1);
  assert.equal(
    (await client.aesthetic.batches(project.id, noRaw.id)).items[0]
      .has_raw_receipt,
    false,
  );
  checks.push(
    "timeout before response headers exposes no invented raw receipt and cannot be locally reparsed",
  );
  mode("heartbeat-total");
  const start = Date.now();
  const total = await runStage(single, {
    first_response_timeout_ms: 400,
    idle_timeout_ms: 200,
    request_timeout_ms: 500,
  });
  assert.equal(total.unknown, 1);
  assert.equal(total.accepted, 0);
  assert.ok(Date.now() - start < 15000);
  assert.ok(
    ledger(
      "SELECT length(r.body) n FROM raw_receipts r JOIN attempts a ON a.id=r.attempt_id JOIN batches b ON b.sequence=a.batch WHERE b.stage_id=?",
      total.id,
    )[0].n > 30,
  );
  checks.push(
    "heartbeats reset idle but never extend the fixed total deadline; timed-out prefix is retained",
  );
  mode("no-terminal");
  const noTerminal = await runStage();
  assert.equal(noTerminal.unknown, 1);
  assert.equal(noTerminal.accepted, 0);
  checks.push(
    "complete HTTP content without the required SSE terminal cannot become accepted evidence",
  );

  mode("first-drop");
  const isolated = await runStage(many);
  cases.isolated = isolated.id;
  assert.deepEqual(
    [
      isolated.attempts,
      isolated.accepted,
      isolated.unknown,
      isolated.progress.round_unclaimed,
    ],
    [4, 3, 1, 0],
  );
  const issues = await client.aesthetic.batches(
    project.id,
    isolated.id,
    undefined,
    undefined,
    { state: "issues" },
  );
  assert.equal(issues.items.length, 1);
  const blocked = await client.aesthetic.candidates(
    project.id,
    isolated.id,
    false,
    undefined,
    undefined,
    undefined,
    true,
  );
  assert.equal(blocked.items.length, 16);
  assert.ok(
    blocked.items.every(
      (c) =>
        c.blocked &&
        c.blocking_batch === issues.items[0].sequence &&
        c.disposition === "active",
    ),
  );
  assert.equal(issues.items[0].stage_sequence, 1);
  assert.ok(issues.items[0].sequence > 1);
  checks.push(
    "one unknown batch does not halt the remaining independent slots; locked candidates and stable local numbering are visible",
  );

  mode("once-drop");
  const retry = await runStage(single, { retry_unknown: true, max_retries: 1 });
  assert.deepEqual([retry.attempts, retry.accepted, retry.unknown], [2, 1, 0]);
  const retryBatch = (await client.aesthetic.batches(project.id, retry.id))
    .items[0];
  const history = (
    await client.aesthetic.attempts(project.id, retry.id, retryBatch.sequence)
  ).items;
  assert.deepEqual(
    history.map((a) => a.state),
    ["outcome_unknown", "accepted"],
  );
  assert.equal(history[1].execution_settings.policy.max_retries, 1);
  checks.push(
    "explicit unknown-outcome policy authorizes one durable backoff retry, with one accepted observation",
  );

  mode("once-rate");
  const rate = await runStage(single, { max_retries: 1 });
  assert.equal(rate.attempts, 2);
  assert.equal(rate.accepted, 1);
  assert.equal(mock.ordinal, 2);
  const rateCalls = mock.calls.filter((c) => c.mode === "once-rate");
  assert.ok(rateCalls[1].at - rateCalls[0].at >= 1000);
  checks.push(
    "Retry-After is honored by the ledger retry queue; no hidden transport retry or uncounted request occurs",
  );

  for (const abstention of ["once-empty", "once-single"]) {
    mode(abstention);
    const recovered = await runStage(single, { max_retries: 1 });
    assert.equal(recovered.state, "completed");
    assert.deepEqual(
      [recovered.attempts, recovered.accepted, recovered.unresolved],
      [2, 1, 0],
    );
    assert.equal(recovered.input_tokens, 26);
    const batches = (await client.aesthetic.batches(project.id, recovered.id))
      .items;
    assert.equal(batches.length, 1);
    const attempts = (
      await client.aesthetic.attempts(
        project.id,
        recovered.id,
        batches[0].sequence,
      )
    ).items;
    assert.deepEqual(
      attempts.map((a) => a.state),
      ["failed", "accepted"],
    );
    assert.equal(attempts[0].failure.code, "EVALUATION_NO_COMPARABLE_EVIDENCE");
    assert.equal(attempts[0].failure.outcome_unknown, false);
    assert.ok(attempts.every((a) => a.raw_receipt && a.receipt));
    assert.ok(
      ledger(
        "SELECT exposures,unjudgeable_streak,blocked,disposition FROM candidates WHERE stage_id=?",
        recovered.id,
      ).every(
        (c) =>
          c.exposures === 1 &&
          c.unjudgeable_streak === 0 &&
          c.blocked === 0 &&
          c.disposition === "active",
      ),
    );
  }
  checks.push(
    "zero or one comparable image retries the same batch and completes with one evidence record, both receipts and all usage retained",
  );

  for (const [abstention, exposures] of [
    ["once-abstain", 1],
    ["after-exposure", 2],
  ]) {
    mode(abstention);
    const recovered = await runStage(
      single,
      {},
      {
        exposures,
        sampling: {
          mode: "balanced",
          min_exposures: exposures,
          max_exposures: exposures + 1,
          rank_tolerance: 0.1,
          seed: 17,
        },
      },
    );
    assert.equal(recovered.state, "completed");
    assert.equal(recovered.unresolved, 0);
    assert.equal(recovered.attempts, exposures + 1);
    assert.equal(recovered.accepted, exposures + 1);
    assert.ok(
      ledger(
        "SELECT exposures,unjudgeable_streak,blocked,disposition FROM candidates WHERE stage_id=?",
        recovered.id,
      ).every(
        (c) =>
          c.exposures >= exposures &&
          c.unjudgeable_streak === 0 &&
          c.blocked === 0 &&
          c.disposition === "active",
      ),
    );
  }
  checks.push(
    "first and previously exposed candidates recover from a single abstention in a later batch and automatically complete sampling",
  );

  mode("once-drop");
  const interrupted = await create(single, {
    retry_unknown: true,
    max_retries: 1,
  });
  await client.aesthetic.control(project.id, interrupted.id, "start");
  await waitStage(interrupted.id, (s) => s.progress.retry_waiting === 1);
  const beforeRestart = mock.calls.length;
  await engine.stop();
  await engine.start();
  client.dispose();
  client = new StudioClient(engine.connection);
  await client.openProject(project.directory);
  assert.ok(
    ["paused", "needs_attention"].includes(
      (await client.aesthetic.stage(project.id, interrupted.id)).state,
    ),
  );
  await sleep(250);
  assert.equal(mock.calls.length, beforeRestart);
  await client.aesthetic.control(project.id, interrupted.id, "start");
  const resumed = await stop(interrupted.id);
  assert.equal(resumed.accepted, 1);
  assert.equal(resumed.attempts, 2);
  checks.push(
    "restart preserves the retry queue and requires explicit stage resume; it does not silently repurchase requests",
  );

  mode("partial");
  const partial = await runStage(single, { idle_timeout_ms: 200 });
  cases.partial = partial.id;
  assert.equal(partial.unknown, 1);
  const partialBatch = (await client.aesthetic.batches(project.id, partial.id))
    .items[0];
  assert.equal(partialBatch.has_raw_receipt, true);
  const beforeDefer = mock.calls.length;
  const deferred = await client.aesthetic.batchAction(project.id, partial.id, {
    idempotency_key: randomUUID(),
    action: "defer",
    batches: [partialBatch.sequence],
    reason: "fixture explicit defer",
    acknowledge_possible_charge: true,
  });
  assert.equal(deferred.succeeded, 1);
  assert.equal(mock.calls.length, beforeDefer);
  assert.equal(
    (await client.aesthetic.stage(project.id, partial.id)).progress.blocked,
    0,
  );
  await assert.rejects(() =>
    client.aesthetic.reparse(project.id, partial.id, partialBatch.sequence),
  );
  mode("valid");
  await client.aesthetic.control(project.id, partial.id, "start");
  const replaced = await stop(partial.id);
  assert.equal(replaced.accepted, 1);
  assert.equal(replaced.progress.deferred, 1);
  checks.push(
    "explicit deferral preserves the failed receipt, adds no exposure and permits later coverage without accepting the retired batch",
  );

  mode("reject");
  const denied = await runStage(many, { retry_unknown: true, max_retries: 2 });
  assert.equal(denied.attempts, 1);
  assert.equal(mock.ordinal, 1);
  assert.equal(denied.progress.round_unclaimed, 3);
  checks.push(
    "authentication errors stop stage dispatch and are not retried by a generic transport loop",
  );

  mode("queue");
  const queued = [];
  for (let n = 0; n < 3; n++)
    queued.push(
      await create(single, {
        request_timeout_ms: 2000,
        first_response_timeout_ms: 1500,
      }),
    );
  await Promise.all(
    queued.map((s) => client.aesthetic.control(project.id, s.id, "start")),
  );
  const queueResults = await Promise.all(queued.map((s) => stop(s.id)));
  assert.ok(queueResults.every((s) => s.state === "completed"));
  checks.push(
    "provider concurrency waits are excluded from each request deadline, including a wait longer than the network budget",
  );

  mode("hold");
  mock.release = false;
  const pause = await create(many, { concurrency: 2 });
  await client.aesthetic.control(project.id, pause.id, "start");
  await until(() => mock.ordinal === 1);
  await client.aesthetic.control(project.id, pause.id, "pause");
  mock.release = true;
  const paused = await waitStage(pause.id, (s) => s.state === "paused");
  assert.equal(paused.attempts, 1);
  assert.equal(mock.ordinal, 1);
  checks.push(
    "pause releases unsent provider-queue waiters while draining and preserving an already-sent response",
  );

  mode("valid");
  const unchanged = await create();
  const hash = unchanged.config_hash;
  provider = await client.llm.providers.save({
    id: provider.id,
    expected_revision: provider.revision,
    config: {
      ...provider.config,
      network: { ...provider.config.network, idle_timeout_ms: 300 },
    },
  });
  const beforeRebind = mock.calls.length;
  await assert.rejects(
    () => client.aesthetic.control(project.id, unchanged.id, "start"),
    (e) => e.code === "REVISION_CONFLICT",
  );
  assert.equal(mock.calls.length, beforeRebind);
  assert.equal(
    (await client.aesthetic.stage(project.id, unchanged.id)).state,
    "ready",
  );
  const executionUpdate = {
    idempotency_key: randomUUID(),
    expected_revision: 1,
    policy: { ...policy, idle_timeout_ms: 700 },
  };
  const rebound = await client.aesthetic.configureExecution(
    project.id,
    unchanged.id,
    executionUpdate,
  );
  assert.equal(rebound.config_hash, hash);
  assert.equal(rebound.execution_settings.provider_revision, provider.revision);
  const same = await client.aesthetic.configureExecution(
    project.id,
    unchanged.id,
    executionUpdate,
  );
  assert.equal(same.execution_settings.revision, 2);
  await client.aesthetic.control(project.id, unchanged.id, "start");
  assert.equal((await stop(unchanged.id)).state, "completed");
  checks.push(
    "explicit execution rebinding accepts network-only provider revisions and preserves the frozen aesthetic configuration",
  );

  const snapshot = await fit(valid.id);
  cases.snapshot = snapshot.id;
  assert.equal(
    (await client.aesthetic.analysis.latestForStage(project.id, valid.id)).id,
    snapshot.id,
  );
  assert.equal(
    await client.aesthetic.analysis.latestForStage(project.id, isolated.id),
    null,
  );
  const filter = { ratings: ["g"], rank_to: 3, include_protected: true };
  const preview = await client.aesthetic.analysis.select(
    project.id,
    snapshot.id,
    { filter, limit: 12, after: null },
  );
  const count = await client.aesthetic.analysis.create(project.id, {
    idempotency_key: randomUUID(),
    name: "筛选统计",
    spec: {
      kind: "preview",
      snapshot_id: snapshot.id,
      filter,
      review_watermark: preview.review_watermark,
    },
  });
  const counted = await engine.wait(
    `/v1/projects/${project.id}/aesthetic/analysis/jobs/${count.id}`,
    (s) => s.state === "completed",
    60000,
  );
  assert.deepEqual(
    [
      counted.result.count,
      counted.result.ranked_count,
      counted.result.protected_added,
      counted.result.boundary_tie_count,
    ],
    [5, 4, 1, 4],
  );
  const protectedRow = preview.items.find((v) => v.effective_protected);
  assert.ok(protectedRow);
  await client.aesthetic.analysis.review(project.id, {
    idempotency_key: randomUUID(),
    snapshot_id: snapshot.id,
    ordinal: protectedRow.ranking.ordinal,
    decision: "release",
    reviewer: "fixture",
    reason: "after frozen preview",
  });
  const derived = await client.aesthetic.analysis.create(project.id, {
    idempotency_key: randomUUID(),
    name: "Top 3 保留并列与保护",
    spec: {
      kind: "derive",
      snapshot_id: snapshot.id,
      filter,
      review_watermark: preview.review_watermark,
    },
  });
  const derivedResult = await engine.wait(
    `/v1/projects/${project.id}/aesthetic/analysis/jobs/${derived.id}`,
    (s) => s.state === "completed",
    60000,
  );
  assert.equal(derivedResult.result.count, 5);
  checks.push(
    "stage-specific snapshots avoid unrelated results; Top N preview counts ties/protection exactly and derivation retains the preview review watermark",
  );
  await client.aesthetic.stageMetadata(project.id, valid.id, {
    name: "已归档的恢复验证",
    archived: true,
  });
  const archived = await client.aesthetic.stages(
    project.id,
    undefined,
    undefined,
    { archived: true, search: "恢复验证" },
  );
  assert.equal(archived.items[0].id, valid.id);
  assert.equal(
    (await client.aesthetic.analysis.latestForStage(project.id, valid.id)).id,
    snapshot.id,
  );
  await client.aesthetic.stageMetadata(project.id, valid.id, {
    name: "流式成功与快照",
    archived: false,
  });
  checks.push(
    "archive/restore and naming preserve paid evidence and snapshots",
  );
  assert.deepEqual(mock.errors, []);
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      {
        passed: true,
        checks,
        cases,
        project: project.directory,
        projectId: project.id,
        calls: mock.calls,
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
      {
        passed: false,
        checks,
        cases,
        project: project?.directory,
        error: String(error),
        calls: mock.calls,
        mockErrors: mock.errors,
      },
      null,
      2,
    ),
  );
  throw error;
} finally {
  mock.release = true;
  client?.dispose();
  await engine.stop();
  server.closeAllConnections();
  await new Promise((done) => server.close(done));
}
