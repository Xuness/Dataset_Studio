import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { EngineFixture, sleep } from "./engine-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(root, ".local/test-runs/cache-inventory-" + Date.now());
await mkdir(run, { recursive: true });
const python = (name, ...args) =>
  promisify(execFile)(
    "python",
    [resolve(root, "tooling", name), resolve(run, "fixture"), ...args],
    { cwd: root, windowsHide: true },
  );
await python("ranking-fixture.py", "512");
const fixture = JSON.parse(
  await readFile(resolve(run, "fixture/fixture.json"), "utf8"),
);
const engine = new EngineFixture(root, resolve(run, "state"));
const checks = [];
let project, base, source, workset, artifact;
const scope = (id) => ({
  project_id: project.id,
  target: { kind: "query_result", result_id: id },
});
const inventory = () =>
  engine.api("/v1/cache/projects/" + project.id + "?limit=127");
const metrics = async () => (await engine.api("/v1/resources")).query_cache;
async function query(spec) {
  const q = await engine.api(base + "/query-results", "POST", { spec });
  const result = await engine.wait(
    base + "/query-results/" + q.id,
    (v) => ["ready", "failed"].includes(v.state),
    30000,
  );
  assert.equal(result.state, "ready", JSON.stringify(result));
  return result;
}
async function page(id, ready = false) {
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) {
    const value = await engine.api(base + "/ranking-browse/assets", "POST", {
      scope: scope(id),
      order: "main",
      limit: 48,
    });
    if (!value.preparing) return value;
    assert.equal(
      ready,
      false,
      "fixed indexes must remain ready across source updates",
    );
    await sleep(30);
  }
  throw new Error("ranking index timed out");
}
const ids = (p) => p.items.map((x) => x.key.asset_id);
const spec = (conditions) => ({
  version: 3,
  source_ids: [source.id],
  conditions,
  observation_rule: "current_post",
  order: "asset_key_asc",
  input_scope: {
    project_id: project.id,
    target: { kind: "workset", collection_id: workset.id },
  },
});
try {
  await engine.start();
  project = await engine.api("/v1/projects", "POST", {
    name: "缓存与每日更新",
  });
  base = "/v1/projects/" + project.id;
  source = await engine.api(base + "/sources", "POST", {
    name: "Fixture",
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
      operator_id: "danbooru.metarecall_v2",
      operator_version: 1,
      parameters_version: 1,
      parameters: {
        mode: "rank",
        cohort_minimum: 4,
        v2: { minimum_effective: 2 },
      },
    },
  });
  const jobs = await engine.wait(
    base + "/jobs",
    (v) =>
      v.items.some(
        (j) => j.id === job.id && ["succeeded", "failed"].includes(j.status),
      ),
    60000,
  );
  assert.equal(jobs.items.find((j) => j.id === job.id).status, "succeeded");
  artifact = await engine.api(base + "/jobs/" + job.id + "/ranking");
  workset = await engine.api(
    base + "/artifacts/" + artifact.id + "/ranking/worksets",
    "POST",
    {
      idempotency_key: crypto.randomUUID(),
      name: "固定排名",
      filter: { order: "main" },
    },
  );
  const fixed = [];
  for (const rating of ["g", "s", "q", "e"]) {
    const s = spec([
      {
        field: `project.${artifact.id}.rating`,
        operator: "in",
        value: { type: "text_list", value: [rating] },
      },
    ]);
    const result = await query(s);
    fixed.push({ spec: s, result, page: await page(result.id) });
  }
  const latestSpec = spec([
    {
      field: "rating",
      operator: "in",
      value: { type: "text_list", value: ["g"] },
    },
  ]);
  const latest = await query(latestSpec);
  const before = await metrics();
  const inv = await engine.wait(
    "/v1/cache/projects/" + project.id + "?limit=127",
    (v) =>
      v.members.length >= 6 &&
      v.members.every((m) => m.estimated_bytes !== null),
  );
  assert.equal(inv.ranked_indexes.length, 4);
  const input = inv.members.find((m) => !m.cached && m.reference_count > 0);
  assert.ok(input);
  assert.equal(input.can_release, false);
  assert.ok(input.references.some((r) => r.includes("固定排名")));
  assert.ok(inv.members.every((m) => m.estimated_bytes !== null));
  const projects = (await engine.api("/v1/cache/projects")).items;
  const overview = (await engine.api("/v1/settings")).storage;
  assert.equal(
    projects.reduce((n, p) => n + BigInt(p.member_bytes), 0n) +
      projects.reduce((n, p) => n + BigInt(p.ranked_index_bytes), 0n),
    BigInt(overview.project_member_bytes),
  );
  const pinfo = projects.find((p) => p.project_id === project.id);
  assert.ok(
    BigInt(pinfo.member_bytes) >=
      inv.members.reduce((n, m) => n + BigInt(m.estimated_bytes), 0n),
  );
  await engine.expectError(
    "/v1/cache/projects/" +
      project.id +
      "/members/" +
      input.result_id +
      "/release",
    "POST",
    undefined,
    "CACHE_IN_USE",
  );
  checks.push(
    "inventory reconciles project totals and lists protected fixed input with human-readable owners",
  );
  await python("ranking-fixture-update.py");
  assert.equal(
    (await engine.api(base + "/query-results/" + latest.id + "/validity"))
      .current,
    false,
  );
  for (const c of fixed) {
    assert.equal(
      (await engine.api(base + "/query-results/" + c.result.id + "/validity"))
        .current,
      true,
    );
    assert.deepEqual(ids(await page(c.result.id, true)), ids(c.page));
    const alias = await query(c.spec);
    assert.equal(alias.cache.mode, "reused");
    assert.equal(alias.count, c.result.count);
    assert.equal(alias.cache.evaluated_objects, 0);
    assert.deepEqual(ids(await page(alias.id, true)), ids(c.page));
    await engine.api(
      base + "/query-results/" + c.result.id + "/assets?limit=2",
    );
  }
  assert.equal(
    (await metrics()).ranked_index_builds,
    before.ranked_index_builds,
  );
  assert.equal((await metrics()).ranked_indexes, before.ranked_indexes);
  checks.push(
    "G/S/Q/E reuse old members and indexes after real image insertion and metadata refresh; latest-source queries remain stale",
  );
  const changed = await query(latestSpec);
  assert.equal(changed.cache.mode, "incremental");
  assert.ok(changed.cache.changed_members > 0);
  const changedPage = await page(changed.id);
  const afterChanged = await metrics();
  await python("ranking-fixture-update.py", "noop");
  const unchanged = await query(latestSpec);
  assert.equal(unchanged.cache.changed_members, 0);
  assert.deepEqual(ids(await page(unchanged.id, true)), ids(changedPage));
  assert.equal(
    (await metrics()).ranked_index_builds,
    afterChanged.ranked_index_builds,
  );
  checks.push(
    "latest metadata changes update members; no-op delta keeps member revision and ranked index",
  );

  const cacheRoot = "/v1/cache/projects/" + project.id;
  const leased = (await inventory()).ranked_indexes[0];
  const db = new DatabaseSync(resolve(project.directory, "project.sqlite"));
  const rankScope = new DatabaseSync(leased.path, { readOnly: true });
  const target = JSON.parse(
    rankScope.prepare("SELECT value FROM meta WHERE key='complete'").get()
      .value,
  ).scope;
  rankScope.close();
  const lease = crypto.randomUUID();
  await engine.api(base + "/ranking-browse/lease", "POST", {
    scope: target,
    lease_id: lease,
    release: false,
  });
  await engine.expectError(
    cacheRoot + "/ranked/" + leased.key + "/release",
    "POST",
    undefined,
    "CACHE_IN_USE",
  );
  await engine.api(base + "/ranking-browse/lease", "POST", {
    scope: target,
    lease_id: lease,
    release: true,
  });
  const other = await engine.api("/v1/projects", "POST", {
    name: "另一个项目",
  });
  await engine.expectError(
    "/v1/cache/projects/" + other.id + "/ranked/" + leased.key + "/release",
    "POST",
    undefined,
    "SCOPE_PROJECT_MISMATCH",
  );
  await engine.api(cacheRoot + "/ranked/" + leased.key + "/release", "POST");
  assert.ok(
    !(await inventory()).ranked_indexes.some((r) => r.key === leased.key),
  );
  await engine.api(base + "/close", "POST");
  const recency = (await engine.api("/v1/projects")).items.find(
    (p) => p.id === project.id,
  ).opened_at;
  assert.ok((await inventory()).members.length);
  await sleep(10500);
  const chosen = fixed[1].result.id;
  await engine.api(cacheRoot + "/members/" + chosen + "/retention", "PUT", {
    tier: "temporary",
    fixed: true,
  });
  assert.equal(
    (await inventory()).members.find((m) => m.result_id === chosen).fixed,
    true,
  );
  await engine.expectError(
    cacheRoot + "/members/" + chosen + "/release",
    "POST",
    undefined,
    "CACHE_IN_USE",
  );
  await engine.api(cacheRoot + "/members/" + chosen + "/retention", "PUT", {
    tier: "temporary",
    fixed: false,
  });
  await engine.api(cacheRoot + "/members/" + chosen + "/release", "POST");
  const closed = (await engine.api("/v1/projects")).items.find(
    (p) => p.id === project.id,
  );
  assert.equal(closed.state, "closed");
  assert.equal(closed.opened_at, recency);
  checks.push(
    "closed projects can inspect, unpin and release caches without opening or changing recency; ranked release respects leases and project identity",
  );

  // Reproduce the old stranded version-1 input state, without a large fixture.
  await engine.stop();
  db.exec("PRAGMA busy_timeout=3000; BEGIN IMMEDIATE");
  const orphan = crypto.randomUUID();
  const inputRow = db
    .prepare("SELECT * FROM query_results WHERE id=?")
    .get(input.result_id);
  db.prepare(
    "INSERT INTO query_results(id,spec_json,versions_json,status,count,created_at) VALUES(?,?,?,'ready',20000,'1')",
  ).run(orphan, inputRow.spec_json, inputRow.versions_json);
  db.prepare(
    "WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<20000) INSERT INTO query_member_data(family_id,source_id,asset_id,valid_from) SELECT ?,?,printf('%064x',x),1 FROM n",
  ).run(orphan, source.id);
  db.prepare(
    "UPDATE query_families SET cached=0,latest_count=0,stored_members=20000,latest_revision=1,latest_result_id=?,prune_pending=1 WHERE id=?",
  ).run(orphan, orphan);
  db.exec(
    "INSERT INTO meta VALUES('query_storage_revision','1') ON CONFLICT(key) DO UPDATE SET value=CAST(value AS INTEGER)+1; COMMIT",
  );
  db.close();
  // Emulate a catalog written before the new orphan audit field existed.
  const catalogPath = resolve(run, "state/query-cache-catalog.json");
  const catalog = JSON.parse(await readFile(catalogPath, "utf8"));
  delete catalog[project.id].unreferenced_members;
  await writeFile(catalogPath, JSON.stringify(catalog));
  await engine.start();
  await engine.wait(
    cacheRoot,
    (v) =>
      v.cleanups.some((t) => t.family_id === orphan && t.state === "completed"),
    60000,
  );
  const verify = new DatabaseSync(
    resolve(project.directory, "project.sqlite"),
    { readOnly: true },
  );
  assert.equal(
    verify
      .prepare("SELECT stored_members FROM query_families WHERE id=?")
      .get(orphan).stored_members,
    0,
  );
  assert.equal(
    verify.prepare("SELECT status FROM query_results WHERE id=?").get(orphan)
      .status,
    "released",
  );
  assert.equal(
    verify
      .prepare("SELECT count(*) AS n FROM result_members WHERE result_id=?")
      .get(input.result_id).n,
    input.members,
  );
  verify.close();
  checks.push(
    "legacy unreferenced version-1 input is found and reclaimed after restart with the project closed; referenced input is preserved",
  );
  await engine.api(base + "/open", "POST");
  for (const c of [fixed[2], fixed[3]]) {
    const alias = await query(c.spec);
    assert.equal(alias.cache.mode, "reused");
    // The explicitly released file may belong to either rating; other files survive.
    const found = (await inventory()).ranked_indexes.some((r) =>
      r.label.includes(c.spec.conditions[0].value.value[0].toUpperCase()),
    );
    await page(alias.id, found);
  }
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify({ checks }, null, 2),
  );
  console.log(`Cache inventory integration passed (${checks.length} checks).`);
} finally {
  await engine.stop();
}
