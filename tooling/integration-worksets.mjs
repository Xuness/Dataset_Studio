import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath, URLSearchParams } from "node:url";
import { EngineFixture, within } from "./engine-fixture.mjs";
import { pythonCommand } from "./platform.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local/test-runs",
  "integration-worksets-" + Date.now(),
);
await mkdir(run, { recursive: true });
const execute = promisify(execFile);
for (const [script, name, args] of [
  ["ui-fixture.py", "images", []],
  ["ranking-fixture.py", "ranking", ["256"]],
]) {
  await execute(
    pythonCommand(),
    [resolve(root, "tooling", script), resolve(run, name), ...args],
    { cwd: root, windowsHide: true },
  );
}
const fixtures = await Promise.all(
  ["images", "ranking"].map(async (name) =>
    JSON.parse(await readFile(resolve(run, name, "fixture.json"), "utf8")),
  ),
);
const engine = new EngineFixture(root, resolve(run, "state"));
const checks = [];
const identity = (key) => key.source_id + ":" + key.asset_id;
const hash = async (path) =>
  createHash("sha256")
    .update(await readFile(path))
    .digest("hex");
let project,
  base,
  source,
  rankSource,
  collection,
  ranked,
  artifact,
  latestRanked;
process.on("exit", () => engine.child?.kill());
try {
  await engine.start();
  project = await engine.api("/v1/projects", "POST", {
    name: "工作集成员编辑验收",
    parent_directory: resolve(run, "projects"),
  });
  within(run, project.directory);
  base = `/v1/projects/${project.id}`;
  [source, rankSource] = await Promise.all(
    fixtures.map((fixture, i) =>
      engine.api(base + "/sources", "POST", {
        name: i ? "排名资料" : "图片资料",
        kind: "danbooru",
        index_root: fixture.lake,
        media_root: fixture.lake,
      }),
    ),
  );
  const key = (number) => ({
    source_id: source.id,
    asset_id: fixtures[0].images.find((i) => i.number === number).sha,
  });
  let selection = await engine.api(base + "/selection", "PATCH", {
    expected_revision: 0,
    add: [key(1), key(2)],
    remove: [],
    clear: true,
  });
  collection = await engine.api(base + "/collections", "POST", {
    name: "可编辑工作集",
  });
  assert.equal(collection.revision, 0);
  const scoped = (c, revision) => ({
    project_id: project.id,
    target: {
      kind: "workset",
      collection_id: c.id,
      ...(revision === undefined ? {} : { revision }),
    },
  });
  const body = (c, kind, input) => ({
    request_id: crypto.randomUUID(),
    expected_revision: c.revision,
    change: { kind, input },
  });
  const edit = (c, request) =>
    engine.api(base + `/collections/${c.id}/members`, "POST", request);
  const points = (keys) => ({ kind: "keys", keys });
  const members = async (c) => {
    const out = [];
    let cursor = null;
    do {
      const params = new URLSearchParams({
        collection_id: c.id,
        limit: "2",
        order: "asset_key_asc",
        ...(cursor ? { cursor } : {}),
      });
      const page = await engine.api(base + "/assets?" + params);
      out.push(...page.items.map((a) => identity(a.key)));
      cursor = page.next_cursor;
      assert.ok(out.length < 1000);
    } while (cursor);
    return out.sort();
  };
  const request = body(collection, "add", points([key(2), key(3), key(3)]));
  const addition = await edit(collection, request);
  assert.deepEqual(
    [addition.requested, addition.changed, addition.collection.count],
    [2, 1, 3],
  );
  assert.deepEqual(await edit(collection, request), addition);
  collection = addition.collection;
  await engine.expectError(
    base + `/collections/${collection.id}/members`,
    "POST",
    { ...request, request_id: crypto.randomUUID() },
    "REVISION_CONFLICT",
  );
  await engine.expectError(
    base + `/collections/${collection.id}/members`,
    "POST",
    { ...request, expected_revision: collection.revision },
    "IDEMPOTENCY_CONFLICT",
  );
  assert.equal(
    (await engine.api(base + "/selection")).revision,
    selection.revision,
  );
  checks.push(
    "HTTP edits deduplicate, replay idempotently and preserve the project selection",
  );
  const oldPage = await engine.api(
    base + `/assets?collection_id=${collection.id}&limit=1`,
  );
  const frozenJob = await engine.api(base + "/jobs", "POST", {
    idempotency_key: crypto.randomUUID(),
    scope: scoped(collection),
    delay_ms: 200,
  });
  const removed = await edit(
    collection,
    body(collection, "remove", points([key(1), key(80)])),
  );
  collection = removed.collection;
  assert.equal(removed.changed, 1);
  await engine.expectError(
    base +
      "/assets?" +
      new URLSearchParams({
        collection_id: collection.id,
        limit: "1",
        cursor: oldPage.next_cursor,
      }),
    "GET",
    undefined,
    "SOURCE_CHANGED",
  );
  assert.deepEqual(
    await members(collection),
    [identity(key(2)), identity(key(3))].sort(),
  );
  const jobs = await engine.wait(base + "/jobs", (value) =>
    value.items.some(
      (j) =>
        j.id === frozenJob.id && ["succeeded", "failed"].includes(j.status),
    ),
  );
  const fixed = jobs.items.find((j) => j.id === frozenJob.id);
  assert.equal(fixed.status, "succeeded");
  assert.equal(fixed.total, 3);
  assert.equal(fixed.input_scope.target.revision, 1);
  const restored = await edit(collection, {
    request_id: crypto.randomUUID(),
    expected_revision: collection.revision,
    change: { kind: "restore", revision: removed.previous_revision },
  });
  collection = restored.collection;
  assert.equal(collection.count, 3);
  const empty = await edit(
    collection,
    body(collection, "remove", { kind: "scope", scope: scoped(collection) }),
  );
  collection = empty.collection;
  assert.equal(collection.count, 0);
  assert.deepEqual(await members(collection), []);
  collection = (
    await edit(collection, body(collection, "add", points([key(1), key(2)])))
  ).collection;
  checks.push(
    "removal invalidates cursors, keeps submitted jobs fixed, supports undo and retains empty worksets",
  );
  const foreign = {
    project_id: crypto.randomUUID(),
    target: { kind: "selection", revision: 0 },
  };
  await engine.expectError(
    base + `/collections/${collection.id}/members`,
    "POST",
    body(collection, "add", { kind: "scope", scope: foreign }),
    "SCOPE_PROJECT_MISMATCH",
  );

  const queuedInput = await engine.api(base + "/query-results", "POST", {
    spec: {
      version: 3,
      source_ids: [source.id],
      conditions: [
        {
          field: "asset.id",
          operator: "eq",
          value: { type: "text", value: key(5).asset_id },
        },
      ],
      observation_rule: "current_post",
      order: "asset_key_asc",
    },
  });
  const savedInput = await engine.wait(
    base + `/query-results/${queuedInput.id}`,
    (r) => ["ready", "failed"].includes(r.state),
  );
  assert.equal(savedInput.state, "ready", JSON.stringify(savedInput));
  const captureRequest = body(collection, "add", {
    kind: "scope",
    scope: {
      project_id: project.id,
      target: { kind: "query_result", result_id: savedInput.id },
    },
  });
  const captured = await edit(collection, captureRequest);
  assert.equal(captured.changed, 1);
  collection = captured.collection;
  await engine.api(base + `/query-results/${savedInput.id}/release`, "POST");
  assert.deepEqual(await edit(collection, captureRequest), captured);
  collection = (
    await edit(collection, body(collection, "remove", points([key(5)])))
  ).collection;
  checks.push(
    "a committed receipt can replay after its original query input is released",
  );

  const job = await engine.api(base + "/tools/jobs", "POST", {
    idempotency_key: crypto.randomUUID(),
    scope: {
      project_id: project.id,
      target: {
        kind: "source",
        source_id: rankSource.id,
        revision: rankSource.revision,
      },
    },
    run: {
      operator_id: "danbooru.metarecall",
      operator_version: 1,
      parameters_version: 1,
      parameters: {
        mode: "rank",
        ratings: ["g", "s", "q", "e"],
        cohort_minimum: 8,
        seed: "workset-members",
      },
    },
  });
  const rankingJobs = await engine.wait(
    base + "/jobs",
    (value) =>
      value.items.some(
        (j) => j.id === job.id && ["succeeded", "failed"].includes(j.status),
      ),
    90000,
  );
  assert.equal(
    rankingJobs.items.find((j) => j.id === job.id).status,
    "succeeded",
    JSON.stringify(rankingJobs),
  );
  artifact = await engine.api(base + `/jobs/${job.id}/ranking`);
  const files = artifact.files.map((f) =>
    within(run, resolve(project.directory, f.path)),
  );
  const hashes = await Promise.all(files.map(hash));
  ranked = await engine.api(
    base + `/artifacts/${artifact.id}/ranking/worksets`,
    "POST",
    {
      idempotency_key: crypto.randomUUID(),
      name: "排名工作集",
      filter: { top: 3, order: "main", eligibility: "eligible" },
    },
  );
  const originalCount = ranked.count;
  const rankedPage = async (scope, options = {}) =>
    engine.api(base + "/ranking-browse/assets", "POST", {
      scope,
      order: "main",
      descending: false,
      limit: 3,
      ...options,
    });
  const original = await rankedPage(scoped(ranked));
  const more = await engine.api(
    base + `/artifacts/${artifact.id}/ranking/rows`,
    "POST",
    {
      filter: { rating: "g", order: "main", eligibility: "eligible" },
      limit: 32,
    },
  );
  const existing = more.items.find((row) => row.scores.main_rank > 3);
  assert.ok(existing);
  const existingKey = {
    source_id: existing.input.source_id,
    asset_id: existing.input.asset_id,
  };
  const added = await edit(
    ranked,
    body(ranked, "add", points([existingKey, key(1)])),
  );
  latestRanked = added.collection;
  const deletion = await edit(
    latestRanked,
    body(latestRanked, "remove", points([original.items[0].key])),
  );
  latestRanked = deletion.collection;
  assert.equal(latestRanked.count, originalCount + 1);
  await engine.expectError(
    base + "/ranking-browse/assets",
    "POST",
    {
      scope: scoped(ranked),
      order: "main",
      descending: false,
      limit: 3,
      cursor: original.next_cursor,
    },
    "INVALID_INPUT",
  );
  for (const descending of [false, true]) {
    let cursor = null;
    const seen = [];
    do {
      const page = await rankedPage(scoped(ranked), {
        descending,
        ...(cursor ? { cursor } : {}),
      });
      assert.equal(page.preparing ?? null, null);
      seen.push(...page.items);
      cursor = page.next_cursor;
      assert.ok(seen.length <= latestRanked.count);
    } while (cursor);
    assert.equal(seen.length, latestRanked.count);
    assert.equal(identity(seen.at(-1).key), identity(key(1)));
    assert.equal(seen.at(-1).ranking ?? null, null);
    assert.ok(
      !seen.some((a) => identity(a.key) === identity(original.items[0].key)),
    );
    assert.equal(
      seen.find((a) => identity(a.key) === identity(existingKey)).ranking
        .main_rank,
      existing.scores.main_rank,
    );
    const last = await rankedPage(scoped(ranked), {
      descending,
      start_rank: String(latestRanked.count),
    });
    assert.equal(identity(last.items[0].key), identity(key(1)));
  }
  const old = await rankedPage(scoped(ranked, 0), { limit: 128 });
  assert.equal(old.items.length, originalCount);
  checks.push(
    "ranked edits merge original scores with appended unranked images in both directions and all anchors",
  );
  const query = async (conditions) => {
    const queued = await engine.api(base + "/query-results", "POST", {
      spec: {
        version: 3,
        source_ids: [source.id, rankSource.id].sort(),
        conditions,
        observation_rule: "current_post",
        order: "asset_key_asc",
        input_scope: scoped(ranked),
      },
    });
    const ready = await engine.wait(base + `/query-results/${queued.id}`, (r) =>
      ["ready", "failed", "interrupted"].includes(r.state),
    );
    assert.equal(ready.state, "ready", JSON.stringify(ready));
    return ready;
  };
  const ratings = [
    {
      field: `project.${artifact.id}.rating`,
      operator: "eq",
      value: { type: "text", value: "s" },
    },
  ];
  const rating = await query(ratings);
  const tag = await query([
    ...ratings,
    {
      field: "tags",
      operator: "has_all_tags",
      value: { type: "text_list", value: ["common"] },
    },
  ]);
  for (const result of [rating, tag]) {
    const page = await rankedPage(
      {
        project_id: project.id,
        target: { kind: "query_result", result_id: result.id },
      },
      { limit: 128 },
    );
    assert.ok(
      page.items.some((a) => identity(a.key) === identity(key(1))),
      "new unranked member must satisfy its frozen Rating and live tags",
    );
  }
  latestRanked = (
    await edit(latestRanked, body(latestRanked, "remove", points([key(1)])))
  ).collection;
  const historical = await rankedPage(
    {
      project_id: project.id,
      target: { kind: "query_result", result_id: tag.id },
    },
    { limit: 128 },
  );
  assert.ok(historical.items.some((a) => identity(a.key) === identity(key(1))));
  const validity = await engine.api(base + `/query-results/${tag.id}/validity`);
  assert.equal(validity.newer_available, true);
  assert.deepEqual(await Promise.all(files.map(hash)), hashes);
  checks.push(
    "Rating and tag refinements include appended members and keep their historical version after later edits",
  );
  const demo = await engine.api(base + "/sources", "POST", {
    name: "内置参考图",
    kind: "demo",
    index_root: null,
    media_root: null,
  });
  const demoKey = (
    await engine.api(base + `/assets?source_id=${demo.id}&limit=1`)
  ).items[0].key;
  assert.ok(demoKey.asset_id.startsWith("sample-"));
  latestRanked = (
    await edit(latestRanked, body(latestRanked, "add", points([demoKey])))
  ).collection;
  assert.ok((await members(ranked)).includes(identity(demoKey)));
  const demoRange = await rankedPage(scoped(ranked), { limit: 128 });
  assert.equal(identity(demoRange.items.at(-1).key), identity(demoKey));
  const demoResult = await engine.api(base + "/query-results", "POST", {
    spec: {
      version: 3,
      source_ids: [rankSource.id, demo.id].sort(),
      conditions: [],
      observation_rule: "current_post",
      order: "asset_key_asc",
      input_scope: scoped(ranked),
    },
  });
  assert.equal(demoResult.state, "ready");
  const fixedDemo = await engine.api(
    base + `/query-results/${demoResult.id}/assets?limit=128`,
  );
  assert.ok(
    fixedDemo.page.items.some((a) => identity(a.key) === identity(demoKey)),
  );
  latestRanked = (
    await edit(latestRanked, body(latestRanked, "remove", points([demoKey])))
  ).collection;
  assert.ok(!(await members(ranked)).includes(identity(demoKey)));
  assert.deepEqual(await Promise.all(files.map(hash)), hashes);
  checks.push(
    "opaque built-in image identities work in ranked and fixed-query member readers",
  );
  await engine.stop();
  await engine.start();
  await engine.api("/v1/projects/open", "POST", {
    directory: project.directory,
  });
  assert.equal(
    (await engine.api(base + "/collections")).items.find(
      (c) => c.id === ranked.id,
    ).revision,
    latestRanked.revision,
  );
  assert.deepEqual(
    await members(collection),
    [identity(key(1)), identity(key(2))].sort(),
  );
  checks.push(
    "member revisions survive engine restart without rewriting immutable ranking materials",
  );
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      {
        passed: true,
        checks,
        project,
        source,
        collection,
        ranked: latestRanked,
        artifact_id: artifact.id,
        ui_key: key(4),
      },
      null,
      2,
    ),
  );
  console.log(JSON.stringify({ passed: true, checks: checks.length, run }));
} catch (error) {
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify({ passed: false, checks, error: String(error) }, null, 2),
  );
  throw error;
} finally {
  await engine.stop();
}
