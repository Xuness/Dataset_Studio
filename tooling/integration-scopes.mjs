import assert from "node:assert/strict";
import { mkdir, readFile, writeFile, rename, copyFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { EngineFixture, sleep, within } from "./engine-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const runDir = resolve(
  root,
  ".local",
  "test-runs",
  "integration-scopes-" + Date.now(),
);
await mkdir(runDir, { recursive: true });
const engine = new EngineFixture(root, resolve(runDir, "state"));
const other = new EngineFixture(root, resolve(runDir, "other-engine"));
const checks = [];
const timings = {};
const projectPath = (id) => "/v1/projects/" + id;
const querySpec = (source, conditions = []) => ({
  version: 1,
  source_ids: [source],
  conditions,
  observation_rule: "any_observation",
  order: "asset_key_asc",
});
const resultScope = (project, result) => ({
  project_id: project,
  target: { kind: "query_result", result_id: result },
});
const jsonFile = (path, data) => writeFile(path, JSON.stringify(data, null, 2));
async function createQuery(pid, spec, name = "条件验证") {
  const query = await engine.api(projectPath(pid) + "/queries", "POST", {
    name,
    spec,
  });
  const result = await engine.api(
    projectPath(pid) + "/queries/" + query.id + "/results",
    "POST",
    { expected_revision: query.revision },
  );
  return { query, result };
}
async function ready(pid, rid) {
  const result = await engine.wait(
    projectPath(pid) + "/query-results/" + rid,
    (r) => ["ready", "failed", "cancelled", "interrupted"].includes(r.state),
    90000,
  );
  assert.equal(result.state, "ready", JSON.stringify(result));
  return result;
}
async function waitJob(pid, jid, predicate) {
  const value = await engine.wait(
    projectPath(pid) + "/jobs",
    (r) => predicate(r.items.find((j) => j.id === jid)),
    30000,
  );
  return value.items.find((j) => j.id === jid);
}
async function fixtureLake(count) {
  const lake = resolve(runDir, "索引夹具");
  const generation = resolve(lake, "indexes/gen-fixture");
  await mkdir(generation, { recursive: true });
  const id = crypto.randomUUID();
  await jsonFile(resolve(lake, "CURRENT.json"), {
    library_id: id,
    index_version: 1,
    generation: "gen-fixture",
  });
  await jsonFile(resolve(lake, "library.json"), {
    library_id: id,
    format_version: 1,
    image_format: "uncompressed-pax-tar",
  });
  const path = resolve(generation, "catalog.sqlite");
  const db = new DatabaseSync(path);
  db.exec(
    "CREATE TABLE state(key TEXT PRIMARY KEY,value INTEGER); INSERT INTO state VALUES ('seq',1); CREATE TABLE objects(sha256 TEXT PRIMARY KEY,pack_path TEXT,offset INTEGER,length INTEGER,stored_ext TEXT) WITHOUT ROWID; BEGIN",
  );
  const insert = db.prepare(
    "INSERT INTO objects VALUES (?,'packs/fixture.tar',0,123,'webp')",
  );
  for (let i = 0; i < count; i++) insert.run(i.toString(16).padStart(64, "0"));
  db.exec("COMMIT");
  db.close();
  return { id, lake, path };
}
process.on("exit", () => {
  engine.child?.kill();
  other.child?.kill();
});
try {
  await engine.start();
  const p = await engine.api("/v1/projects", "POST", { name: "范围协议验证" });
  const p2 = await engine.api("/v1/projects", "POST", { name: "隔离引用" });
  const path = projectPath(p.id);
  const demo = await engine.api(path + "/sources", "POST", {
    name: "内置参考",
    kind: "demo",
  });
  const fields = await engine.api(path + "/sources/" + demo.id + "/fields");
  assert.equal(fields.version, 1);
  assert.ok(
    fields.fields
      .find((f) => f.id === "stored.bytes")
      .operators.includes("gte"),
  );
  await engine.expectError(
    path + "/queries",
    "POST",
    {
      name: "类型不匹配",
      spec: querySpec(demo.id, [
        {
          field: "stored.bytes",
          operator: "gte",
          value: { type: "text", value: "1" },
        },
      ]),
    },
    "INVALID_INPUT",
  );
  await engine.expectError(
    path + "/queries",
    "POST",
    {
      name: "不支持",
      spec: querySpec(demo.id, [
        {
          field: "rating",
          operator: "eq",
          value: { type: "text", value: "g" },
        },
      ]),
    },
    "QUERY_UNSUPPORTED",
  );
  checks.push(
    "field directory rejects unsupported capabilities and wrong value types",
  );

  const { query, result } = await createQuery(p.id, querySpec(demo.id));
  assert.equal(result.count, null);
  const complete = await ready(p.id, result.id);
  assert.equal(complete.count, 32);
  const beforeSelection = await engine.api(path + "/selection");
  const first = await engine.api(
    path + "/query-results/" + result.id + "/assets?limit=7",
  );
  const second = await engine.api(
    path +
      "/query-results/" +
      result.id +
      "/assets?limit=7&cursor=" +
      encodeURIComponent(first.page.next_cursor),
  );
  assert.equal(
    new Set(
      [...first.page.items, ...second.page.items].map((a) => a.key.asset_id),
    ).size,
    14,
  );
  assert.deepEqual(await engine.api(path + "/selection"), beforeSelection);
  const duplicateResult = await engine.api(
    path + "/queries/" + query.id + "/results",
    "POST",
    { expected_revision: 1 },
  );
  await ready(p.id, duplicateResult.id);
  await engine.expectError(
    path +
      "/query-results/" +
      duplicateResult.id +
      "/assets?cursor=" +
      encodeURIComponent(first.page.next_cursor),
    "GET",
    undefined,
    "INVALID_INPUT",
  );
  await engine.expectError(
    projectPath(p2.id) + "/query-results/" + result.id,
    "GET",
    undefined,
    "NOT_FOUND",
  );
  checks.push(
    "result references page without duplicates, preserve focus-only reads and reject foreign cursors/projects",
  );

  const selectBody = {
    expected_revision: 0,
    scope: resultScope(p.id, result.id),
    operation: "replace",
  };
  assert.ok(Buffer.byteLength(JSON.stringify(selectBody)) < 240);
  let selection = await engine.api(
    path + "/selection/scope",
    "POST",
    selectBody,
  );
  assert.equal(selection.count, 32);
  assert.equal(selection.base_result, result.id);
  selection = await engine.api(path + "/selection", "PATCH", {
    expected_revision: selection.revision,
    add: [],
    remove: [first.page.items[0].key],
    clear: false,
  });
  assert.equal(selection.count, 31);
  assert.equal(selection.excluded_count, 1);
  const db = new DatabaseSync(resolve(p.directory, "project.sqlite"), {
    readOnly: true,
  });
  assert.equal(db.prepare("SELECT COUNT(*) AS n FROM selection").get().n, 0);
  db.close();
  await engine.expectError(
    projectPath(p2.id) + "/selection/scope",
    "POST",
    selectBody,
    "SCOPE_PROJECT_MISMATCH",
  );
  const frozen = {
    project_id: p.id,
    target: { kind: "selection", revision: selection.revision },
  };
  const collection = await engine.api(path + "/collections", "POST", {
    name: "固定 31 项",
    scope: frozen,
  });
  assert.equal(collection.count, 31);
  await engine.expectError(
    path + "/query-results/" + result.id + "/release",
    "POST",
    undefined,
    "RESULT_IN_USE",
  );
  checks.push(
    "all-select request is a small reference with sparse exclusion and referenced results cannot be released",
  );

  const submission = {
    idempotency_key: crypto.randomUUID(),
    scope: frozen,
    delay_ms: 55,
  };
  const job = await engine.api(path + "/jobs", "POST", submission);
  assert.equal(job.total, 31);
  await engine.api(path + "/selection", "PATCH", {
    expected_revision: selection.revision,
    add: [],
    remove: [],
    clear: true,
  });
  const updated = await engine.api(path + "/queries/" + query.id, "PATCH", {
    expected_revision: 1,
    name: "变更后的条件",
    spec: querySpec(demo.id, [
      {
        field: "stored.bytes",
        operator: "gte",
        value: { type: "integer", value: "1" },
      },
    ]),
  });
  assert.equal(updated.revision, 2);
  assert.equal(
    (await engine.api(path + "/jobs", "POST", submission)).id,
    job.id,
  );
  await waitJob(
    p.id,
    job.id,
    (j) => j?.status === "running" && j.completed >= 8,
  );
  assert.equal((await engine.api(path + "/close", "POST")).state, "background");
  await engine.expectError(
    path + "/selection",
    "GET",
    undefined,
    "PROJECT_CLOSED",
  );
  await engine.stop(true);
  await engine.start();
  await engine.api(path + "/open", "POST");
  const done = await waitJob(p.id, job.id, (j) => j?.status === "succeeded");
  assert.equal(done.total, 31);
  assert.ok(done.attempt >= 2);
  const output = await engine.response(path + "/jobs/" + job.id + "/artifact");
  const rows = (await output.text()).trim().split("\n").map(JSON.parse);
  assert.equal(rows.length, 31);
  assert.equal(new Set(rows.map((r) => r.asset.key.asset_id)).size, 31);
  assert.ok(
    rows.every(
      (r) => r.asset.key.asset_id !== first.page.items[0].key.asset_id,
    ),
  );
  assert.equal((await engine.api(path + "/collections")).items[0].count, 31);
  const metadata = JSON.parse(
    await readFile(
      resolve(p.directory, "artifacts", job.id + ".manifest.json"),
      "utf8",
    ),
  );
  assert.deepEqual(metadata.input_scope, frozen);
  checks.push(
    "closed views retain background jobs; a crash, changed selection and edited query preserve exact frozen input",
  );

  for (const scope of [
    {
      project_id: p.id,
      target: { kind: "source", source_id: demo.id, revision: demo.revision },
    },
    {
      project_id: p.id,
      target: { kind: "workset", collection_id: collection.id },
    },
    resultScope(p.id, result.id),
  ]) {
    const j = await engine.api(path + "/jobs", "POST", {
      idempotency_key: crypto.randomUUID(),
      scope,
    });
    const completed = await waitJob(
      p.id,
      j.id,
      (j) => j?.status === "succeeded",
    );
    assert.equal(completed.total, scope.target.kind === "workset" ? 31 : 32);
  }
  checks.push(
    "manifest accepts source, workset, query result and selection scopes through one protocol",
  );

  const lake = await fixtureLake(20000);
  const attached = await engine.api(path + "/sources", "POST", {
    name: "只读目录夹具",
    kind: "danbooru",
    index_root: lake.lake,
    media_root: lake.lake,
  });
  assert.equal(attached.id, lake.id);
  const started = Date.now();
  const large = await createQuery(p.id, querySpec(lake.id), "两万项范围");
  assert.equal((await ready(p.id, large.result.id)).count, 20000);
  timings.catalog_20000_ms = Date.now() - started;
  const state = await engine.api(path + "/selection");
  const beginSelect = Date.now();
  const largeSelection = await engine.api(path + "/selection/scope", "POST", {
    ...selectBody,
    expected_revision: state.revision,
    scope: resultScope(p.id, large.result.id),
  });
  assert.equal(largeSelection.count, 20000);
  timings.select_all_20000_ms = Date.now() - beginSelect;
  const middle = await engine.api(
    path + "/query-results/" + large.result.id + "/assets?limit=128",
  );
  assert.equal(middle.page.items.length, 128);
  const catalog = new DatabaseSync(lake.path);
  catalog.exec("UPDATE state SET value=2 WHERE key='seq'");
  catalog.close();
  assert.equal(
    (await engine.api(path + "/query-results/" + large.result.id + "/validity"))
      .current,
    false,
  );
  await engine.expectError(
    path + "/query-results/" + large.result.id + "/assets",
    "GET",
    undefined,
    "SOURCE_CHANGED",
  );
  await engine.expectError(
    path + "/selection/scope",
    "POST",
    {
      ...selectBody,
      expected_revision: largeSelection.revision,
      scope: resultScope(p.id, large.result.id),
    },
    "SOURCE_CHANGED",
  );
  assert.equal((await engine.api(path + "/selection")).count, 20000);
  checks.push(
    "20,000 stored objects build with exact counts; source changes invalidate live query use while frozen selection remains",
  );

  const cancelled = await createQuery(p.id, querySpec(lake.id), "取消验证");
  await engine.wait(
    path + "/query-results/" + cancelled.result.id,
    (r) => r.state === "running",
  );
  await engine.api(
    path + "/query-results/" + cancelled.result.id + "/cancel",
    "POST",
  );
  await sleep(100);
  assert.equal(
    (await engine.api(path + "/query-results/" + cancelled.result.id)).state,
    "cancelled",
  );
  await engine.expectError(
    path + "/query-results/" + cancelled.result.id + "/assets",
    "GET",
    undefined,
    "CANCELLED",
  );
  assert.equal(
    (
      await engine.api(
        path + "/query-results/" + cancelled.result.id + "/release",
        "POST",
      )
    ).state,
    "released",
  );
  checks.push(
    "cancelled partial builds are inaccessible and can be explicitly released",
  );

  const relocated = resolve(runDir, "重关联的索引");
  await mkdir(resolve(relocated, "indexes/gen-fixture"), { recursive: true });
  await copyFile(
    resolve(lake.lake, "CURRENT.json"),
    resolve(relocated, "CURRENT.json"),
  );
  await copyFile(
    lake.path,
    resolve(relocated, "indexes/gen-fixture/catalog.sqlite"),
  );
  const wrong = resolve(runDir, "错误的索引");
  await mkdir(wrong, { recursive: true });
  await jsonFile(resolve(wrong, "CURRENT.json"), {
    library_id: crypto.randomUUID(),
    index_version: 1,
    generation: "gen-fixture",
  });
  await engine.expectError(
    path + "/sources/" + lake.id + "/relink",
    "POST",
    { index_root: wrong, media_root: lake.lake },
    "SOURCE_ID_MISMATCH",
  );
  await engine.api(projectPath(p2.id) + "/open", "POST");
  await engine.api(projectPath(p2.id) + "/sources", "POST", {
    name: "共享位置",
    kind: "danbooru",
    index_root: lake.lake,
    media_root: lake.lake,
  });
  const moved = await engine.api(
    path + "/sources/" + lake.id + "/relink",
    "POST",
    { index_root: relocated, media_root: lake.lake },
  );
  assert.equal(moved.impact, "all_projects_in_this_app_registry");
  within(runDir, lake.path);
  await rename(lake.path, resolve(lake.path, "..", "old-catalog.sqlite"));
  assert.equal(
    (await engine.api(projectPath(p2.id) + "/sources")).items[0].available,
    true,
  );
  checks.push(
    "relink rejects a different lake identity and updates existing shared references",
  );

  const neighbors = [];
  for (const name of ["离线", "损坏", "未来版本", "其他引擎占用"]) {
    const p = await engine.api("/v1/projects", "POST", { name });
    neighbors.push(p);
    await engine.api(projectPath(p.id) + "/close", "POST");
  }
  await engine.api(path + "/close", "POST");
  await engine.api(projectPath(p2.id) + "/close", "POST");
  await engine.stop();
  const [offline, corrupt, future, busy] = neighbors;
  const dbPath = (p) => within(runDir, resolve(p.directory, "project.sqlite"));
  await rename(
    dbPath(offline),
    within(runDir, resolve(offline.directory, "offline.sqlite")),
  );
  await writeFile(dbPath(corrupt), "deliberately invalid fixture");
  const futureDb = new DatabaseSync(dbPath(future));
  futureDb.exec("PRAGMA user_version=999");
  futureDb.close();
  const beforeFuture = await readFile(dbPath(future));
  const registry = new DatabaseSync(resolve(engine.dataDir, "registry.sqlite"));
  for (const neighbor of neighbors)
    registry
      .prepare("UPDATE projects SET background_pending=1 WHERE id=?")
      .run(neighbor.id);
  registry.close();
  await other.start();
  await other.api("/v1/projects/open", "POST", { directory: busy.directory });
  await engine.start();
  const recents = await engine.api("/v1/projects");
  assert.equal(recents.items.length, 6);
  await engine.api(path + "/open", "POST");
  await engine.expectError(
    projectPath(busy.id) + "/open",
    "POST",
    undefined,
    "PROJECT_BUSY",
  );
  await engine.expectError(
    projectPath(future.id) + "/open",
    "POST",
    undefined,
    "FORMAT_UNSUPPORTED",
  );
  assert.deepEqual(await readFile(dbPath(future)), beforeFuture);
  const healthy = await engine.api(path + "/jobs", "POST", {
    idempotency_key: crypto.randomUUID(),
    scope: {
      project_id: p.id,
      target: { kind: "workset", collection_id: collection.id },
    },
  });
  await waitJob(p.id, healthy.id, (j) => j?.status === "succeeded");
  await engine.api(path + "/close", "POST");
  await engine.wait(
    "/v1/projects",
    (r) => r.items.find((item) => item.id === p.id)?.state === "closed",
  );
  await other.api("/v1/projects/open", "POST", { directory: p.directory });
  checks.push(
    "engine starts and healthy jobs finish beside offline, corrupt, busy and future projects; idle close releases the lease",
  );

  const report = {
    passed: true,
    checks,
    timings,
    boundaries:
      "Catalog fixture has no media bytes; native metadata predicates are covered by DuckDB fixtures. This is not a full-lake stress test.",
  };
  await jsonFile(resolve(runDir, "report.json"), report);
  console.log(
    JSON.stringify(
      { ...report, report: resolve(runDir, "report.json") },
      null,
      2,
    ),
  );
} catch (error) {
  await jsonFile(resolve(runDir, "failure.json"), {
    error: String(error),
    checks,
    timings,
  });
  throw error;
} finally {
  await engine.stop();
  await other.stop();
}
