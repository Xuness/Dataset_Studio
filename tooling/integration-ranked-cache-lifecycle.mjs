import { pythonCommand } from "./platform.mjs";
// Regression for real query aliases and asynchronous shared-member cleanup.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { performance } from "node:perf_hooks";
import { EngineFixture, sleep } from "./engine-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local/test-runs/integration-ranked-cache-lifecycle-" + Date.now(),
);
await mkdir(run, { recursive: true });
await promisify(execFile)(
  pythonCommand(),
  [
    resolve(root, "tooling/ranking-fixture.py"),
    resolve(run, "fixture"),
    "2048",
  ],
  { cwd: root, windowsHide: true },
);
const fixture = JSON.parse(
  await readFile(resolve(run, "fixture/fixture.json"), "utf8"),
);
const engine = new EngineFixture(root, resolve(run, "state"));
const checks = [],
  timings = [];
let project, source, base;
const metrics = async () => (await engine.api("/v1/resources")).query_cache;
const scope = (id) => ({
  project_id: project.id,
  target: { kind: "workset", collection_id: id },
});
const resultScope = (id) => ({
  project_id: project.id,
  target: { kind: "query_result", result_id: id },
});
async function page(target, order = "main", descending = false, ready = false) {
  const deadline = Date.now() + 30000;
  while (true) {
    const value = await engine.api(base + "/ranking-browse/assets", "POST", {
      scope: target,
      order,
      descending,
      limit: 48,
    });
    if (!value.preparing) return value;
    assert.equal(
      ready,
      false,
      "prepared membership must never rebuild for another alias",
    );
    assert.ok(Date.now() < deadline);
    await sleep(25);
  }
}
const ids = (p) => p.items.map((r) => r.key.asset_id);
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
try {
  await engine.start();
  project = await engine.api("/v1/projects", "POST", {
    name: "Rank cache lifecycle",
  });
  base = "/v1/projects/" + project.id;
  source = await engine.api(base + "/sources", "POST", {
    name: "Fixture",
    kind: "danbooru",
    index_root: fixture.lake,
    media_root: fixture.lake,
  });
  const cases = [];
  for (const version of [1, 2]) {
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
        operator_id:
          version === 1 ? "danbooru.metarecall" : "danbooru.metarecall_v2",
        operator_version: 1,
        parameters_version: 1,
        parameters: {
          mode: "rank",
          cohort_minimum: 8,
          ...(version === 2 ? { v2: { minimum_effective: 8 } } : {}),
        },
      },
    });
    const jobs = await engine.wait(
      base + "/jobs",
      (v) =>
        v.items.some(
          (j) => j.id === job.id && ["succeeded", "failed"].includes(j.status),
        ),
      30000,
    );
    assert.equal(jobs.items.find((j) => j.id === job.id).status, "succeeded");
    const artifact = await engine.api(base + "/jobs/" + job.id + "/ranking");
    const workset = await engine.api(
      base + "/artifacts/" + artifact.id + "/ranking/worksets",
      "POST",
      {
        idempotency_key: crypto.randomUUID(),
        name: "v" + version,
        filter: { eligibility: "eligible", order: "main" },
      },
    );
    for (const rating of ["g", "e"]) {
      const spec = {
        version: 3,
        source_ids: [source.id],
        conditions: [
          {
            field: "rating",
            operator: "in",
            value: { type: "text_list", value: [rating] },
          },
        ],
        observation_rule: "current_post",
        order: "asset_key_asc",
        input_scope: scope(workset.id),
      };
      const first = await query(spec);
      const target = resultScope(first.id);
      const expected = await page(target);
      const before = await metrics();
      const view = (
        await engine.api(base + "/ranking-browse?result_id=" + first.id)
      ).ranking.view_key;
      for (let i = 0; i < 3; i++) {
        const alias = await query(spec);
        assert.notEqual(alias.id, first.id);
        assert.equal(alias.cache.mode, "reused");
        assert.equal(
          (await engine.api(base + "/ranking-browse?result_id=" + alias.id))
            .ranking.view_key,
          view,
        );
        assert.deepEqual(
          ids(await page(resultScope(alias.id), "main", false, true)),
          ids(expected),
        );
        await page(resultScope(alias.id), "rescue", true, true);
      }
      const after = await metrics();
      assert.equal(after.ranked_index_builds, before.ranked_index_builds);
      assert.equal(after.ranked_indexes, before.ranked_indexes);
      cases.push({ spec, first, expected, target, artifact: artifact.id });
    }
  }
  checks.push(
    "V1/V2 G/E aliases share one index in both directions without builds or duplicate files",
  );
  for (const c of cases)
    assert.deepEqual(
      ids(await page(c.target, "main", false, true)),
      ids(c.expected),
    );
  await engine.api(base + "/close", "POST");
  await engine.api(base + "/open", "POST");
  await engine.stop();
  await engine.start();
  await engine.api(base + "/open", "POST");
  for (const c of cases) {
    const alias = await query(c.spec);
    assert.deepEqual(
      ids(await page(resultScope(alias.id), "main", false, true)),
      ids(c.expected),
    );
  }
  assert.equal((await metrics()).ranked_index_builds, 0);
  checks.push(
    "project reopen and engine restart reuse equivalent newly-created query aliases",
  );

  // Test-only family in this isolated project: 200k keys, no image IO.
  const trash = await query({
    version: 3,
    source_ids: [source.id],
    conditions: [
      {
        field: "tags",
        operator: "has_all_tags",
        value: { type: "text_list", value: ["cleanup_probe"] },
      },
    ],
    observation_rule: "current_post",
    order: "asset_key_asc",
  });
  const db = new DatabaseSync(resolve(project.directory, "project.sqlite"));
  const family = db
    .prepare("SELECT family_id FROM query_results WHERE id=?")
    .get(trash.id).family_id;
  db.exec("BEGIN IMMEDIATE");
  db.prepare("DELETE FROM query_member_data WHERE family_id=?").run(family);
  db.prepare(
    "WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<200000) INSERT INTO query_member_data(family_id,source_id,asset_id,valid_from,post_id) SELECT ?,?,printf('%064x',x),1,x FROM n",
  ).run(family, source.id);
  db.prepare(
    "UPDATE query_families SET stored_members=200000,latest_count=200000,latest_revision=1 WHERE id=?",
  ).run(family);
  db.prepare(
    "UPDATE query_results SET count=200000,member_revision=1 WHERE family_id=?",
  ).run(family);
  db.exec(
    "INSERT INTO meta VALUES('query_storage_revision','1') ON CONFLICT(key) DO UPDATE SET value=CAST(value AS INTEGER)+1; COMMIT",
  );
  db.close();
  await sleep(10500); // Let the new-result handoff grace expire, like an unused UI entry.
  const started = performance.now();
  assert.equal(
    (
      await engine.api(
        base + "/query-results/" + trash.id + "/cache-release",
        "POST",
      )
    ).state,
    "released",
  );
  const requestMs = performance.now() - started;
  timings.push({ kind: "queue-200k-cleanup", ms: requestMs });
  assert.ok(requestMs < 1500, "request must return before deleting the family");
  const active = await engine.wait(
    base + "/cache-entries",
    (v) =>
      v.cleanups.some(
        (t) =>
          t.family_id === family && t.state === "deleting" && t.removed > 0,
      ),
    30000,
  );
  const progress = active.cleanups.find((t) => t.family_id === family);
  assert.ok(progress.removed < 200000);
  const readStarted = performance.now();
  await page(cases[0].target, "main", false, true);
  timings.push({
    kind: "ranking-during-cleanup",
    ms: performance.now() - readStarted,
  });
  await engine.stop();
  await engine.start();
  // Leave the project closed: the persisted cleanup catalog must resume it.
  await engine.wait("/v1/settings", (v) => !v.storage.cleanup_pending, 30000);
  await engine.api(base + "/open", "POST");
  const restored = await engine.api(base + "/cache-entries");
  assert.ok(
    restored.cleanups.find((t) => t.family_id === family).removed >=
      progress.removed,
  );
  const done = await engine.wait(
    base + "/cache-entries",
    (v) =>
      v.cleanups.some((t) => t.family_id === family && t.state === "completed"),
    30000,
  );
  assert.equal(
    done.cleanups.find((t) => t.family_id === family).removed,
    200000,
  );
  for (const c of cases)
    assert.deepEqual(
      ids(await page(c.target, "main", false, true)),
      ids(c.expected),
    );
  checks.push(
    "200k shared members clean asynchronously, expose progress, resume with the project closed after restart and preserve ranked reads",
  );

  // Same count is not a member identity: retain a referenced old revision while
  // replacing one member in a new revision of the same family.
  const original = cases[0];
  const changed = await query(original.spec);
  const lease = crypto.randomUUID();
  await engine.api(
    base + "/query-results/" + original.first.id + "/leases/" + lease,
    "POST",
  );
  const beforeChange = await metrics();
  const revisionDb = new DatabaseSync(
    resolve(project.directory, "project.sqlite"),
  );
  const rankingFiles = JSON.parse(
    revisionDb
      .prepare("SELECT files_json FROM artifacts WHERE id=?")
      .get(original.artifact).files_json,
  );
  for (const [alias, suffix] of [
    ["oracle_scores", ".ranking.sqlite"],
    ["oracle_input", ".ranking-input.sqlite"],
  ]) {
    const file = rankingFiles.find((f) => f.path.endsWith(suffix));
    assert.ok(file);
    revisionDb
      .prepare(`ATTACH DATABASE ? AS ${alias}`)
      .run(
        pathToFileURL(resolve(project.directory, file.path)).href + "?mode=ro",
      );
  }
  revisionDb.exec("PRAGMA busy_timeout=3000; BEGIN IMMEDIATE");
  const parent = revisionDb
    .prepare("SELECT family_id FROM query_results WHERE id=?")
    .get(changed.id).family_id;
  const replacement = revisionDb
    .prepare(
      "SELECT i.source_id,lower(hex(i.asset_id)) AS asset_id FROM oracle_scores.scores s JOIN oracle_input.input_rows i ON i.ordinal=s.ordinal WHERE s.eligibility='eligible' AND NOT EXISTS(SELECT 1 FROM result_members r WHERE r.result_id=? AND r.source_id=i.source_id AND r.asset_id=lower(hex(i.asset_id))) LIMIT 1",
    )
    .get(original.first.id);
  revisionDb
    .prepare(
      "UPDATE query_member_data SET valid_until=2 WHERE family_id=? AND asset_id=? AND valid_until IS NULL",
    )
    .run(parent, original.expected.items[0].key.asset_id);
  revisionDb
    .prepare(
      "INSERT INTO query_member_data(family_id,source_id,asset_id,valid_from) VALUES(?,?,?,2)",
    )
    .run(parent, replacement.source_id, replacement.asset_id);
  revisionDb
    .prepare(
      "UPDATE query_results SET member_revision=2,cache_mode='incremental' WHERE id=?",
    )
    .run(changed.id);
  revisionDb
    .prepare(
      "UPDATE query_families SET latest_revision=2,latest_result_id=?,stored_members=stored_members+1 WHERE id=?",
    )
    .run(changed.id, parent);
  revisionDb.exec(
    "UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='query_storage_revision'; COMMIT",
  );
  revisionDb.close();
  const changedPage = await page(resultScope(changed.id));
  assert.notDeepEqual(ids(changedPage), ids(original.expected));
  assert.deepEqual(
    ids(await page(original.target, "main", false, true)),
    ids(original.expected),
  );
  assert.equal(
    (await metrics()).ranked_index_builds,
    beforeChange.ranked_index_builds + 1,
  );
  checks.push(
    "a changed member revision with the same count builds its own index and retains the protected old snapshot",
  );

  // Change only fixture source version. Immutable ranking reads must still work.
  const catalog = new DatabaseSync(
    resolve(fixture.lake, "indexes/gen-ranking/catalog.sqlite"),
  );
  catalog.exec("UPDATE state SET value=value+1 WHERE key='seq'");
  catalog.close();
  for (const c of cases)
    assert.deepEqual(
      ids(await page(c.target, "main", false, true)),
      ids(c.expected),
    );
  checks.push(
    "historical fixed ranking remains readable after source watermark changes",
  );
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify({ checks, timings }, null, 2),
  );
  process.stdout.write(
    `Rank cache lifecycle integration passed (${checks.length} checks).\n`,
  );
} catch (error) {
  await writeFile(
    resolve(run, "failure.json"),
    JSON.stringify({ checks, error: String(error.stack ?? error) }, null, 2),
  );
  throw error;
} finally {
  await engine.stop();
}
