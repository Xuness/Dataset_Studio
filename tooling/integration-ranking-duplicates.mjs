import { pythonCommand } from "./platform.mjs";
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { URLSearchParams, fileURLToPath } from "node:url";
import { EngineFixture, sleep } from "./engine-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local/test-runs",
  "integration-ranking-duplicates-" + Date.now(),
);
await mkdir(run, { recursive: true });
await promisify(execFile)(
  pythonCommand(),
  [
    resolve(root, "tooling/ranking-fixture.py"),
    resolve(run, "fixture"),
    "128",
    "--duplicate-heat",
  ],
  { cwd: root, windowsHide: true },
);
const f = JSON.parse(
  await readFile(resolve(run, "fixture/fixture.json"), "utf8"),
);
const engine = new EngineFixture(root, resolve(run, "state"));
const checks = [];
const hash = async (p) =>
  createHash("sha256")
    .update(await readFile(p))
    .digest("hex");
const sourceFiles = ["catalog.sqlite", "analysis.duckdb"].map((n) =>
  resolve(f.lake, "indexes/gen-ranking", n),
);
const before = await Promise.all(sourceFiles.map(hash));
let base, project, source, scope;
async function rank(policy, v2 = true) {
  const job = await engine.api(base + "/tools/jobs", "POST", {
    idempotency_key: crypto.randomUUID(),
    scope,
    run: {
      operator_id: v2 ? "danbooru.metarecall_v2" : "danbooru.metarecall",
      operator_version: 1,
      parameters_version: 1,
      parameters: {
        ratings: ["g", "s", "q", "e"],
        mode: "rank",
        cohort_minimum: 4,
        duplicate_heat: policy,
        ...(v2 ? { v2: { minimum_effective: 2 } } : {}),
      },
    },
  });
  const jobs = await engine.wait(
    base + "/jobs",
    (v) =>
      v.items.some(
        (j) => j.id === job.id && ["succeeded", "failed"].includes(j.status),
      ),
    90000,
  );
  assert.equal(
    jobs.items.find((j) => j.id === job.id).status,
    "succeeded",
    JSON.stringify(jobs),
  );
  const artifact = await engine.api(base + "/jobs/" + job.id + "/ranking");
  const rows = [];
  let cursor = null;
  do {
    const page = await engine.api(
      base + "/artifacts/" + artifact.id + "/ranking/rows",
      "POST",
      { filter: { order: "input" }, cursor, limit: 96 },
    );
    rows.push(...page.items);
    cursor = page.next_cursor;
  } while (cursor);
  return {
    artifact,
    rows,
    subject: rows.find((r) => r.input.asset_id === f.objects[7].sha),
  };
}
async function query(field, ratings, extra = []) {
  const job = await engine.api(base + "/query-results", "POST", {
    spec: {
      version: 3,
      source_ids: [source.id],
      conditions: [
        { field, operator: "in", value: { type: "text_list", value: ratings } },
        ...extra,
      ],
      observation_rule: "current_post",
      order: "asset_key_asc",
      input_scope: scope,
    },
  });
  return engine.wait(
    base + "/query-results/" + job.id,
    (r) => ["ready", "failed"].includes(r.state),
    60000,
  );
}
async function browse(result, descending = false, order = "main") {
  const items = [];
  let cursor = null;
  const deadline = Date.now() + 45000;
  while (Date.now() < deadline) {
    const page = await engine.api(base + "/ranking-browse/assets", "POST", {
      scope: {
        project_id: project.id,
        target: { kind: "query_result", result_id: result.id },
      },
      descending,
      order,
      limit: 12,
      cursor,
    });
    if (page.preparing) {
      assert.ok(Date.now() < deadline);
      await sleep(40);
      continue;
    }
    items.push(...page.items);
    cursor = page.next_cursor;
    if (!cursor) return items;
  }
  throw new Error("Ranking browse timed out");
}
try {
  await engine.start();
  project = await engine.api("/v1/projects", "POST", {
    name: "Duplicate post ranking",
  });
  base = "/v1/projects/" + project.id;
  source = await engine.api(base + "/sources", "POST", {
    name: "Duplicate fixture",
    kind: "danbooru",
    index_root: f.lake,
    media_root: f.lake,
  });
  scope = {
    project_id: project.id,
    target: { kind: "source", source_id: source.id, revision: source.revision },
  };
  const legacy = await rank(null);
  assert.equal(legacy.subject.input.fav_count, "12");
  assert.equal(legacy.subject.input.evidence, undefined);
  const oldFiles = legacy.artifact.files.map((x) =>
    resolve(project.directory, x.path),
  );
  const oldHashes = await Promise.all(oldFiles.map(hash));
  checks.push("legacy input remains selectable and immutable");
  const high = await rank("highest");
  const sum = await rank("sum");
  const v1 = await rank("highest", false);
  for (const r of [high, sum, v1]) {
    assert.equal(r.subject.input.post_id, "4529147");
    assert.equal(r.subject.input.rating, "s");
    assert.equal(r.subject.input.evidence.post_count, 2);
    assert.equal(r.subject.input.evidence.metadata.fav_count, "12");
    assert.equal(r.subject.input.evidence.heat[0].post_id, "10007");
  }
  assert.equal(high.subject.input.fav_count, "50");
  assert.equal(high.subject.input.score, "32");
  assert.equal(v1.subject.input.fav_count, "50");
  assert.equal(sum.subject.input.fav_count, "62");
  assert.equal(sum.subject.input.score, "39");
  assert.equal(sum.subject.input.observed_at_us, null);
  assert.equal(sum.subject.input.time_quality, "aggregate");
  assert.ok(high.subject.scores.main_score > legacy.subject.scores.main_score);
  checks.push(
    "v1 and v2 use highest heat with latest rating; sum does not include historical 9999 votes",
  );
  const point = await engine.api(
    base +
      "/artifacts/" +
      high.artifact.id +
      "/ranking/rows/" +
      high.subject.input.ordinal,
  );
  assert.deepEqual(point, high.subject);
  checks.push(
    "single row inspection returns exact frozen numeric and record evidence",
  );
  const mbase = base + "/sources/" + source.id + "/assets/" + f.objects[7].sha;
  let overview;
  for (let i = 0; i < 200; i++) {
    const response = await engine.response(mbase + "/metadata");
    const value = await response.json();
    if (response.ok) {
      overview = value;
      break;
    }
    assert.equal(value.code, "SOURCE_INDEX_PREPARING", JSON.stringify(value));
    await sleep(100);
  }
  assert.ok(overview);
  const exact = await engine.api(
    mbase +
      "/records/" +
      point.input.record_id +
      "/observations?" +
      new URLSearchParams({
        version: overview.version.token,
        observation_id: point.input.observation_id,
      }),
  );
  assert.equal(exact.items.length, 1);
  assert.equal(exact.items[0].post_id, "4529147");
  const raw = await engine.api(
    mbase +
      "/records/" +
      point.input.record_id +
      "/observations/" +
      point.input.observation_id +
      "/raw?" +
      new URLSearchParams({ version: overview.version.token }),
  );
  assert.equal(JSON.parse(raw.json).fav_count, 12);
  checks.push(
    "metadata inspection can target the exact scoring observation while retaining aggregate evidence",
  );
  const workset = await engine.api(
    base + "/artifacts/" + high.artifact.id + "/ranking/worksets",
    "POST",
    {
      idempotency_key: crypto.randomUUID(),
      name: "Ranked duplicate fixture",
      filter: { eligibility: "eligible", order: "main" },
    },
  );
  scope = {
    project_id: project.id,
    target: { kind: "workset", collection_id: workset.id },
  };
  const old = await query("rating", ["g"]);
  assert.equal(old.state, "ready");
  const oldPage = await browse(old);
  assert.ok(oldPage.some((a) => a.ranking.post_id === "4529147"));
  const oldInfo = await engine.api(
    base + "/ranking-browse?result_id=" + old.id,
  );
  assert.equal(oldInfo.ranking.current_rating_filter, true);
  const field = "project." + high.artifact.id + ".rating";
  const g = await query(field, ["g"]);
  assert.equal(g.state, "ready", JSON.stringify(g));
  const expected = high.rows
    .filter(
      (r) => r.scores.eligibility === "eligible" && r.scores.rating === "g",
    )
    .map((r) => r.input.asset_id)
    .sort();
  for (const descending of [false, true])
    for (const order of ["main", "rescue", "direct", "fused", "input"]) {
      const page = await browse(g, descending, order);
      assert.ok(page.every((a) => a.ranking.rating === "g"));
      assert.deepEqual(page.map((a) => a.key.asset_id).sort(), expected);
    }
  const freshInfo = await engine.api(
    base + "/ranking-browse?result_id=" + g.id,
  );
  assert.equal(freshInfo.ranking.current_rating_filter, false);
  checks.push(
    "fixed rating query excludes S/Q leakage in every ordering and both directions",
  );
  const tagged = await query(
    field,
    ["s"],
    [
      {
        field: "tags",
        operator: "has_tag",
        value: { type: "text", value: "duplicate" },
      },
    ],
  );
  assert.equal(tagged.state, "ready", JSON.stringify(tagged));
  assert.equal(tagged.count, 1);
  assert.equal((await browse(tagged))[0].ranking.post_id, "4529147");
  checks.push(
    "fixed rating and current tag predicates compose on the same image without a source rating restriction",
  );
  const builds = (await engine.api("/v1/resources")).query_cache
    .ranked_index_builds;
  const reused = await query(field, ["g"]);
  assert.equal(reused.cache.mode, "reused");
  await browse(reused, true);
  assert.equal(
    (await engine.api("/v1/resources")).query_cache.ranked_index_builds,
    builds,
  );
  await engine.stop();
  await engine.start();
  await engine.api(base + "/open", "POST");
  await browse(reused);
  assert.equal(
    (await engine.api("/v1/resources")).query_cache.ranked_index_builds,
    0,
  );
  checks.push(
    "equivalent filtered scopes reuse their index across aliases and engine restart",
  );
  assert.deepEqual(await Promise.all(oldFiles.map(hash)), oldHashes);
  assert.deepEqual(await Promise.all(sourceFiles.map(hash)), before);
  checks.push(
    "source lake and previously published ranking artifacts are unchanged",
  );
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      {
        checks,
        subject: {
          legacy: legacy.subject,
          highest: high.subject,
          sum: sum.subject,
        },
        counts: { old: old.count, fixed: g.count },
      },
      null,
      2,
    ),
  );
  process.stdout.write(JSON.stringify({ passed: checks.length, run }) + "\n");
} catch (error) {
  await writeFile(
    resolve(run, "failure.json"),
    JSON.stringify(
      { checks, error: String(error), stack: error.stack },
      null,
      2,
    ),
  );
  throw error;
} finally {
  await engine.stop();
}
