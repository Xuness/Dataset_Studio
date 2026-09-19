// R0/R1: real engine/SQLite/HTTP, paid calls replaced by a loopback provider.
// Fault hooks exist only in this explicitly built test binary, never ordinary builds.
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { randomUUID } from "node:crypto";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, readFile, writeFile, unlink, open } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { cargo, finished } from "./cargo.mjs";
import { EngineFixture, sleep } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";
import { network } from "./llm-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const stamp = Date.now();
const run = resolve(root, ".local/test-runs/aesthetic-recovery-" + stamp);
const faults = resolve(run, "faults");
await mkdir(faults, { recursive: true });
await mkdir(resolve(root, ".local/logs"), { recursive: true });
const buildLog = await open(
  resolve(root, `.local/logs/aesthetic-recovery-build-${stamp}.log`),
  "w",
);
try {
  await finished(
    cargo(["build", "-p", "studio-engine", "--features", "test-faults"], {
      cwd: root,
      windowsHide: true,
      stdio: ["ignore", buildLog.fd, buildLog.fd],
    }),
  );
} finally {
  await buildLog.close();
}
await promisify(execFile)(
  "python",
  [resolve(root, "tooling/ranking-fixture.py"), resolve(run, "fixture"), "128"],
  { cwd: root, windowsHide: true },
);
const fixture = JSON.parse(
  await readFile(resolve(run, "fixture/fixture.json"), "utf8"),
);
const { StudioClient } = await clientFixture(root, resolve(run, "sdk"));
const engine = new EngineFixture(
  root,
  resolve(run, "中文运行环境"),
  resolve(run, "logs"),
  { profile: "debug", env: { STUDIO_TEST_FAULT_DIR: faults } },
);
const mock = { calls: [], mode: "valid" };
const server = createServer(async (req, res) => {
  res.on("error", () => {});
  try {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    const body = JSON.parse(Buffer.concat(chunks));
    const parts = body.messages
      .filter((m) => m.role === "user")
      .flatMap((m) => m.content);
    const ids = parts.map((v) => v.text).filter((v) => /^img\d{2}$/.test(v));
    assert.ok(ids.length >= 2 && ids.length <= 16);
    assert.equal(parts.filter((v) => v.image_url).length, ids.length);
    mock.calls.push({
      mode: mock.mode,
      ids,
      messages: body.messages,
      bytes: Buffer.concat(chunks).length,
    });
    res.writeHead(200, {
      "content-type": "application/json",
      "x-request-id": "r1-provider-request",
    });
    if (mock.mode === "invalid_http_json") {
      res.end("{broken");
      return;
    }
    if (mock.mode === "invalid_provider_shape") {
      res.end("{}");
      return;
    }
    const abstain = mock.mode === "unjudgeable";
    const judged = abstain ? ids.slice(1) : ids;
    res.end(
      JSON.stringify({
        id: "r1-response",
        model: "fixture",
        choices: [
          {
            index: 0,
            message: {
              content: JSON.stringify({
                schema_version: 1,
                tiers: [judged],
                elite_candidates: [judged.at(-1)],
                unjudgeable: abstain
                  ? [{ id: ids[0], reason: "mock unreadable image" }]
                  : [],
              }),
            },
            finish_reason: "stop",
          },
        ],
        usage: { prompt_tokens: 13, completion_tokens: 7, total_tokens: 20 },
      }),
    );
  } catch (error) {
    if (!res.headersSent) res.writeHead(500);
    res.end(String(error));
  }
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
const projects = [];
const checks = [];
let client, model, prompt;
const route = (p, id) => `/v1/projects/${p.id}/aesthetic/stages/${id}`;
const hook = (point, id) => resolve(faults, `${point}-${id}`);
async function until(fn, test, timeout = 30000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const value = await fn();
    if (test(value)) return value;
    await sleep(40);
  }
  throw new Error("R1 condition timed out");
}
async function waitHit(point, id) {
  return until(
    () => readFile(hook(point, id) + ".hit", "utf8").catch(() => null),
    Boolean,
  );
}
async function start() {
  await engine.start();
  client = new StudioClient(engine.connection);
  for (const p of projects) await client.openProject(p.directory);
}
async function restart(crash = false) {
  client?.dispose();
  await engine.stop(crash);
  await start();
}
async function makeProject() {
  const p = await client.createProject({
    name: "R1 中文项目 " + projects.length,
    parent_directory: null,
  });
  const source = await engine.api(`/v1/projects/${p.id}/sources`, "POST", {
    kind: "danbooru",
    name: "只读合成图源",
    index_root: fixture.lake,
    media_root: fixture.lake,
  });
  const keys = fixture.objects
    .filter((v) => v.number >= 32 && v.number < 96 && v.number % 4 === 0)
    .map((v) => ({ source_id: source.id, asset_id: v.sha }));
  const selected = await client.selection(p.id);
  await client.changeSelection(p.id, {
    expected_revision: selected.revision,
    add: keys,
    remove: [],
    clear: true,
  });
  const collection = await client.createCollection(p.id, "单 Rating 十六图");
  Object.assign(p, { source, collection });
  projects.push(p);
  return p;
}
function request(p, extra = {}) {
  return {
    idempotency_key: randomUUID(),
    name: "R1 stage",
    collection_id: p.collection.id,
    model_id: model.id,
    system_prompt_id: prompt.id,
    overrides: {},
    exposures: 1,
    max_calls: 20,
    concurrency: 1,
    ...extra,
  };
}
async function waitState(p, id, state) {
  return engine.wait(route(p, id), (v) => v.state === state, 30000);
}
async function create(p, extra = {}) {
  const value = request(p, extra);
  await client.aesthetic.create(p.id, value);
  return waitState(p, value.idempotency_key, "ready");
}
async function complete(p, stage) {
  await client.aesthetic.control(p.id, stage.id, "start");
  return waitState(p, stage.id, "completed");
}
function readDb(p, name, fn) {
  const db = new DatabaseSync(resolve(p.directory, name), { readOnly: true });
  try {
    return fn(db);
  } finally {
    db.close();
  }
}
async function healthy(p) {
  return until(
    () => client.aesthetic.metrics(p.id),
    (v) => v.dispatch_health === "healthy",
  );
}
try {
  await start();
  const p = await makeProject();
  const other = await makeProject();
  prompt = await client.llm.systemPrompts.save({
    expected_revision: 0,
    config: {
      name: "R1 standard",
      description: "mock",
      text: "frozen original system standard",
    },
  });
  const provider = await client.llm.providers.save({
    expected_revision: 0,
    api_key: "fixture-secret",
    config: {
      name: "R1 loopback",
      kind: "openai_compatible",
      base_url: `http://127.0.0.1:${server.address().port}`,
      enabled: true,
      headers: {},
      network: {
        ...network,
        request_timeout_ms: 30000,
        idle_timeout_ms: 30000,
        rate_limit_retries: 2,
      },
    },
  });
  model = await client.llm.models.save({
    provider_id: provider.id,
    expected_revision: 0,
    config: {
      name: "R1 mock",
      remote_model_id: "fixture",
      protocol: "openai_chat",
      enabled: true,
      parameters: {},
      capability_overrides: { input_image: "supported" },
    },
  });

  const preflightRequest = request(p);
  const preview = await client.aesthetic.preflight(p.id, preflightRequest);
  assert.equal(preview.admitted, true);
  assert.equal(preview.total, 16);
  assert.equal(
    (await client.aesthetic.capabilities(p.id)).max_stage_candidates,
    1000000,
  );
  assert.equal(mock.calls.length, 0);
  await client.aesthetic.create(p.id, {
    ...preflightRequest,
    expected_input_version: preview.input_version,
  });
  const frozen = await waitState(p, preflightRequest.idempotency_key, "ready");
  prompt = await client.llm.systemPrompts.save({
    id: prompt.id,
    expected_revision: prompt.revision,
    config: { ...prompt.config, text: "updated system standard" },
  });
  const frozenComplete = await complete(p, frozen);
  assert.equal(frozenComplete.config_hash, frozen.config_hash);
  assert.ok(
    JSON.stringify(mock.calls.at(-1).messages).includes(
      "frozen original system standard",
    ),
  );
  assert.ok(
    !JSON.stringify(mock.calls.at(-1).messages).includes(
      "updated system standard",
    ),
  );
  const frozenUser = frozen.config.model.messages
    .filter((m) => m.role === "user")
    .flatMap((m) => m.content)
    .find((c) => c.type === "text").text;
  assert.ok(
    mock.calls
      .at(-1)
      .messages.some(
        (m) =>
          m.role === "user" && m.content.some((c) => c.text === frozenUser),
      ),
  );
  const firstBatch = (await client.aesthetic.batches(p.id, frozen.id)).items[0];
  const firstAttempt = (
    await client.aesthetic.attempts(p.id, frozen.id, firstBatch.sequence)
  ).items[0];
  assert.match(firstAttempt.semantic_request_hash, /^[0-9a-f]{64}$/);
  assert.ok(!JSON.stringify(firstAttempt).includes("fixture-secret"));
  checks.push(
    "versioned capability/preflight, immutable system/user templates and credential-free semantic request digest on real HTTP",
  );

  for (const action of ["exclude", "rejudge"]) {
    mock.mode = "unjudgeable";
    const stage = await create(p);
    await client.aesthetic.control(p.id, stage.id, "start");
    const pending = await waitState(p, stage.id, "needs_attention");
    assert.deepEqual(
      [pending.total, pending.comparable, pending.excluded, pending.unresolved],
      [16, 15, 0, 1],
    );
    const candidate = (
      await client.aesthetic.candidates(p.id, stage.id)
    ).items.find((v) => v.disposition === "needs_review");
    assert.equal(candidate.exposures, 0);
    const before = (await client.aesthetic.batches(p.id, stage.id)).items[0];
    const decision = {
      idempotency_key: randomUUID(),
      action,
      reason: "explicit fixture decision",
    };
    await client.aesthetic.decideCandidate(
      p.id,
      stage.id,
      candidate.ordinal,
      decision,
    );
    await client.aesthetic.decideCandidate(
      p.id,
      stage.id,
      candidate.ordinal,
      decision,
    );
    mock.mode = "valid";
    if (action === "exclude") {
      const done = await waitState(p, stage.id, "completed_with_exclusions");
      assert.deepEqual(
        [done.excluded, done.comparable, done.unresolved, done.attempts],
        [1, 15, 0, 1],
      );
    } else {
      const done = await complete(p, stage);
      assert.equal(done.attempts, 2);
      assert.equal(done.comparable, 16);
      const batches = (await client.aesthetic.batches(p.id, stage.id)).items;
      assert.equal(batches.length, 2);
      assert.notEqual(batches[0].sequence, batches[1].sequence);
      assert.ok(
        batches[1].members.length >= 2 &&
          batches[1].members.every(
            (v) => v.candidate.rating === candidate.rating,
          ),
      );
    }
    assert.deepEqual(
      (await client.aesthetic.batches(p.id, stage.id)).items[0].observation,
      before.observation,
    );
  }
  checks.push(
    "15 judged plus one abstention: explicit exclusion or new same-Rating comparison, append-only decisions and unchanged accepted evidence",
  );

  for (const mode of ["invalid_http_json", "invalid_provider_shape"]) {
    mock.mode = mode;
    const stage = await create(p);
    const calls = mock.calls.length;
    await client.aesthetic.control(p.id, stage.id, "start");
    await waitState(p, stage.id, "needs_attention");
    const batch = (await client.aesthetic.batches(p.id, stage.id)).items[0];
    const attempt = (
      await client.aesthetic.attempts(p.id, stage.id, batch.sequence)
    ).items[0];
    assert.equal(attempt.failure.provider_request_id, "r1-provider-request");
    assert.equal(attempt.state, "outcome_unknown");
    assert.equal(mock.calls.length, calls + 1);
  }
  mock.mode = "valid";
  checks.push(
    "invalid HTTP JSON and provider shape retain request ID and unknown outcome without automatic network retry",
  );

  for (const point of ["create_project_committed", "create_ledger_committed"]) {
    const value = request(p);
    const calls = mock.calls.length;
    await writeFile(hook(point, value.idempotency_key), "crash");
    await assert.rejects(() => client.aesthetic.create(p.id, value));
    await waitHit(point, value.idempotency_key);
    await unlink(hook(point, value.idempotency_key));
    await restart(true);
    const recovered = await client.aesthetic.stage(p.id, value.idempotency_key);
    assert.equal(recovered.state, "paused");
    assert.equal(recovered.frozen, 0);
    readDb(p, "project.sqlite", (db) => {
      assert.equal(
        db
          .prepare("SELECT collection_id FROM evaluation_stage_refs WHERE id=?")
          .get(recovered.id).collection_id,
        p.collection.id,
      );
      assert.equal(
        db
          .prepare(
            "SELECT count(*) AS n FROM evaluation_source_refs WHERE stage_id=? AND source_id=?",
          )
          .get(recovered.id, p.source.id).n,
        1,
      );
    });
    assert.equal(
      (await client.aesthetic.create(p.id, value)).config_hash,
      recovered.config_hash,
    );
    await assert.rejects(
      () => client.aesthetic.create(p.id, { ...value, name: "different" }),
      (e) => e.code === "IDEMPOTENCY_CONFLICT",
    );
    assert.equal(mock.calls.length, calls);
    await client.aesthetic.control(p.id, recovered.id, "start");
    await waitState(p, recovered.id, "ready");
    await client.aesthetic.control(p.id, recovered.id, "cancel");
    await assert.rejects(
      () => client.aesthetic.create(p.id, value),
      (e) => e.code === "OBJECT_REMOVED",
    );
    readDb(p, "evaluation.sqlite", (db) =>
      assert.equal(
        db
          .prepare("SELECT count(*) AS n FROM stages WHERE id=?")
          .get(recovered.id).n,
        1,
      ),
    );
  }
  checks.push(
    "process abort at each cross-database creation boundary: one stage, frozen config and protected references restored; cancelled key cannot revive",
  );

  const abandoned = request(p);
  await writeFile(
    hook("create_project_committed", abandoned.idempotency_key),
    "busy_once",
  );
  await assert.rejects(
    () => client.aesthetic.create(p.id, abandoned),
    (e) => e.code === "EVALUATION_BUSY",
  );
  await client.aesthetic.abandonCreation(p.id, abandoned.idempotency_key);
  await client.aesthetic.abandonCreation(p.id, abandoned.idempotency_key);
  await assert.rejects(
    () => client.aesthetic.create(p.id, abandoned),
    (e) => e.code === "OBJECT_REMOVED",
  );
  await restart();
  await assert.rejects(
    () => client.aesthetic.create(p.id, abandoned),
    (e) => e.code === "OBJECT_REMOVED",
  );
  checks.push(
    "explicit abandonment of an incomplete creation releases references and leaves a persistent idempotent tombstone",
  );

  for (const mode of ["busy", "full", "io"]) {
    const stage = await create(p);
    const second = await create(other);
    const calls = mock.calls.length;
    await writeFile(hook("receipt_before_commit", stage.id), mode);
    await client.aesthetic.control(p.id, stage.id, "start");
    await until(
      () => client.aesthetic.metrics(p.id),
      (v) =>
        v.retained_outcomes === 1 &&
        v.dispatch_health === "storage_backpressure",
    );
    if (mode === "busy") await complete(other, second);
    else
      await assert.rejects(
        () => client.aesthetic.control(other.id, second.id, "start"),
        (e) => e.code === "EVALUATION_STORAGE_UNHEALTHY",
      );
    const resumedAt = Date.now();
    await unlink(hook("receipt_before_commit", stage.id));
    const done = await waitState(p, stage.id, "completed");
    await healthy(p);
    assert.ok(Date.now() - resumedAt < 10000);
    assert.deepEqual(
      [done.accepted, done.attempts, done.input_tokens],
      [1, 1, 13],
    );
    if (mode !== "busy") await complete(other, second);
    assert.equal(mock.calls.length, calls + 2);
  }
  checks.push(
    "BUSY stays ledger-local; FULL closes the shared volume and IOERR closes shared admission; retained outcomes drain once and another project can resume",
  );

  const lostAck = await create(p);
  const ackCalls = mock.calls.length;
  await writeFile(hook("receipt_after_commit", lostAck.id), "lost_ack_once");
  const ackDone = await complete(p, lostAck);
  await healthy(p);
  assert.deepEqual(
    [
      ackDone.attempts,
      ackDone.accepted,
      ackDone.input_tokens,
      mock.calls.length - ackCalls,
    ],
    [1, 1, 13, 1],
  );
  checks.push(
    "lost acknowledgement after receipt COMMIT retries persistence only, without duplicate tokens, exposure or network calls",
  );

  for (const point of ["settle_before_commit", "project_sync_before_commit"]) {
    const stage = await create(p);
    const calls = mock.calls.length;
    // Hold the already-admitted request so the hook only targets finalization,
    // not the HTTP control command's initial project projection.
    await writeFile(hook("dispatch_after_upload", stage.id), "hold");
    await client.aesthetic.control(p.id, stage.id, "start");
    await waitHit("dispatch_after_upload", stage.id);
    await writeFile(hook(point, stage.id), "busy_once");
    await unlink(hook("dispatch_after_upload", stage.id));
    const done = await waitState(p, stage.id, "completed");
    await healthy(p);
    assert.deepEqual(
      [done.accepted, done.attempts, mock.calls.length - calls],
      [1, 1, 1],
    );
    readDb(p, "project.sqlite", (db) =>
      assert.equal(
        db
          .prepare("SELECT state FROM evaluation_stage_refs WHERE id=?")
          .get(stage.id).state,
        "completed",
      ),
    );
  }
  checks.push(
    "ledger settlement and project projection retry transient commit failure without leaving a finished stage stuck or reviving a terminal state",
  );

  const waiting = await create(p);
  const waitCalls = mock.calls.length;
  await writeFile(hook("dispatch_after_upload", waiting.id), "hold");
  await client.aesthetic.control(p.id, waiting.id, "start");
  await waitHit("dispatch_after_upload", waiting.id);
  await client.aesthetic.control(p.id, waiting.id, "pause");
  await unlink(hook("dispatch_after_upload", waiting.id));
  await waitState(p, waiting.id, "paused");
  assert.equal(mock.calls.length, waitCalls);
  assert.equal((await client.aesthetic.stage(p.id, waiting.id)).attempts, 0);
  await complete(p, waiting);
  checks.push(
    "pause after upload waiting but before durable send admission blocks the request and leaves a resumable batch",
  );

  const held = await create(other);
  const faulting = await create(p);
  const heldCalls = mock.calls.length;
  await writeFile(hook("dispatch_after_upload", held.id), "hold");
  await client.aesthetic.control(other.id, held.id, "start");
  await waitHit("dispatch_after_upload", held.id);
  await writeFile(hook("receipt_before_commit", faulting.id), "full");
  await client.aesthetic.control(p.id, faulting.id, "start");
  await until(
    () => client.aesthetic.metrics(p.id),
    (v) => v.retained_outcomes === 1,
  );
  await unlink(hook("dispatch_after_upload", held.id));
  await waitState(other, held.id, "needs_attention");
  assert.equal(mock.calls.length, heldCalls + 1);
  assert.equal((await client.aesthetic.stage(other.id, held.id)).attempts, 0);
  await unlink(hook("receipt_before_commit", faulting.id));
  await waitState(p, faulting.id, "completed");
  await healthy(p);
  await complete(other, held);
  checks.push(
    "volume fault closes final admission for another stage already past upload pacing; recovery resumes its original never-sent batch",
  );

  for (const point of [
    "receipt_after_commit",
    "parse_before_commit",
    "parse_after_commit",
  ]) {
    const stage = await create(p);
    const calls = mock.calls.length;
    await writeFile(hook(point, stage.id), "crash");
    await client.aesthetic.control(p.id, stage.id, "start");
    await waitHit(point, stage.id);
    await unlink(hook(point, stage.id));
    await restart(true);
    const done = await complete(p, stage);
    assert.deepEqual(
      [done.attempts, done.accepted, done.input_tokens],
      [1, 1, 13],
    );
    assert.equal(mock.calls.length, calls + 1);
  }
  checks.push(
    "crash after received COMMIT and before/after evidence COMMIT: startup plus local parse accepts exactly once",
  );

  const exited = await create(p);
  const exitCalls = mock.calls.length;
  await writeFile(hook("receipt_before_commit", exited.id), "exit");
  await client.aesthetic.control(p.id, exited.id, "start");
  await until(
    () => client.aesthetic.metrics(p.id),
    (v) =>
      v.dispatch_health === "manual_recovery_required" &&
      v.storage_error_code === "EVALUATION_WRITER_EXITED",
  );
  await complete(other, await create(other));
  await unlink(hook("receipt_before_commit", exited.id));
  await restart();
  const unknown = await client.aesthetic.stage(p.id, exited.id);
  assert.equal(unknown.unknown, 1);
  assert.equal(unknown.accepted, 0);
  assert.equal(mock.calls.length, exitCalls + 2);
  await sleep(250);
  assert.equal(mock.calls.length, exitCalls + 2);
  checks.push(
    "writer thread exit is distinct from queue pressure, other ledgers remain usable, shutdown finishes and uncommitted outcome becomes unknown without a new call",
  );

  // Seed a real immutable million-member workset while the fixture engine is stopped.
  const capacityRequest = request(other);
  const oldPreview = await client.aesthetic.preflight(
    other.id,
    capacityRequest,
  );
  client.dispose();
  await engine.stop();
  const capacityDb = new DatabaseSync(
    resolve(other.directory, "project.sqlite"),
  );
  const bigId = randomUUID();
  try {
    capacityDb.exec("PRAGMA journal_mode=WAL; BEGIN IMMEDIATE");
    capacityDb
      .prepare(
        "INSERT INTO collections(id,name,count) VALUES (?,'capacity fixture',1000000)",
      )
      .run(bigId);
    capacityDb
      .prepare(
        "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<1000000) INSERT INTO collection_members(collection_id,source_id,asset_id) SELECT ?,?,printf('%064x',x) FROM n",
      )
      .run(bigId, other.source.id);
    capacityDb.exec("COMMIT");
  } finally {
    capacityDb.close();
  }
  await start();
  const bigRequest = request(other, { collection_id: bigId });
  const admitted = await client.aesthetic.preflight(other.id, bigRequest);
  assert.equal(admitted.total, 1000000);
  assert.equal(admitted.admitted, true);
  client.dispose();
  await engine.stop();
  const changed = new DatabaseSync(resolve(other.directory, "project.sqlite"));
  try {
    changed.exec("BEGIN IMMEDIATE");
    changed
      .prepare(
        "INSERT INTO collection_members VALUES (?,?,printf('%064x',1000001))",
      )
      .run(bigId, other.source.id);
    changed
      .prepare("UPDATE collections SET count=1000001 WHERE id=?")
      .run(bigId);
    // Source location metadata participates in the smaller workset's preflight token.
    const row = changed
      .prepare("SELECT json FROM sources WHERE id=?")
      .get(other.source.id);
    const sourceValue = JSON.parse(row.json);
    sourceValue.name = "changed source descriptor";
    changed
      .prepare("UPDATE sources SET json=? WHERE id=?")
      .run(JSON.stringify(sourceValue), other.source.id);
    changed.exec("COMMIT");
  } finally {
    changed.close();
  }
  await start();
  const capacityCalls = mock.calls.length;
  const denied = await client.aesthetic.preflight(other.id, bigRequest);
  assert.equal(denied.total, 1000001);
  assert.equal(denied.rejection_code, "EVALUATION_CAPACITY_EXCEEDED");
  await assert.rejects(
    () => client.aesthetic.create(other.id, bigRequest),
    (e) => e.code === "EVALUATION_CAPACITY_EXCEEDED",
  );
  await assert.rejects(
    () =>
      client.aesthetic.create(other.id, {
        ...capacityRequest,
        expected_input_version: oldPreview.input_version,
      }),
    (e) => e.code === "EVALUATION_INPUT_CHANGED",
  );
  assert.equal(mock.calls.length, capacityCalls);
  readDb(other, "project.sqlite", (db) =>
    assert.equal(
      db
        .prepare("SELECT count(*) AS n FROM evaluation_stage_refs WHERE id=?")
        .get(bigRequest.idempotency_key).n,
      0,
    ),
  );
  checks.push(
    "real 1000000/1000001 SQLite membership admission, authoritative create rejection before ledger registration or paid calls, and stale preflight input rejection",
  );

  for (const p of projects)
    readDb(p, "evaluation.sqlite", (db) => {
      assert.equal(db.prepare("PRAGMA quick_check").get().quick_check, "ok");
      assert.equal(db.prepare("PRAGMA foreign_key_check").all().length, 0);
      assert.equal(
        db
          .prepare(
            "SELECT count(*) AS n FROM stages s WHERE s.comparable!=(SELECT count(*) FROM candidates c WHERE c.stage_id=s.id AND c.exposures>0 AND c.disposition!='excluded' AND c.rating IN ('g','s','q','e')) OR s.excluded!=(SELECT count(*) FROM candidates c WHERE c.stage_id=s.id AND c.disposition='excluded') OR s.unresolved!=(SELECT count(*) FROM candidates c WHERE c.stage_id=s.id AND c.disposition!='excluded' AND (c.blocked=1 OR c.disposition!='active' OR c.exposures<json_extract(s.config,'$.request.exposures')))",
          )
          .get().n,
        0,
      );
    });
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      {
        passed: true,
        checks,
        mock_calls: mock.calls.length,
        native_request_bytes: mock.calls.map((v) => v.bytes),
        limits: { max_stage_candidates: 1000000, recovery_deadline_ms: 10000 },
        scope:
          "R0/R1 real engine, SQLite and local HTTP; no commercial model, no million-image/replay throughput claim",
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
} finally {
  client?.dispose();
  await engine.stop(true);
  server.closeAllConnections();
  await new Promise((done) => server.close(done));
  // Leave the shared development executable as an ordinary build without test hooks.
  const log = await open(
    resolve(root, `.local/logs/aesthetic-recovery-restore-${stamp}.log`),
    "w",
  );
  try {
    await finished(
      cargo(["build", "-p", "studio-engine"], {
        cwd: root,
        windowsHide: true,
        stdio: ["ignore", log.fd, log.fd],
      }),
    );
  } finally {
    await log.close();
  }
}
