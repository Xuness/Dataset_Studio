import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, readFile, writeFile, copyFile, link } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath, URLSearchParams } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { EngineFixture, sleep } from "./engine-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local/test-runs",
  "integration-scoped-browse-" + Date.now(),
);
await mkdir(run, { recursive: true });
await promisify(execFile)(
  "python",
  [
    resolve(root, "tooling/ranking-fixture.py"),
    resolve(run, "fixture"),
    "20000",
  ],
  { cwd: root, windowsHide: true },
);
const fixture = JSON.parse(
  await readFile(resolve(run, "fixture/fixture.json"), "utf8"),
);
const otherLake = resolve(run, "second-lake");
await mkdir(resolve(otherLake, "indexes/gen-ranking"), { recursive: true });
await mkdir(resolve(otherLake, "packs"), { recursive: true });
for (const file of ["catalog.sqlite", "analysis.duckdb"])
  await copyFile(
    resolve(fixture.lake, "indexes/gen-ranking", file),
    resolve(otherLake, "indexes/gen-ranking", file),
  );
await link(
  resolve(fixture.lake, "packs/fixture.tar"),
  resolve(otherLake, "packs/fixture.tar"),
);
const secondId = crypto.randomUUID();
for (const file of ["CURRENT.json", "library.json"]) {
  const value = JSON.parse(await readFile(resolve(fixture.lake, file), "utf8"));
  value.library_id = secondId;
  await writeFile(resolve(otherLake, file), JSON.stringify(value));
}
const engine = new EngineFixture(root, resolve(run, "state"));
const checks = [],
  timings = {};
const lex = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
const identity = (key) => key.source_id + ":" + key.asset_id;
const posts = new Map();
for (const asset of fixture.assets)
  if (asset.post_id != null)
    posts.set(
      asset.sha256,
      Math.min(posts.get(asset.sha256) ?? Infinity, asset.post_id),
    );
