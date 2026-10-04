import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { createHash } from "node:crypto";
import { performance } from "node:perf_hooks";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { EngineFixture, within, sleep } from "./engine-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local/test-runs",
  "integration-ranking-browse-" + Date.now(),
);
await mkdir(run, { recursive: true });
const execute = promisify(execFile);
await execute(
  "python",
  [
    resolve(root, "tooling/ranking-fixture.py"),
    resolve(run, "fixture"),
    "32768",
    "--sparse-browse",
  ],
  { cwd: root, windowsHide: true },
);
const fixture = JSON.parse(
  await readFile(resolve(run, "fixture/fixture.json"), "utf8"),
);
const engine = new EngineFixture(root, resolve(run, "state"));
const checks = [];
const samples = [];
const hash = async (path) =>
  createHash("sha256")
    .update(await readFile(path))
    .digest("hex");
const sourceFiles = ["catalog.sqlite", "analysis.duckdb"].map((name) =>
  resolve(fixture.lake, "indexes/gen-ranking", name),
);
const sourceHashes = await Promise.all(sourceFiles.map(hash));
let database,
  project,
  base,
  artifact,
  full,
  mixed,
  small,
  source,
  sparseResult,
  sparseExpected,
  gapResult,
  gapExpected;
const scope = (collection) => ({
  project_id: project.id,
  target: { kind: "workset", collection_id: collection.id },
});
const identity = (asset) => asset.key.source_id + ":" + asset.key.asset_id;
const oracleIdentity = (row) => row.source_id + ":" + row.asset_id;
function oracle(collection, order = "main", descending = false, resultId) {
  const position =
    order === "input"
      ? "s.ordinal"
      : `coalesce(s.${order}_rank,9223372036854775807)`;
  const direction = descending ? "DESC" : "ASC";
  const unranked =
    order === "input" ? "" : `(${position}=9223372036854775807),`;
  const membership = resultId
    ? "EXISTS(SELECT 1 FROM project_state.result_members m WHERE m.result_id=? AND m.source_id=i.source_id AND m.asset_id=lower(hex(i.asset_id)))"
    : collection.id === small.id
      ? "s.eligibility='eligible' AND s.main_rank IS NOT NULL AND s.main_rank<=5"
      : collection.id === mixed?.id
        ? "1=1"
        : "s.eligibility='eligible'";
  return database
    .prepare(
      `SELECT s.ordinal,s.rating,s.main_rank,s.rescue_rank,s.main_score,s.rescue_score,i.post_id,i.source_id,lower(hex(i.asset_id)) AS asset_id FROM scores s JOIN fixed_input.input_rows i USING(ordinal) WHERE ${membership} ORDER BY ${unranked}coalesce(s.rating,'z') ${direction},${position} ${direction},s.ordinal ${direction}`,
    )
    .all(...(resultId ? [resultId] : []));
}
async function page(target, options = {}) {
  let cursor = options.cursor;
  let preparations = 0;
  for (;;) {
    const value = await engine.api(base + "/ranking-browse/assets", "POST", {
      scope: target,
      limit: 13,
      ...options,
      ...(cursor ? { cursor } : {}),
    });
    if (!value.preparing) return { ...value, preparations };
    assert.equal(value.items.length, 0);
    assert.ok(
      ++preparations < 4096,
      "bounded preparation cursor must make progress",
    );
    cursor = value.next_cursor ?? cursor;
    await sleep(40);
  }
}
function expectRows(actual, expected) {
  assert.deepEqual(actual.map(identity), expected.map(oracleIdentity));
  for (let index = 0; index < actual.length; index++) {
    const ranking = actual[index].ranking;
    assert.ok(ranking);
    for (const field of [
      "ordinal",
      "rating",
      "main_rank",
      "rescue_rank",
      "main_score",
      "rescue_score",
    ])
      assert.deepEqual(ranking[field], expected[index][field], field);
    assert.equal(
      ranking.post_id,
      expected[index].post_id == null ? null : String(expected[index].post_id),
    );
  }
}
try {
  await engine.start();
  project = await engine.api("/v1/projects", "POST", {
    name: "排名工作集浏览验证",
  });
  within(run, project.directory);
  base = "/v1/projects/" + project.id;
  source = await engine.api(base + "/sources", "POST", {
    name: "有界排名资料",
    kind: "danbooru",
    index_root: fixture.lake,
    media_root: fixture.lake,
  });
  const job = await engine.api(base + "/tools/jobs", "POST", {
    idempotency_key: crypto.randomUUID(),
    scope: {
      project_id: project.id,
      target: {
        kind: "source",
        source_id: source.id,
        revision: source.revision,
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
        seed: "browse-v091",
      },
    },
  });
  const jobs = await engine.wait(
    base + "/jobs",
    (value) =>
      value.items.some(
        (item) =>
          item.id === job.id &&
          ["succeeded", "failed", "cancelled"].includes(item.status),
      ),
    120000,
  );
  assert.equal(
    jobs.items.find((item) => item.id === job.id).status,
    "succeeded",
    JSON.stringify(jobs),
  );
  artifact = await engine.api(base + "/jobs/" + job.id + "/ranking");
  const artifactFiles = artifact.files.map((file) =>
    within(run, resolve(project.directory, file.path)),
  );
  const artifactHashes = await Promise.all(artifactFiles.map(hash));
  full = await engine.api(
    base + "/artifacts/" + artifact.id + "/ranking/worksets",
    "POST",
    {
      idempotency_key: crypto.randomUUID(),
      name: "全部合格排名",
      filter: { eligibility: "eligible", order: "main" },
    },
  );
  small = await engine.api(
    base + "/artifacts/" + artifact.id + "/ranking/worksets",
    "POST",
    {
      idempotency_key: crypto.randomUUID(),
      name: "各分级主排名前五",
      filter: { eligibility: "eligible", top: 5, order: "main" },
    },
  );
  assert.ok(full.count > 4096);
  assert.equal(small.count, 20);
  database = new DatabaseSync(
    resolve(project.directory, "artifacts", job.id + ".ranking.sqlite"),
    { readOnly: true },
  );
  database
    .prepare("ATTACH DATABASE ? AS fixed_input")
    .run(
      pathToFileURL(
        resolve(
          project.directory,
          "artifacts",
          job.id + ".ranking-input.sqlite",
        ),
      ).href + "?mode=ro",
    );
  database
    .prepare("ATTACH DATABASE ? AS project_state")
    .run(
      pathToFileURL(resolve(project.directory, "project.sqlite")).href +
        "?mode=ro",
    );
  database.exec("PRAGMA query_only=ON");
  const info = await engine.api(
    base + "/ranking-browse?collection_id=" + full.id,
  );
  assert.equal(info.ranking.artifact_id, artifact.id);
  assert.equal(info.ranking.count, full.count);
  assert.equal(info.ranking.saved_filter.order, "main");
  checks.push(
    "existing ranking provenance exposes saved order without changing the workset",
  );

  for (const order of [undefined, "main", "rescue", "input"]) {
    for (const descending of [false, true]) {
      const expected = oracle(full, order ?? "main", descending);
      const first = await page(scope(full), { order, descending });
      expectRows(first.items, expected.slice(0, 13));
      const second = await page(scope(full), {
        order,
        descending,
        cursor: first.next_cursor,
      });
      expectRows(second.items, expected.slice(13, 26));
      samples.push({
        order: order ?? "saved",
        descending,
        first: first.items[0].ranking.ordinal,
        preparations: first.preparations + second.preparations,
      });
    }
  }
  checks.push(
    "large worksets paginate in both directions for saved, main, rescue and input order",
  );

  mixed = await engine.api(
    base + "/artifacts/" + artifact.id + "/ranking/worksets",
    "POST",
    {
      idempotency_key: crypto.randomUUID(),
      name: "已排名与未排名",
      filter: { order: "main" },
    },
  );
  const derivedMixed = await engine.api(base + "/query-results", "POST", {
    spec: {
      version: 3,
      source_ids: [source.id],
      observation_rule: "current_post",
      order: "asset_key_asc",
      input_scope: scope(mixed),
      conditions: [
        {
          field: "stored.bytes",
          operator: "gte",
          value: { type: "integer", value: "1" },
        },
      ],
    },
  });
  const readyMixed = await engine.wait(
    base + "/query-results/" + derivedMixed.id,
    (r) => ["ready", "failed"].includes(r.state),
    60000,
  );
  assert.equal(readyMixed.state, "ready");
  const derivedScope = {
    project_id: project.id,
    target: { kind: "query_result", result_id: readyMixed.id },
  };
  for (const [target, resultId] of [
    [scope(mixed), undefined],
    [derivedScope, readyMixed.id],
  ]) {
    for (const order of ["main", "rescue"]) {
      for (const descending of [false, true]) {
        const expected = oracle(mixed, order, descending, resultId);
        const boundary = expected.findIndex(
          (row) => row[order + "_rank"] === null,
        );
        assert.ok(
          boundary > 13 && boundary < expected.length,
          "fixture must contain both ranked and unranked members",
        );
        assert.ok(
          expected
            .slice(boundary)
            .every((row) => row[order + "_rank"] === null),
        );
        expectRows(
          (await page(target, { order, descending })).items,
          expected.slice(0, 13),
        );
        const firstTail = await page(target, {
          order,
          descending,
          start_rank: String(boundary),
          limit: 1,
        });
        expectRows(firstTail.items, expected.slice(boundary - 1, boundary));
        assert.ok(firstTail.next_cursor);
        const tail = await page(target, {
          order,
          descending,
          start_rank: String(boundary),
          cursor: firstTail.next_cursor,
        });
        expectRows(tail.items, expected.slice(boundary, boundary + 13));
        const missing = expected.slice(boundary).find((row) => row.post_id > 0);
        if (missing) {
          const located = await page(target, {
            order,
            descending,
            start_post_id: String(missing.post_id),
          });
          assert.equal(located.items[0].ranking.ordinal, missing.ordinal);
        }
      }
    }
  }
  checks.push(
    "direct and materialized ranking scopes put all unranked images last in both directions, including cross-boundary pagination and numeric/post anchors",
  );

  let rankBookmark;
  for (const order of ["main", "rescue", "input", "direct", "fused"]) {
    for (const descending of [false, true]) {
      const expected = oracle(
        full,
        ["direct", "fused"].includes(order) ? "main" : order,
        descending,
      );
      for (const position of [
        1,
        128,
        129,
        expected.length - 48,
        expected.length,
      ]) {
        const options = { order, descending, start_rank: String(position) };
        const anchored = await page(scope(full), options);
        expectRows(anchored.items, expected.slice(position - 1, position + 12));
        assert.ok(anchored.start_cursor);
        const resized = await page(scope(full), {
          ...options,
          cursor: anchored.start_cursor,
          limit: 7,
        });
        expectRows(resized.items, expected.slice(position - 1, position + 6));
        if (anchored.next_cursor) {
          const next = await page(scope(full), {
            ...options,
            cursor: anchored.next_cursor,
          });
          expectRows(next.items, expected.slice(position + 12, position + 25));
        } else assert.equal(position, expected.length);
        if (order === "main" && !descending && position === 129)
          rankBookmark = anchored;
      }
      if (order === "input") continue;
      const rankOrder = ["direct", "fused"].includes(order) ? "main" : order;
      for (const rating of ["g", "s", "q", "e"]) {
        const n = expected.findIndex(
          (row) => row.rating === rating && row[rankOrder + "_rank"] === 3,
        );
        assert.ok(n >= 0);
        const value = await page(scope(full), {
          order,
          descending,
          start_rank: "3",
          start_rating: rating,
        });
        expectRows(value.items, expected.slice(n, n + 13));
      }
    }
  }
  checks.push(
    "numeric total positions and frozen Rating ranks include their member across orders, directions, bookmark edges, resizing and continuation",
  );
  for (const body of [
    ...["0", "-1", "1e3", "1.5", "9223372036854775808"].map((start_rank) => ({
      start_rank,
    })),
    { start_rank: "1", start_post_id: "1" },
    { start_rating: "g" },
    { start_rank: "1", start_rating: "x" },
    { start_rank: "1", start_rating: "g", order: "input" },
  ])
    await engine.expectError(
      base + "/ranking-browse/assets",
      "POST",
      { scope: scope(full), ...body },
      "INVALID_INPUT",
    );
  for (const body of [
    { start_rank: String(small.count + 1) },
    { start_rank: "6", start_rating: "g" },
  ]) {
    await page(scope(small));
    await engine.expectError(
      base + "/ranking-browse/assets",
      "POST",
      { scope: scope(small), order: "main", ...body },
      "RANK_POSITION_NOT_FOUND",
    );
  }
  for (const change of [
    { start_rank: "130" },
    { start_rating: "g" },
    { descending: true },
    { order: "rescue" },
    { scope: scope(small) },
  ])
    await engine.expectError(
      base + "/ranking-browse/assets",
      "POST",
      {
        scope: scope(full),
        order: "main",
        start_rank: "129",
        cursor: rankBookmark.start_cursor,
        ...change,
      },
      "INVALID_INPUT",
    );
  const forgedRank = JSON.parse(
    Buffer.from(rankBookmark.start_cursor, "base64url").toString("utf8"),
  );
  forgedRank.start = oracle(full)[0].ordinal;
  await engine.expectError(
    base + "/ranking-browse/assets",
    "POST",
    {
      scope: scope(full),
      order: "main",
      start_rank: "129",
      cursor: Buffer.from(JSON.stringify(forgedRank)).toString("base64url"),
    },
    "INVALID_INPUT",
  );
  checks.push(
    "invalid numeric targets, missing filtered ranks, conflicting anchors and mismatched or forged rank cursors are rejected",
  );

  for (const order of ["main", "rescue", "input"]) {
    for (const descending of [false, true]) {
      const expected = oracle(small, order, descending);
      const actual = [];
      let cursor;
      do {
        const result = await page(scope(small), {
          order,
          descending,
          cursor,
          limit: 7,
        });
        actual.push(...result.items);
        cursor = result.next_cursor;
      } while (cursor);
      expectRows(actual, expected);
      assert.equal(actual.length, 20);
      assert.ok(actual.every((item) => item.ranking.main_rank <= 5));
    }
  }
  checks.push(
    "changing view order never reinterprets the saved top-five membership condition",
  );

  const middle = oracle(full)[Math.floor(full.count / 2) + 37];
  assert.ok(middle.main_rank > 5);
  const postId = String(middle.post_id);
  let anchored;
  for (const order of ["main", "rescue", "input"]) {
    for (const descending of [false, true]) {
      const expected = oracle(full, order, descending);
      const index = expected.findIndex((row) => String(row.post_id) === postId);
      const first = await page(scope(full), {
        order,
        descending,
        start_post_id: postId,
      });
      expectRows(first.items, expected.slice(index, index + 13));
      assert.equal(first.items[0].ranking.post_id, postId);
      assert.ok(first.start_cursor);
      const replay = await page(scope(full), {
        order,
        descending,
        start_post_id: postId,
        cursor: first.start_cursor,
      });
      expectRows(replay.items, expected.slice(index, index + 13));
      if (first.next_cursor) {
        const second = await page(scope(full), {
          order,
          descending,
          start_post_id: postId,
          cursor: first.next_cursor,
        });
        expectRows(second.items, expected.slice(index + 13, index + 26));
      }
      if (order === "main" && !descending) anchored = first;
    }
  }
  checks.push(
    "Danbooru ID anchors include the located member and continue from its position in either direction",
  );
  for (const descending of [false, true]) {
    const expected = oracle(full, "main", descending);
    const end = expected.at(-1);
    const value = await page(scope(full), {
      order: "main",
      descending,
      start_post_id: String(end.post_id),
    });
    expectRows(value.items, [end]);
    assert.equal(value.next_cursor, null);
  }
  checks.push("anchors at either end finish without an empty extra page");

  await engine.expectError(
    base + "/ranking-browse/assets",
    "POST",
    { scope: scope(full), start_post_id: "9223372036854775807" },
    "RANK_ANCHOR_NOT_FOUND",
  );
  for (const start_post_id of ["-1", "0", "1e3", "9223372036854775808"])
    await engine.expectError(
      base + "/ranking-browse/assets",
      "POST",
      { scope: scope(full), start_post_id },
      "INVALID_INPUT",
    );
  await engine.expectError(
    base + "/ranking-browse/assets",
    "POST",
    { scope: scope(small), start_post_id: postId },
    "RANK_ANCHOR_NOT_FOUND",
  );
  checks.push(
    "missing, out-of-scope and invalid Danbooru IDs produce explicit errors",
  );

  for (const changed of [
    { scope: scope(small) },
    { descending: true },
    { order: "rescue" },
    { start_post_id: String(Number(postId) + 1) },
  ]) {
    await engine.expectError(
      base + "/ranking-browse/assets",
      "POST",
      {
        scope: scope(full),
        order: "main",
        descending: false,
        start_post_id: postId,
        cursor: anchored.next_cursor,
        ...changed,
      },
      "INVALID_INPUT",
    );
  }
  const smallFirst = await page(scope(small), { order: "main", limit: 2 });
  const forged = JSON.parse(
    Buffer.from(smallFirst.next_cursor, "base64url").toString("utf8"),
  );
  forged.state.pending = [middle.ordinal];
  await engine.expectError(
    base + "/ranking-browse/assets",
    "POST",
    {
      scope: scope(small),
      order: "main",
      cursor: Buffer.from(JSON.stringify(forged)).toString("base64url"),
    },
    "INVALID_INPUT",
  );
  checks.push(
    "cursor scope, order, direction, anchor and buffered membership are validated",
  );

  const natural = await engine.api(
    base + "/assets?collection_id=" + full.id + "&order=asset_key_asc&limit=8",
  );
  assert.ok(
    natural.items.every((item) => item.ranking?.artifact_id === artifact.id),
  );
  const existing = await engine.api(base + "/objects/workset/" + full.id);
  await engine.api(base + "/objects/workset/" + full.id, "PATCH", {
    expected_revision: existing.object.revision,
    name: "改名的排名工作集",
    notes: "位置应继续有效",
  });
  const afterRename = await page(scope(full), {
    order: "main",
    start_post_id: postId,
    cursor: anchored.start_cursor,
  });
  assert.equal(afterRename.items[0].ranking.post_id, postId);
  checks.push(
    "normal image ordering still exposes scores, and renaming preserves ranking cursors",
  );

  const definition = await engine.api(base + "/queries", "POST", {
    name: "排名工作集中的 G",
    spec: {
      version: 3,
      source_ids: [source.id],
      conditions: [
        {
          field: "rating",
          operator: "eq",
          value: { type: "text", value: "g" },
        },
      ],
      observation_rule: "current_post",
      order: "asset_key_asc",
      input_scope: scope(full),
    },
  });
  const queued = await engine.api(
    base + "/queries/" + definition.id + "/results",
    "POST",
    { expected_revision: definition.revision },
  );
  const result = await engine.wait(
    base + "/query-results/" + queued.id,
    (value) => ["ready", "failed"].includes(value.state),
    60000,
  );
  assert.equal(result.state, "ready", JSON.stringify(result));
  const filteredScope = {
    project_id: project.id,
    target: { kind: "query_result", result_id: result.id },
  };
  const filteredInfo = await engine.api(
    base + "/ranking-browse?result_id=" + result.id,
  );
  assert.equal(filteredInfo.ranking.workset_id, full.id);
  assert.equal(filteredInfo.ranking.count, result.count);
  const filtered = await page(filteredScope, {
    order: "rescue",
    descending: true,
  });
  expectRows(
    filtered.items,
    oracle(full, "rescue", true, result.id).slice(0, 13),
  );
  const filteredNatural = await engine.api(
    base +
      "/query-results/" +
      result.id +
      "/assets?order=asset_key_asc&limit=5",
  );
  assert.ok(filteredNatural.page.items.every((item) => item.ranking));
  checks.push(
    "queries within a ranking workset retain scores and obey their own fixed membership",
  );

  assert.ok(result.count > 4096, "exercise the large query scope path");
  const cold = await page(filteredScope, { limit: 48 });
  assert.equal(
    cold.preparations,
    0,
    "the existing scope index must serve other orders",
  );
  const expectedG = oracle(full, "main", false, result.id);
  expectRows(cold.items, expectedG.slice(0, 48));
  for (const limit of [12, 96, 48]) {
    const started = performance.now();
    const resized = await engine.api(base + "/ranking-browse/assets", "POST", {
      scope: filteredScope,
      limit,
    });
    assert.equal(resized.preparing ?? null, null);
    expectRows(resized.items, expectedG.slice(0, limit));
    samples.push({
      kind: "cached-first-page",
      limit,
      ms: performance.now() - started,
    });
  }
  checks.push(
    "changing page size reuses the resolved G origin without rescanning the leading E members",
  );
  const falseFirst = JSON.parse(
    Buffer.from(cold.next_cursor, "base64url").toString("utf8"),
  );
  falseFirst.first_page = true;
  const later = await engine.api(base + "/ranking-browse/assets", "POST", {
    scope: filteredScope,
    limit: 12,
    cursor: Buffer.from(JSON.stringify(falseFirst)).toString("base64url"),
  });
  expectRows(later.items, expectedG.slice(48, 60));
  const stillFirst = await engine.api(base + "/ranking-browse/assets", "POST", {
    scope: filteredScope,
    limit: 12,
  });
  expectRows(stillFirst.items, expectedG.slice(0, 12));
  checks.push(
    "a client-modified first-page flag cannot poison the shared resolved origin",
  );

  const keys = cold.items.slice(0, 3).map((item) => item.key);
  const selectionBefore = await engine.api(base + "/selection");
  const selectionAfter = await engine.api(base + "/selection", "PATCH", {
    expected_revision: selectionBefore.revision,
    add: [keys[1]],
    remove: [],
    clear: true,
  });
  const flags = await engine.api(base + "/selection/members", "POST", { keys });
  assert.equal(flags.revision, selectionAfter.revision);
  assert.deepEqual(flags.selected, [false, true, false]);
  checks.push(
    "selection membership can refresh independently of the ranked asset page",
  );

  let summaries;
  for (let attempt = 0; attempt < 600; attempt++) {
    summaries = await engine.api(base + "/assets/summaries", "POST", { keys });
    if (!summaries.preparing) break;
    await new Promise((done) => setTimeout(done, 100));
  }
  assert.equal(summaries.preparing, false);
  for (const item of summaries.items) {
    const posts = [
      ...new Set(
        fixture.assets
          .filter((asset) => asset.sha256 === item.key.asset_id)
          .map((asset) => String(asset.post_id)),
      ),
    ].sort((a, b) => Number(a) - Number(b));
    assert.deepEqual(item.summary.post_ids, posts.slice(0, 8));
    assert.equal(item.summary.post_count, String(posts.length));
  }
  checks.push(
    "asynchronous identity summaries match the source oracle and preserve source hashes",
  );

  const rankedRows = base + "/artifacts/" + artifact.id + "/ranking/rows";
  const rankedCount = base + "/artifacts/" + artifact.id + "/ranking/count";
  const sparseFilter = {
    eligibility: "eligible",
    missing_only: true,
    order: "main",
  };
  const sparsePage = await engine.api(rankedRows, "POST", {
    filter: sparseFilter,
    limit: 48,
  });
  assert.equal(
    sparsePage.count,
    null,
    "the first page must not synchronously calculate an unknown count",
  );
  if (sparsePage.scan) assert.ok(sparsePage.scan.scanned <= 4096);
  const counted = await engine.api(rankedCount, "POST", {
    filter: sparseFilter,
  });
  const exactMissing = Number(
    database
      .prepare(
        "SELECT count(*) AS n FROM scores WHERE eligibility='eligible' AND missing_flags!='[]'",
      )
      .get().n,
  );
  assert.equal(counted.count, exactMissing);
  assert.deepEqual(
    await engine.api(rankedCount, "POST", { filter: sparseFilter }),
    counted,
  );
  checks.push(
    "sparse row reads have a bounded scan and counts are calculated and cached separately",
  );

  for (const [path, body] of [
    [base + "/ranking-browse/assets", { scope: filteredScope, limit: 12 }],
    [
      rankedRows,
      { filter: { eligibility: "eligible", order: "main" }, limit: 12 },
    ],
    [rankedCount, { filter: sparseFilter }],
    [base + "/selection/members", { keys }],
    [base + "/assets/summaries", { keys }],
  ]) {
    const readId = crypto.randomUUID();
    await engine.api(base + "/read-requests/" + readId + "/cancel", "POST");
    const response = await fetch(engine.connection.endpoint + path, {
      method: "POST",
      headers: {
        authorization: "Bearer " + engine.connection.token,
        "content-type": "application/json",
        "x-studio-read-id": readId,
      },
      body: JSON.stringify(body),
    });
    assert.equal(response.status, 409);
    assert.equal((await response.json()).code, "CANCELLED");
  }
  checks.push(
    "all five POST read routes honor explicit cancellation before work starts",
  );

  const viewLease = crypto.randomUUID();
  await engine.api(base + "/ranking-browse/lease", "POST", {
    scope: scope(full),
    lease_id: viewLease,
    release: false,
  });
  const clearIndexes = async () => {
    await engine.api("/v1/settings/cache/clear", "POST", { tier: "temporary" });
    await engine.wait("/v1/settings", (s) => !s.storage.cleanup_pending, 15000);
  };
  await clearIndexes();
  assert.equal((await page(scope(full))).preparations, 0);
  assert.equal(
    (await engine.api("/v1/resources")).query_cache.ranked_indexes,
    0,
  );
  await engine.api(base + "/ranking-browse/lease", "POST", {
    scope: scope(full),
    lease_id: viewLease,
    release: true,
  });
  const beforeRebuild = (await engine.api("/v1/resources")).query_cache
    .ranked_index_builds;
  for (let attempt = 0; attempt < 2; attempt++) {
    await clearIndexes();
    assert.equal(
      (await engine.api("/v1/resources")).query_cache.ranked_indexes,
      0,
    );
    const started = performance.now();
    const rebuilt = await page(scope(full));
    assert.ok(performance.now() - started < 15000);
    expectRows(rebuilt.items, oracle(full).slice(0, 13));
    assert.equal(
      (await engine.api("/v1/resources")).query_cache.ranked_index_builds,
      beforeRebuild,
    );
  }
  checks.push(
    "fixed ranking worksets remain directly readable after repeated cache clearing, without rebuilding indexes",
  );

  sparseResult = await engine.api(base + "/query-results", "POST", {
    spec: {
      version: 3,
      source_ids: [source.id],
      observation_rule: "current_post",
      order: "asset_key_asc",
      input_scope: scope(full),
      conditions: [
        {
          field: "tags",
          operator: "has_all_tags",
          value: { type: "text_list", value: ["sparse_browse"] },
        },
      ],
    },
  });
  sparseResult = await engine.wait(
    base + "/query-results/" + sparseResult.id,
    (r) => ["ready", "failed"].includes(r.state),
    60000,
  );
  assert.equal(sparseResult.state, "ready");
  assert.ok(sparseResult.count > 4096);
  const sparseScope = {
    project_id: project.id,
    target: { kind: "query_result", result_id: sparseResult.id },
  };
  sparseExpected = oracle(full, "main", false, sparseResult.id);
  assert.equal(sparseExpected[0].rating, "e");
  assert.ok(sparseExpected.slice(0, 96).some((item) => item.rating === "s"));
  const allPositions = new Map(
    oracle(full).map((row, index) => [row.ordinal, index]),
  );
  assert.ok(
    allPositions.get(sparseExpected[95].ordinal) -
      allPositions.get(sparseExpected[0].ordinal) >
      4096,
  );
  const beforeBuilds = (await engine.api("/v1/resources")).query_cache
    .ranked_index_builds;
  await Promise.all(
    [12, 48, 96].map((limit) =>
      engine.api(base + "/ranking-browse/assets", "POST", {
        scope: sparseScope,
        limit,
      }),
    ),
  );
  const sparseReady = await page(sparseScope, { limit: 96 });
  expectRows(sparseReady.items, sparseExpected.slice(0, 96));
  const sparseRank = await page(sparseScope, { start_rank: "4097", limit: 12 });
  expectRows(sparseRank.items, sparseExpected.slice(4096, 4108));
  const sparseRating = sparseExpected.find((row) => row.rating === "s");
  const sparseOffset = sparseExpected.indexOf(sparseRating);
  const sparseByRating = await page(sparseScope, {
    start_rank: String(sparseRating.main_rank),
    start_rating: "s",
  });
  expectRows(
    sparseByRating.items,
    sparseExpected.slice(sparseOffset, sparseOffset + 13),
  );
  checks.push(
    "numeric anchors use the sparse query result's fixed members and retain original Rating rank numbers",
  );
  assert.equal(
    (await engine.api("/v1/resources")).query_cache.ranked_index_builds,
    beforeBuilds + 1,
  );
  for (const limit of [12, 48, 96, 12]) {
    const value = await engine.api(base + "/ranking-browse/assets", "POST", {
      scope: sparseScope,
      limit,
    });
    assert.equal(value.preparing ?? null, null);
    expectRows(value.items, sparseExpected.slice(0, limit));
  }
  checks.push(
    "one persistent index serves concurrent sizes and every first-page size for a sparse scope",
  );

  gapResult = await engine.api(base + "/query-results", "POST", {
    spec: {
      version: 3,
      source_ids: [source.id],
      observation_rule: "current_post",
      order: "asset_key_asc",
      input_scope: scope(full),
      conditions: [
        {
          field: "rating",
          operator: "in",
          value: { type: "text_list", value: ["e", "s"] },
        },
      ],
    },
  });
  gapResult = await engine.wait(
    base + "/query-results/" + gapResult.id,
    (r) => ["ready", "failed"].includes(r.state),
    60000,
  );
  assert.equal(gapResult.state, "ready");
  const gapScope = {
    project_id: project.id,
    target: { kind: "query_result", result_id: gapResult.id },
  };
  gapExpected = oracle(full, "main", false, gapResult.id);
  assert.ok(gapExpected.length > 110 * 96);
  const firstGap = await page(gapScope, { limit: 96 });
  const bookmarks = [null];
  let next = firstGap.next_cursor;
  expectRows(firstGap.items, gapExpected.slice(0, 96));
  const built = (await engine.api("/v1/resources")).query_cache
    .ranked_index_builds;
  for (let number = 1; number < 110; number++) {
    bookmarks.push(next);
    const result = await engine.api(base + "/ranking-browse/assets", "POST", {
      scope: gapScope,
      limit: 96,
      cursor: next,
    });
    assert.equal(
      result.preparing ?? null,
      null,
      "every subsequent page must use the range index",
    );
    expectRows(result.items, gapExpected.slice(number * 96, (number + 1) * 96));
    next = result.next_cursor;
  }
  for (const number of [2, 84, 17, 85, 109, 84]) {
    const result = await engine.api(base + "/ranking-browse/assets", "POST", {
      scope: gapScope,
      limit: 96,
      cursor: bookmarks[number],
    });
    assert.equal(result.preparing ?? null, null);
    expectRows(result.items, gapExpected.slice(number * 96, (number + 1) * 96));
  }
  assert.equal(
    (await engine.api("/v1/resources")).query_cache.ranked_index_builds,
    built,
  );
  samples.push({
    kind: "all-page-index",
    forward_pages: 110,
    backward_replays: 6,
    members_verified: 110 * 96,
  });
  checks.push(
    "110 pages and repeated old-page bookmarks cross the missing-rating gap without any scan or rebuild",
  );

  const frozenSpec = {
    version: 3,
    source_ids: [source.id],
    observation_rule: "current_post",
    order: "post_id_desc",
    input_scope: scope(full),
    conditions: [
      {
        field: `project.${artifact.id}.rating`,
        operator: "in",
        value: { type: "text_list", value: ["s", "e", "s"] },
      },
    ],
  };
  const frozen = await engine.api(base + "/query-results", "POST", {
    spec: frozenSpec,
  });
  const alias = await engine.api(base + "/query-results", "POST", {
    spec: frozenSpec,
  });
  assert.equal(frozen.state, "ready");
  assert.equal(frozen.count, gapResult.count);
  assert.equal(alias.cache.mode, "reused");
  assert.equal(
    database
      .prepare(
        "SELECT storage_kind FROM project_state.query_results WHERE id=?",
      )
      .get(frozen.id).storage_kind,
    "ranking",
  );
  const frozenScope = {
    project_id: project.id,
    target: { kind: "query_result", result_id: frozen.id },
  };
  expectRows((await page(frozenScope)).items, gapExpected.slice(0, 13));
  for (const order of [
    "asset_key_asc",
    "asset_key_desc",
    "post_id_asc",
    "post_id_desc",
  ]) {
    const read = (id, cursor) =>
      engine.api(
        base +
          "/query-results/" +
          id +
          "/assets?order=" +
          order +
          "&limit=13" +
          (cursor ? "&cursor=" + encodeURIComponent(cursor) : ""),
      );
    const projected = await read(frozen.id);
    const materialized = await read(gapResult.id);
    assert.deepEqual(
      projected.page.items.map(identity),
      materialized.page.items.map(identity),
    );
    assert.deepEqual(
      (await read(frozen.id, projected.page.next_cursor)).page.items.map(
        identity,
      ),
      (await read(gapResult.id, materialized.page.next_cursor)).page.items.map(
        identity,
      ),
    );
    await engine.expectError(
      base +
        "/query-results/" +
        alias.id +
        "/assets?order=" +
        order +
        "&cursor=" +
        encodeURIComponent(projected.page.next_cursor),
      "GET",
      undefined,
      "INVALID_INPUT",
    );
  }
  const empty = await engine.api(base + "/query-results", "POST", {
    spec: {
      ...frozenSpec,
      input_scope: frozenScope,
      conditions: [
        {
          field: `project.${artifact.id}.rating`,
          operator: "eq",
          value: { type: "text", value: "g" },
        },
      ],
    },
  });
  assert.equal(empty.state, "ready");
  assert.equal(empty.count, 0);
  const emptyPage = await page({
    project_id: project.id,
    target: { kind: "query_result", result_id: empty.id },
  });
  assert.deepEqual(emptyPage.items, []);
  assert.equal(emptyPage.next_cursor, null);
  checks.push(
    "frozen multiple-Rating refinements are immediately ready, reuse members, preserve all natural orders and reject foreign cursors; conflicting refinements are empty",
  );

  const owner = await engine.api(
    base + "/artifacts/" + artifact.id + "/ranking/worksets",
    "POST",
    {
      idempotency_key: crypto.randomUUID(),
      name: "引用生命周期",
      filter: { eligibility: "eligible", top: 3, order: "main" },
    },
  );
  let selected = await engine.api(base + "/selection/scope", "POST", {
    expected_revision: (await engine.api(base + "/selection")).revision,
    scope: scope(owner),
    operation: "replace",
  });
  assert.equal(selected.count, 12);
  assert.ok(selected.base_result);
  assert.equal(
    database.prepare("SELECT count(*) n FROM project_state.selection").get().n,
    0,
  );
  const ownerPage = await page(scope(owner));
  const ownerKeys = ownerPage.items.map((item) => item.key);
  selected = await engine.api(base + "/selection", "PATCH", {
    expected_revision: selected.revision,
    add: [],
    remove: [ownerKeys[0]],
    clear: false,
  });
  assert.equal(selected.count, 11);
  assert.equal(selected.excluded_count, 1);
  const snapshot = await engine.api(base + "/collections", "POST", {
    name: "稀疏排除的普通工作集",
    scope: {
      project_id: project.id,
      target: { kind: "selection", revision: selected.revision },
    },
  });
  const capturedJob = await engine.api(base + "/jobs", "POST", {
    idempotency_key: crypto.randomUUID(),
    scope: scope(snapshot),
    delay_ms: 0,
  });
  assert.equal(capturedJob.total, 11);
  assert.equal(
    database
      .prepare(
        "SELECT count(*) n FROM project_state.collection_member_legacy WHERE collection_id IN (?,?)",
      )
      .get(owner.id, snapshot.id).n,
    0,
  );
  assert.equal(
    database
      .prepare(
        "SELECT count(*) n FROM project_state.job_input_legacy WHERE job_id=?",
      )
      .get(capturedJob.id).n,
    0,
  );
  const ownerDetail = await engine.api(base + "/objects/workset/" + owner.id);
  await engine.api(base + "/objects/workset/" + owner.id + "/actions", "POST", {
    action: "remove",
    expected_revision: ownerDetail.object.revision,
  });
  const flagsAfterDelete = await engine.api(
    base + "/selection/members",
    "POST",
    { keys: ownerKeys },
  );
  assert.deepEqual(flagsAfterDelete.selected, [false, ...Array(11).fill(true)]);
  assert.equal(
    (await engine.api(base + "/ranking-browse?collection_id=" + snapshot.id))
      .ranking,
    null,
  );
  const completed = await engine.wait(
    base + "/jobs",
    (value) =>
      value.items.some(
        (item) =>
          item.id === capturedJob.id &&
          ["succeeded", "failed"].includes(item.status),
      ),
    30000,
  );
  assert.equal(
    completed.items.find((item) => item.id === capturedJob.id).status,
    "succeeded",
  );
  await engine.expectError(
    base + "/query-results/" + selected.base_result + "/release",
    "POST",
    undefined,
    "RESULT_IN_USE",
  );
  checks.push(
    "ranked all-selection, sparse exclusion, ordinary workset and job share fixed members without copies and survive deletion of the original workset",
  );

  const ordinary = await engine.api(base + "/collections", "POST", {
    name: "普通成员副本",
    scope: scope(small),
  });
  assert.equal(
    (await engine.api(base + "/ranking-browse?collection_id=" + ordinary.id))
      .ranking,
    null,
  );
  await engine.expectError(
    base + "/ranking-browse/assets",
    "POST",
    { scope: scope(ordinary) },
    "RANKING_SCOPE_UNSUPPORTED",
  );
  checks.push(
    "an ordinary workset is not misclassified just because it references an artifact",
  );

  database.close();
  database = undefined;
  await engine.stop();
  await engine.start();
  await engine.api(base + "/open", "POST");
  const resumed = await page(scope(full), {
    order: "main",
    start_post_id: postId,
    cursor: anchored.start_cursor,
  });
  assert.equal(resumed.items[0].ranking.post_id, postId);
  assert.equal(resumed.preparations, 0);
  assert.equal(
    (await engine.api("/v1/resources")).query_cache.ranked_index_builds,
    0,
    "restart must reuse completed disk indexes",
  );
  checks.push("a resolved starting cursor survives an engine restart");
  const resumedRank = await page(scope(full), {
    order: "main",
    start_rank: "129",
    cursor: rankBookmark.start_cursor,
  });
  assert.deepEqual(resumedRank.items, rankBookmark.items);
  assert.equal(resumedRank.preparations, 0);
  checks.push(
    "a numeric starting cursor and its sparse positional index survive an engine restart",
  );
  assert.deepEqual(await Promise.all(artifactFiles.map(hash)), artifactHashes);
  assert.deepEqual(await Promise.all(sourceFiles.map(hash)), sourceHashes);
  checks.push(
    "browsing and positioning leave all ranking materials and data lake files unchanged",
  );
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      {
        passed: true,
        checks,
        samples,
        project_id: project.id,
        full,
        small,
        artifact_id: artifact.id,
        anchor_post_id: postId,
        sparse_result_id: sparseResult.id,
        gap_result_id: gapResult.id,
        gap_post_ids: gapExpected.map((row) => String(row.post_id)),
        sparse_first_members: sparseExpected.slice(0, 129),
      },
      null,
      2,
    ),
  );
  console.log(JSON.stringify({ passed: true, checks: checks.length, run }));
} catch (error) {
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      { passed: false, checks, samples, error: String(error) },
      null,
      2,
    ),
  );
  throw error;
} finally {
  database?.close();
  await engine.stop();
}
