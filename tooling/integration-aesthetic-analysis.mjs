import { pythonCommand } from "./platform.mjs";
// Synthetic images and localhost provider only. All ranking, experiment, review,
// and workset operations after evidence collection use the production offline API.
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { createHash, randomUUID } from "node:crypto";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve, toNamespacedPath } from "node:path";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { EngineFixture, sleep } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";
import { network } from "./llm-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local/test-runs/integration-aesthetic-analysis-" + Date.now(),
);
await mkdir(run, { recursive: true });
await promisify(execFile)(
  pythonCommand(),
  [resolve(root, "tooling/ranking-fixture.py"), resolve(run, "fixture"), "128"],
  { cwd: root, windowsHide: true },
);
const fixture = JSON.parse(
  await readFile(resolve(run, "fixture/fixture.json"), "utf8"),
);
const engine = new EngineFixture(root, resolve(run, "state"));
const { StudioClient } = await clientFixture(root, resolve(run, "sdk"));
let calls = 0;
const server = createServer(async (req, res) => {
  try {
    const chunks = [];
    for await (const part of req) chunks.push(part);
    const body = JSON.parse(Buffer.concat(chunks));
    const blocks = body.messages
      .filter((m) => m.role === "user")
      .flatMap((m) => m.content);
    const labels = blocks
      .filter((b) => /^img\d{2}$/.test(b.text))
      .map((b) => b.text);
    const images = blocks
      .filter((b) => b.image_url)
      .map((b) => b.image_url.url);
    assert.equal(labels.length, images.length);
    const sorted = labels
      .map((id, i) => ({
        id,
        order: createHash("sha256").update(images[i]).digest("hex"),
      }))
      .sort((a, b) => a.order.localeCompare(b.order));
    const tiers = [];
    for (let i = 0; i < sorted.length; i += 3)
      tiers.push(sorted.slice(i, i + 3).map((v) => v.id));
    calls++;
    const text = JSON.stringify({
      schema_version: 1,
      tiers,
      elite_candidates: [sorted[0].id],
      unjudgeable: [],
    });
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(
      JSON.stringify({
        id: "mock",
        model: "fixture",
        choices: [
          { index: 0, finish_reason: "stop", message: { content: text } },
        ],
        usage: { prompt_tokens: 20, completion_tokens: 10, total_tokens: 30 },
      }),
    );
  } catch (e) {
    res.writeHead(500);
    res.end(String(e));
  }
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
let client, project;
const checks = [];
let snapshot;
const estimator = (kind = "davidson_v1") => ({
  kind,
  iterations: 128,
  regularization: 0.05,
  tie_strength: 1,
});
const analysisPath = () => `/v1/projects/${project.id}/aesthetic/analysis`;
async function waitJob(id) {
  const result = await engine.wait(
    `${analysisPath()}/jobs/${id}`,
    (v) =>
      ["completed", "failed", "cancelled", "interrupted"].includes(v.state),
    90000,
  );
  assert.equal(result.state, "completed", JSON.stringify(result));
  return result;
}
async function allRows(id) {
  const result = [];
  let after;
  do {
    const page = await client.aesthetic.analysis.rows(project.id, id, {
      after,
      limit: 7,
    });
    result.push(...page.items);
    after = page.next_cursor;
  } while (after);
  return result;
}
try {
  await engine.start();
  client = new StudioClient(engine.connection);
  project = await client.createProject({
    name: "离线美学后端验收",
    parent_directory: null,
  });
  const source = await engine.api(
    `/v1/projects/${project.id}/sources`,
    "POST",
    {
      kind: "danbooru",
      name: "合成图",
      index_root: fixture.lake,
      media_root: fixture.lake,
    },
  );
  const keys = fixture.objects
    .filter((v) => v.number >= 32 && v.number < 96)
    .map((v) => ({ source_id: source.id, asset_id: v.sha }));
  const selection = await client.selection(project.id);
  await client.changeSelection(project.id, {
    expected_revision: selection.revision,
    add: keys,
    remove: [],
    clear: true,
  });
  const collection = await client.createCollection(project.id, "校准样本");
  const prompt = await client.llm.systemPrompts.save({
    expected_revision: 0,
    config: {
      name: "合成标准",
      description: "local mock only",
      text: "合成评审 fixture",
    },
  });
  const provider = await client.llm.providers.save({
    expected_revision: 0,
    config: {
      name: "local",
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
  const stageRequest = {
    idempotency_key: randomUUID(),
    name: "已知合成顺序",
    collection_id: collection.id,
    model_id: model.id,
    system_prompt_id: prompt.id,
    overrides: {},
    exposures: 4,
    max_calls: 30,
    concurrency: 2,
  };
  const stage = await client.aesthetic.create(project.id, stageRequest);
  await engine.wait(
    `/v1/projects/${project.id}/aesthetic/stages/${stage.id}`,
    (v) => v.state === "ready",
  );
  await client.aesthetic.control(project.id, stage.id, "start");
  const completed = await engine.wait(
    `/v1/projects/${project.id}/aesthetic/stages/${stage.id}`,
    (v) => ["completed", "needs_attention"].includes(v.state),
    90000,
  );
  assert.equal(completed.state, "completed", completed.error);
  assert.equal(calls, 16);
  checks.push(
    "real ledger and business parser collect sixteen local mock observations",
  );

  const fit = {
    stage_id: stage.id,
    estimator: estimator(),
    stability_seed: 17,
  };
  const request = {
    idempotency_key: randomUUID(),
    name: "离线快照",
    spec: { kind: "fit", config: fit, experiment_id: null, variant: null },
  };
  snapshot = await client.aesthetic.analysis.create(project.id, request);
  snapshot = await waitJob(snapshot.id);
  assert.equal(snapshot.input.observations, 16);
  assert.equal(snapshot.result.kind, "fit");
  assert.ok(
    snapshot.result.groups.every(
      (g) => g.fully_connected && g.candidates === 16,
    ),
  );
  const rows = await allRows(snapshot.id);
  assert.equal(rows.length, 64);
  await assert.rejects(
    () => client.aesthetic.analysis.candidate(project.id, snapshot.id, 9999999),
    (error) => error.code === "NOT_FOUND",
  );
  await assert.rejects(
    () =>
      client.aesthetic.analysis.candidate(project.id, snapshot.id, 10000000),
    (error) => error.code === "INVALID_INPUT",
  );
  const explicitRating = await client.aesthetic.analysis.rows(
    project.id,
    snapshot.id,
    { rating: "e", limit: 5 },
  );
  assert.equal(explicitRating.items.length, 5);
  assert.ok(explicitRating.items.every((row) => row.rating === "e"));
  const mixedRatings = ["g", "e"];
  const mixed = [];
  for (let after; ; ) {
    const page = await client.aesthetic.analysis.rows(project.id, snapshot.id, {
      ...(after ? { after } : {}),
      rating: mixedRatings.join(","),
      limit: 5,
    });
    mixed.push(...page.items);
    if (!page.next_cursor) break;
    after = page.next_cursor;
  }
  assert.deepEqual(
    mixed.map((r) => r.position),
    rows
      .filter((r) => mixedRatings.includes(r.rating))
      .map((r) => r.position)
      .sort((a, b) => a - b),
  );
  await assert.rejects(
    () =>
      client.aesthetic.analysis.rows(project.id, snapshot.id, {
        rating: "g,x",
      }),
    (error) => error.code === "INVALID_INPUT",
  );
  assert.equal(new Set(rows.map((r) => r.position)).size, 64);
  assert.ok(rows.every((r) => r.exposures === 4 && r.rating_rank_min !== null));
  assert.equal(
    (await client.aesthetic.analysis.create(project.id, request)).id,
    snapshot.id,
  );
  await engine.expectError(
    `${analysisPath()}/jobs`,
    "POST",
    { ...request, name: "conflict" },
    "IDEMPOTENCY_CONFLICT",
  );
  checks.push(
    "watermarked offline snapshot with Rating ranks, ties, stable single and multi-Rating pagination and idempotency",
  );

  const experiment = await client.aesthetic.analysis.createExperiment(
    project.id,
    {
      idempotency_key: randomUUID(),
      name: "估计器对照",
      description: "same frozen ledger, no remote call",
      variants: [
        { label: "Davidson", fit },
        { label: "Borda", fit: { ...fit, estimator: estimator("borda_v1") } },
        {
          label: "Davidson v2",
          fit: { ...fit, estimator: estimator("davidson_v2") },
        },
      ],
    },
  );
  assert.equal(
    new Set(experiment.inputs.map((v) => v.evidence_watermark)).size,
    1,
  );
  const runs = await client.aesthetic.analysis.runExperiment(
    project.id,
    experiment.id,
  );
  for (const job of runs.items) await waitJob(job.id);
  assert.deepEqual(
    (
      await client.aesthetic.analysis.runExperiment(project.id, experiment.id)
    ).items.map((v) => v.id),
    runs.items.map((v) => v.id),
  );
  const compare = await client.aesthetic.analysis.create(project.id, {
    idempotency_key: randomUUID(),
    name: "实验对照",
    spec: { kind: "compare", left: runs.items[0].id, right: runs.items[1].id },
  });
  const compared = await waitJob(compare.id);
  assert.ok(
    compared.result.groups.every(
      (g) => g.comparable && g.rank_correlation > 0.99,
    ),
  );
  assert.equal(
    (
      await client.aesthetic.analysis.comparison(project.id, compare.id, {
        limit: 128,
      })
    ).items.length,
    64,
  );
  checks.push(
    "immutable experiment variants share evidence and compare rankings without mixing model scores",
  );

  const filter = {
    top_percent: 25,
    include_protected: false,
    protected_only: false,
    needs_review: false,
    ratings: [],
  };
  const expected = rows.filter((r) => r.rating_rank_min <= 4);
  assert.ok(expected.length > 16, "score ties at cutoff must be included");
  const derive = await client.aesthetic.analysis.create(project.id, {
    idempotency_key: randomUUID(),
    name: "Top25含边界并列",
    spec: { kind: "derive", snapshot_id: snapshot.id, filter },
  });
  const derived = await waitJob(derive.id);
  assert.equal(derived.result.count, expected.length);
  assert.ok(
    (await client.collections(project.id)).items.some(
      (c) =>
        c.id === derived.result.collection_id && c.count === expected.length,
    ),
  );
  const child = await client.aesthetic.create(project.id, {
    ...stageRequest,
    idempotency_key: randomUUID(),
    name: "精筛候选冻结",
    collection_id: derived.result.collection_id,
    exposures: 2,
  });
  const readyChild = await engine.wait(
    `/v1/projects/${project.id}/aesthetic/stages/${child.id}`,
    (v) => v.state === "ready",
  );
  assert.equal(readyChild.total, expected.length);
  assert.equal(readyChild.attempts, 0);
  checks.push(
    "Top percentage includes score ties and publishes ordinary workset usable by next-stage evaluation",
  );

  const low = rows.find((r) => r.rating_rank_min > 12 && !r.protected);
  const review = {
    idempotency_key: randomUUID(),
    snapshot_id: snapshot.id,
    ordinal: low.ordinal,
    decision: "protect",
    reviewer: "校准参考",
    reason: "保留进入下一阶段",
  };
  const saved = await client.aesthetic.analysis.review(project.id, review);
  const candidateHistory = await client.aesthetic.analysis.candidateReviews(
    project.id,
    snapshot.id,
    low.ordinal,
  );
  assert.equal(candidateHistory.items[0].sequence, saved.sequence);
  assert.ok(
    candidateHistory.items.every(
      (item) => item.request.ordinal === low.ordinal,
    ),
  );
  assert.equal(
    (
      await client.aesthetic.analysis.candidateReviews(
        project.id,
        snapshot.id,
        rows.find((r) => r.ordinal !== low.ordinal).ordinal,
      )
    ).items.length,
    0,
  );
  await assert.rejects(
    () =>
      client.aesthetic.analysis.candidateReviews(
        project.id,
        snapshot.id,
        1000000,
      ),
    (error) => error.code === "NOT_FOUND",
  );
  assert.equal(
    (await client.aesthetic.analysis.review(project.id, review)).sequence,
    saved.sequence,
  );
  const unchanged = await client.aesthetic.analysis.candidate(
    project.id,
    snapshot.id,
    low.ordinal,
  );
  assert.equal(unchanged.score, low.score);
  assert.equal(unchanged.protected, false);
  const protectedWorkset = await client.aesthetic.analysis.create(project.id, {
    idempotency_key: randomUUID(),
    name: "Top25与保护池",
    spec: {
      kind: "derive",
      snapshot_id: snapshot.id,
      filter: { ...filter, include_protected: true },
    },
  });
  const protectedResult = await waitJob(protectedWorkset.id);
  assert.equal(protectedResult.result.count, expected.length + 1);
  assert.equal(
    (await client.aesthetic.analysis.reviews(project.id, snapshot.id)).items
      .length,
    1,
  );
  checks.push(
    "append-only human review affects protection selection without modifying published scores",
  );

  const poolFilter = { ratings: [], protected_only: true };
  const preview = await client.aesthetic.analysis.select(
    project.id,
    snapshot.id,
    { filter: poolFilter, limit: 1 },
  );
  assert.equal(preview.items.length, 1);
  assert.ok(preview.next_cursor);
  await client.aesthetic.analysis.review(project.id, {
    ...review,
    idempotency_key: randomUUID(),
    decision: "release",
    reason: "验证复核水位冻结",
  });
  const pool = [...preview.items];
  let cursor = preview.next_cursor;
  while (cursor) {
    const page = await client.aesthetic.analysis.select(
      project.id,
      snapshot.id,
      { filter: poolFilter, after: cursor, limit: 2 },
    );
    assert.equal(page.review_watermark, preview.review_watermark);
    pool.push(...page.items);
    cursor = page.next_cursor;
  }
  assert.ok(
    pool.some(
      (r) => r.ranking.ordinal === low.ordinal && r.effective_protected,
    ),
  );
  const pinned = await client.aesthetic.analysis.create(project.id, {
    idempotency_key: randomUUID(),
    name: "固定预览复核水位",
    spec: {
      kind: "derive",
      snapshot_id: snapshot.id,
      filter: poolFilter,
      review_watermark: preview.review_watermark,
    },
  });
  assert.equal((await waitJob(pinned.id)).result.count, pool.length);
  await engine.expectError(
    `${analysisPath()}/snapshots/${snapshot.id}/select`,
    "POST",
    { filter: { ...poolFilter, ratings: ["g"] }, after: preview.next_cursor },
    "INVALID_INPUT",
  );
  checks.push(
    "bounded selection pagination pins review watermark and derived members match the reviewed preview",
  );

  const slow = await client.aesthetic.analysis.create(project.id, {
    idempotency_key: randomUUID(),
    name: "取消重放",
    spec: {
      kind: "fit",
      config: { ...fit, estimator: { ...estimator(), regularization: 0.001 } },
      experiment_id: null,
      variant: null,
    },
  });
  await client.aesthetic.analysis.control(project.id, slow.id, "cancel");
  await engine.wait(
    `${analysisPath()}/jobs/${slow.id}`,
    (v) => v.state === "cancelled",
  );
  await sleep(100);
  await client.aesthetic.analysis.control(project.id, slow.id, "resume");
  await waitJob(slow.id);
  assert.equal(calls, 16);
  checks.push(
    "offline cancellation and explicit restart reuse evidence with no provider calls",
  );

  const crash = await client.aesthetic.analysis.create(project.id, {
    idempotency_key: randomUUID(),
    name: "中断恢复",
    spec: {
      kind: "fit",
      config: { ...fit, estimator: { ...estimator(), regularization: 0.001 } },
      experiment_id: null,
      variant: null,
    },
  });
  await engine.stop(true);
  await engine.start();
  client = new StudioClient(engine.connection);
  await engine.api(`/v1/projects/${project.id}/open`, "POST");
  const recovered = await client.aesthetic.analysis.job(project.id, crash.id);
  assert.ok(
    ["interrupted", "completed"].includes(recovered.state),
    JSON.stringify(recovered),
  );
  if (recovered.state === "interrupted") {
    await client.aesthetic.analysis.control(project.id, crash.id, "resume");
    await waitJob(crash.id);
  }
  assert.deepEqual(await allRows(snapshot.id), rows);
  assert.equal(calls, 16);
  checks.push(
    "engine restart preserves published snapshots and recovers pending offline jobs without remote replay",
  );

  const db = new DatabaseSync(resolve(project.directory, "project.sqlite"), {
    readOnly: true,
  });
  assert.equal(db.prepare("PRAGMA user_version").get().user_version, 15);
  const provenance = JSON.parse(
    db
      .prepare(
        "SELECT provenance_json FROM collection_scopes WHERE collection_id=?",
      )
      .get(derived.result.collection_id).provenance_json,
  );
  assert.equal(provenance.selection.snapshot_id, snapshot.id);
  assert.equal(
    provenance.frozen_input.evidence_watermark,
    snapshot.input.evidence_watermark,
  );
  db.close();
  const backup = await client.aesthetic.backup(project.id);
  const ledger = new DatabaseSync(
    toNamespacedPath(resolve(project.directory, backup.relative_path)),
    { readOnly: true },
  );
  assert.equal(ledger.prepare("PRAGMA user_version").get().user_version, 13);
  assert.equal(ledger.prepare("PRAGMA quick_check").get().quick_check, "ok");
  assert.equal(ledger.prepare("SELECT COUNT(*) n FROM reviews").get().n, 2);
  ledger.close();
  checks.push(
    "project lineage, independent ledger schema and backups retain experiment and review evidence",
  );
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      {
        passed: true,
        checks,
        providerCalls: calls,
        realUserImages: 0,
        commercialApiCalls: 0,
        snapshot: snapshot.id,
        rankingRows: rows.length,
      },
      null,
      2,
    ),
  );
  console.log(`Aesthetic offline integration passed: ${checks.length} checks.`);
} catch (error) {
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify({ passed: false, checks, error: String(error) }, null, 2),
  );
  throw error;
} finally {
  await engine.stop();
  server.closeAllConnections();
  await new Promise((done) => server.close(done));
}
