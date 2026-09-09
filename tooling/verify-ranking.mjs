// Opt-in bounded real-lake trial. Reads index metadata; no image payloads are requested.
import assert from "node:assert/strict";
import { mkdir, readFile, writeFile, stat } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { EngineFixture, sleep } from "./engine-fixture.mjs";

const arg = (name) => process.argv[process.argv.indexOf(name) + 1];
for (const name of ["--index-root", "--media-root"])
  if (!process.argv.includes(name)) throw new Error("Required: " + name);
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local",
  "test-runs",
  "ranking-verification-" + Date.now(),
);
await mkdir(run, { recursive: true });
const engine = new EngineFixture(root, resolve(run, "state"));
const stages = [],
  checks = [];
const json = (name, value) =>
  writeFile(
    resolve(run, name),
    JSON.stringify(
      value,
      (_, v) => (typeof v === "bigint" ? v.toString() : v),
      2,
    ),
  );
const pointer = resolve(arg("--index-root"), "CURRENT.json");
const current = JSON.parse(await readFile(pointer, "utf8"));
const sourceFiles = ["catalog.sqlite", "analysis.duckdb"].map((f) =>
  resolve(arg("--index-root"), "indexes", current.generation, f),
);
async function sourceState() {
  return {
    current: await readFile(pointer, "utf8"),
    files: await Promise.all(
      sourceFiles.map(async (path) => {
        const s = await stat(path);
        return { path, bytes: s.size, mtimeMs: s.mtimeMs };
      }),
    ),
  };
}
const before = await sourceState();
const started = Date.now();
process.on("exit", () => engine.child?.kill());
try {
  await engine.start();
  const project = await engine.api("/v1/projects", "POST", {
    name: "MetaRecall 真实元数据有界验收",
  });
  const base = "/v1/projects/" + project.id;
  const source = await engine.api(base + "/sources", "POST", {
    name: "Danbooru 只读试点",
    kind: "danbooru",
    index_root: arg("--index-root"),
    media_root: arg("--media-root"),
  });
  const spec = {
    version: 3,
    source_ids: [source.id],
    observation_rule: "current_post",
    order: "asset_key_asc",
    conditions: [
      { field: "rating", operator: "eq", value: { type: "text", value: "g" } },
      {
        field: "tags",
        operator: "has_tag",
        value: { type: "text", value: "solo" },
      },
      {
        field: "post.id",
        operator: "gte",
        value: { type: "integer", value: "6000000" },
      },
      {
        field: "post.id",
        operator: "lte",
        value: { type: "integer", value: "6500000" },
      },
    ],
  };
  const queued = await engine.api(base + "/query-results", "POST", { spec });
  const queryStarted = Date.now();
  const result = await engine.wait(
    base + "/query-results/" + queued.id,
    (r) => ["ready", "failed", "cancelled"].includes(r.state),
    900000,
  );
  assert.equal(result.state, "ready", JSON.stringify(result));
  assert.ok(
    result.count > 5000 && result.count <= 500001,
    "Trial must remain bounded and exercise populated time cohorts",
  );
  const queryMs = Date.now() - queryStarted;
  console.log(
    JSON.stringify({ phase: "query-ready", count: result.count, ms: queryMs }),
  );
  const rankStarted = Date.now();
  const job = await engine.api(base + "/tools/jobs", "POST", {
    idempotency_key: crypto.randomUUID(),
    scope: {
      project_id: project.id,
      target: { kind: "query_result", result_id: result.id },
    },
    run: {
      operator_id: "danbooru.metarecall",
      operator_version: 1,
      parameters_version: 1,
      parameters: {
        ratings: ["g"],
        mode: "select",
        seed: "real-trial-20260909",
        cohort_minimum: 5000,
        artist_enabled: false,
        minimum_stored_side: null,
      },
    },
  });
  let finished;
  for (let n = 0; n < 1800; n++) {
    const currentJob = (await engine.api(base + "/jobs")).items.find(
      (j) => j.id === job.id,
    );
    const phase = currentJob.status + "/" + (currentJob.stage?.name ?? "");
    if (stages.at(-1)?.phase !== phase) {
      stages.push({
        phase,
        ms: Date.now() - rankStarted,
        progress: currentJob.stage,
      });
      console.log(JSON.stringify(stages.at(-1)));
    }
    if (["succeeded", "failed", "cancelled"].includes(currentJob.status)) {
      finished = currentJob;
      break;
    }
    await sleep(1000);
  }
  assert.equal(finished?.status, "succeeded", JSON.stringify(finished));
  const rankMs = Date.now() - rankStarted;
  const artifact = await engine.api(base + "/jobs/" + job.id + "/ranking");
  const summary = await engine.api(
    base + "/artifacts/" + artifact.id + "/ranking",
  );
  const evidence = await engine.api(
    base + "/artifacts/" + artifact.id + "/ranking/evidence",
  );
  const resources = await engine.api("/v1/resources");
  assert.equal(summary.input_count, result.count);
  assert.equal(resources.previews.source_bytes, "0");
  assert.equal(resources.previews.pack_opens, 0);
  assert.equal(resources.previews.generated, 0);
  checks.push(
    "bounded current G + solo query freezes its actual members and completes without image reads or preview generation",
  );

  const scores = new DatabaseSync(
    resolve(project.directory, "artifacts", job.id + ".ranking.sqlite"),
    { readOnly: true },
  );
  const inputs = new DatabaseSync(
    resolve(project.directory, "artifacts", job.id + ".ranking-input.sqlite"),
    { readOnly: true },
  );
  try {
    const values = inputs
      .prepare(
        "SELECT ordinal,CAST(fav_count AS TEXT) AS fav,CAST(up_score AS TEXT) AS up FROM input_rows",
      )
      .all();
    const indexed = new Map(
      scores
        .prepare(
          "SELECT ordinal,g,main_score,rescue_score,main_rank,rescue_rank,eligibility,selected_route FROM scores",
        )
        .all()
        .map((r) => [r.ordinal, r]),
    );
    const key = (value) =>
      value == null || BigInt(value) < 0n ? null : BigInt(value) + 1n;
    const heat = values
      .filter((r) => indexed.get(r.ordinal).eligibility === "eligible")
      .map((r) => {
        const f = key(r.fav),
          u = key(r.up);
        return {
          ordinal: r.ordinal,
          key:
            f == null ? (u == null ? null : u * u) : u == null ? f * f : f * u,
        };
      });
    const valid = heat
      .filter((r) => r.key != null)
      .sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
    let compared = 0;
    for (let i = 0; i < valid.length;) {
      let j = i + 1;
      while (j < valid.length && valid[j].key === valid[i].key) j++;
      const percentile = (i + j) / (2 * valid.length);
      for (let k = i; k < j; k++) {
        assert.ok(
          Math.abs(indexed.get(valid[k].ordinal).g - percentile) < 1e-12,
        );
        compared++;
      }
      i = j;
    }
    for (const r of heat.filter((r) => r.key == null))
      assert.equal(indexed.get(r.ordinal).g, 0.5);
    for (const [rank, score] of [
      ["main_rank", "main_score"],
      ["rescue_rank", "rescue_score"],
    ]) {
      const ordered = [...indexed.values()]
        .filter((r) => r.eligibility === "eligible")
        .sort((a, b) => a[rank] - b[rank]);
      for (let i = 0; i < ordered.length; i++) {
        assert.equal(ordered[i][rank], i + 1);
        if (i) assert.ok(ordered[i - 1][score] >= ordered[i][score]);
      }
    }
    const distribution = scores
      .prepare(
        "SELECT selected_route,count(*) AS count FROM scores WHERE eligibility='eligible' GROUP BY selected_route",
      )
      .all();
    for (const group of summary.ratings) {
      assert.deepEqual(group.selected, group.quotas);
      assert.equal(
        group.selected.reduce((a, b) => a + b, 0),
        Math.floor((group.eligible * 333) / 1000),
      );
    }
    checks.push(
      `independent BigInt oracle matches G for all ${compared} valid-heat rows; every main/rescue rank is ordered and every quota is exact`,
    );
    await json("oracle.json", {
      compared,
      distribution,
      inputRows: values.length,
    });
  } finally {
    scores.close();
    inputs.close();
  }
  const after = await sourceState();
  assert.deepEqual(after, before);
  checks.push(
    "source generation pointer and both index file sizes and modification timestamps remain unchanged",
  );
  await json("report.json", {
    passed: true,
    checks,
    scope: spec,
    project,
    source,
    result,
    artifact,
    summary,
    evidence,
    resources,
    sourceState: before,
    timings: { queryMs, rankMs, totalMs: Date.now() - started },
    stages,
    run,
  });
  console.log(
    JSON.stringify({
      passed: true,
      input: summary.input_count,
      eligible: summary.eligible_count,
      timeUsed: summary.ratings.map((r) => r.time_used),
      queryMs,
      rankMs,
      run,
    }),
  );
} finally {
  await engine.stop();
}