const compare = (order) => (a, b) => {
  const ap = posts.get(a.asset_id),
    bp = posts.get(b.asset_id);
  const sign = order.endsWith("desc") ? -1 : 1;
  if (order.startsWith("post")) {
    if (ap == null && bp != null) return 1;
    if (ap != null && bp == null) return -1;
    if (ap != null && bp != null && ap !== bp) return sign * (ap - bp);
  }
  return sign * (lex(a.source_id, b.source_id) || lex(a.asset_id, b.asset_id));
};
let project, base;
async function page(cid, order, cursor = null, limit = 113, selection = false) {
  return engine.api(
    base +
      "/assets?" +
      new URLSearchParams({
        ...(selection ? { selection: "true" } : { collection_id: cid }),
        order,
        limit: String(limit),
        ...(cursor ? { cursor } : {}),
      }),
  );
}
async function all(cid, order, expected) {
  const rows = [],
    seen = new Set();
  let cursor = null,
    scans = 0;
  for (let request = 0; request < 1000; request++) {
    const data = await page(cid, order, cursor);
    if (data.preparing) {
      assert.equal(data.items.length, 0);
      assert.ok(
        !data.result_id,
        "Indexed scope browsing must not materialize another query",
      );
      if (data.scan) {
        scans++;
        assert.ok(data.next_cursor);
        assert.ok(
          data.scan.scanned > 0 && data.scan.scanned <= data.scan.total,
        );
      }
      cursor = data.next_cursor ?? cursor;
      await sleep(20);
      continue;
    }
    assert.ok(data.items.length <= 113);
    for (const item of data.items) {
      assert.ok(!seen.has(identity(item.key)), "No duplicate across pages");
      seen.add(identity(item.key));
      rows.push(item.key);
    }
    cursor = data.next_cursor;
    if (!cursor) break;
    if (request === 999) throw new Error("Scope paging did not terminate");
  }
  assert.deepEqual(
    rows.map(identity),
    [...expected].sort(compare(order)).map(identity),
  );
  return { count: rows.length, scans };
}
try {
  await engine.start();
  project = await engine.api("/v1/projects", "POST", {
    name: "有界范围排序验收",
  });
  base = "/v1/projects/" + project.id;
  const a = await engine.api(base + "/sources", "POST", {
    name: "来源 A",
    kind: "danbooru",
    index_root: fixture.lake,
    media_root: fixture.lake,
  });
  const b = await engine.api(base + "/sources", "POST", {
    name: "来源 B",
    kind: "danbooru",
    index_root: otherLake,
    media_root: otherLake,
  });
  const keysA = fixture.objects.map((o) => ({
    source_id: a.id,
    asset_id: o.sha,
  }));
  const keysB = fixture.objects.map((o) => ({
    source_id: b.id,
    asset_id: o.sha,
  }));
  const oldest = [...keysA]
    .filter((key) => posts.has(key.asset_id))
    .sort(compare("post_id_asc"))
    .slice(0, 5000);
  const missing = keysA.find((key) => !posts.has(key.asset_id));
  assert.ok(missing);
  const small = [
    missing,
    keysA.find((key) => posts.has(key.asset_id)),
    keysB.find((key) => posts.has(key.asset_id)),
  ];
  const collections = [
    { id: crypto.randomUUID(), name: "全范围", keys: keysA },
    { id: crypto.randomUUID(), name: "稀疏旧成员", keys: oldest },
    {
      id: crypto.randomUUID(),
      name: "多来源全范围",
      keys: [...keysA, ...keysB],
    },
    { id: crypto.randomUUID(), name: "小范围与未知帖子", keys: small },
  ];
  await engine.stop();
  // Seed only this stopped fixture's immutable worksets; the API scenarios below
  // exercise the actual engine, catalog, index and cursor implementations.
  const db = new DatabaseSync(resolve(project.directory, "project.sqlite"));
  db.exec("BEGIN");
  const collection = db.prepare("INSERT INTO collections VALUES (?,?,?)");
  const member = db.prepare("INSERT INTO collection_members VALUES (?,?,?)");
  for (const item of collections) {
    collection.run(item.id, item.name, item.keys.length);
    for (const key of item.keys)
      member.run(item.id, key.source_id, key.asset_id);
  }
  db.exec("COMMIT");
  db.close();
  await engine.start();
  await engine.api(base + "/open", "POST");
  for (const [i, order] of [
    [0, "post_id_desc"],
    [0, "post_id_asc"],
    [1, "post_id_desc"],
    [2, "post_id_desc"],
    [2, "post_id_asc"],
    [3, "post_id_desc"],
    [3, "post_id_asc"],
    [3, "asset_key_desc"],
  ]) {
    const begin = Date.now();
    const result = await all(collections[i].id, order, collections[i].keys);
    timings[collections[i].name + ":" + order] = {
      milliseconds: Date.now() - begin,
      ...result,
    };
    if (i === 1)
      assert.ok(
        result.scans > 0,
        "Sparse prefix must use a bounded continuation",
      );
  }
  checks.push(
    "dense and sparse ranges page exactly, including two-source merges, ties and missing posts in both directions",
  );
  const first = await page(collections[0].id, "post_id_desc");
  assert.equal(first.items.length, 113);
  assert.ok(first.next_cursor);
  await engine.expectError(
    base +
      "/assets?" +
      new URLSearchParams({
        collection_id: collections[1].id,
        order: "post_id_desc",
        cursor: first.next_cursor,
      }),
    "GET",
    undefined,
    "SOURCE_CHANGED",
  );
  await engine.expectError(
    base +
      "/assets?" +
      new URLSearchParams({
        collection_id: collections[0].id,
        order: "post_id_asc",
        cursor: first.next_cursor,
      }),
    "GET",
    undefined,
    "SOURCE_CHANGED",
  );
  checks.push("cursors reject another scope and another order");
  let selection = await engine.api(base + "/selection");
  selection = await engine.api(base + "/selection", "PATCH", {
    expected_revision: selection.revision,
    add: small,
    remove: [],
    clear: false,
  });
  const selected = await page(null, "post_id_desc", null, 2, true);
  assert.deepEqual(
    selected.items.map((item) => identity(item.key)),
    [...small].sort(compare("post_id_desc")).slice(0, 2).map(identity),
  );
  await engine.api(base + "/selection", "PATCH", {
    expected_revision: selection.revision,
    add: [],
    remove: [small[0]],
    clear: false,
  });
  await engine.expectError(
    base +
      "/assets?" +
      new URLSearchParams({
        selection: "true",
        order: "post_id_desc",
        cursor: selected.next_cursor,
      }),
    "GET",
    undefined,
    "SOURCE_CHANGED",
  );
  checks.push(
    "selection sorting respects membership and rejects stale selection cursors",
  );
  const read = new DatabaseSync(resolve(project.directory, "project.sqlite"), {
    readOnly: true,
  });
  assert.equal(read.prepare("SELECT count(*) n FROM query_results").get().n, 0);
  read.close();
  checks.push("scope browsing creates no full-population query result");
  await engine.stop();
  const catalog = new DatabaseSync(
    resolve(fixture.lake, "indexes/gen-ranking/catalog.sqlite"),
  );
  catalog.prepare("UPDATE state SET value=2 WHERE key='seq'").run();
  catalog.close();
  await engine.start();
  await engine.api(base + "/open", "POST");
  await engine.expectError(
    base +
      "/assets?" +
      new URLSearchParams({
        collection_id: collections[0].id,
        order: "post_id_desc",
        cursor: first.next_cursor,
      }),
    "GET",
    undefined,
    "SOURCE_CHANGED",
  );
  checks.push(
    "source revision changes reject a previous cursor before rebuilding an index",
  );
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify({ passed: true, checks, timings, run }, null, 2),
  );
  console.log(JSON.stringify({ passed: true, checks: checks.length, run }));
} catch (error) {
  await writeFile(
    resolve(run, "failure.json"),
    JSON.stringify(
      { error: String(error), stack: error.stack, checks, timings },
      null,
      2,
    ),
  );
  throw error;
} finally {
  await engine.stop();
}
