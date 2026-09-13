import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { EngineFixture, sleep } from "./engine-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local/test-runs",
  "integration-ranking-v2-" + Date.now(),
);
await mkdir(run, { recursive: true });
await promisify(execFile)(
  "python",
  [
    resolve(root, "tooling/ranking-fixture.py"),
    resolve(run, "fixture"),
    "1024",
    "--v2-tags",
  ],
  { cwd: root, windowsHide: true },
);
const fixture = JSON.parse(
  await readFile(resolve(run, "fixture/fixture.json"), "utf8"),
);
const engine = new EngineFixture(root, resolve(run, "state"));
const checks = [];
let base, project, source, scope;
const hash = async (p) =>
  createHash("sha256")
    .update(await readFile(p))
    .digest("hex");
const sourcePaths = ["catalog.sqlite", "analysis.duckdb"].map((n) =>
  resolve(fixture.lake, "indexes/gen-ranking", n),
);
const sourceHashes = await Promise.all(sourcePaths.map(hash));
const parameters = (lambda = 0.3, mode = "rank") => ({
  ratings: ["g", "s", "q", "e"],
  mode,
  cohort_minimum: 8,
  artist_enabled: false,
  v2: {
    profiles: Object.fromEntries(
      ["g", "s", "q", "e"].map((r) => [
        r,
        { time_up: 0.3, time_down: 0, vote_weight: 0.08, era_weight: lambda },
      ]),
    ),
    feather_days: 90,
    minimum_effective: 2,
    comic_penalty: 0,
    keep_per_mille: 500,
    direct_rescue: 50,
    era_rescue: 50,
    audit: 30,
    eras: [],
    strict_era_targets: false,
  },
});
const submit = (p, delay = 0) =>
  engine.api(base + "/tools/jobs", "POST", {
    idempotency_key: crypto.randomUUID(),
    scope,
    delay_ms: delay,
    run: {
      operator_id: p.v2 ? "danbooru.metarecall_v2" : "danbooru.metarecall",
      operator_version: 1,
      parameters_version: 1,
      parameters: p,
    },
  });
async function finish(job, expected = "succeeded") {
  const response = await engine.wait(
    base + "/jobs",
    (v) =>
      v.items.some(
        (j) =>
          j.id === job.id &&
          ["succeeded", "failed", "cancelled"].includes(j.status),
      ),
    90000,
  );
  const current = response.items.find((j) => j.id === job.id);
  if (current.status !== expected) {
    const log = await readFile(
      resolve(
        project.directory,
        ".staging",
        job.id,
        "attempt-" + current.attempt + ".log",
      ),
      "utf8",
    ).catch(() => "");
    throw new Error(JSON.stringify(current) + "\n" + log.slice(-7000));
  }
  if (expected !== "succeeded") return { job: current };
  const artifact = await engine.api(base + "/jobs/" + job.id + "/ranking");
  const summary = await engine.api(
    base + "/artifacts/" + artifact.id + "/ranking",
  );
  return { job: current, artifact, summary };
}
async function rows(aid, filter = { order: "input" }) {
  const all = [];
  let cursor = null;
  do {
    const page = await engine.api(
      base + "/artifacts/" + aid + "/ranking/rows",
      "POST",
      { filter, cursor, limit: 96 },
    );
    all.push(...page.items);
    cursor = page.next_cursor;
  } while (cursor);
  return all;
}
async function browse(target, order, descending = false) {
  const all = [];
  let cursor = null;
  const deadline = Date.now() + 45000;
  while (true) {
    const page = await engine.api(base + "/ranking-browse/assets", "POST", {
      scope: target,
      order,
      descending,
      cursor,
      limit: 12,
    });
    if (page.preparing) {
      assert.ok(Date.now() < deadline, "index preparation finishes");
      await sleep(30);
      continue;
    }
    all.push(...page.items);
    cursor = page.next_cursor;
    if (!cursor) break;
  }
  return all;
}
process.on("exit", () => engine.child?.kill());
try {
  await engine.start();
  const catalog = await engine.api("/v1/operators");
  assert.ok(catalog.items.some((o) => o.id === "danbooru.metarecall"));
  assert.ok(catalog.items.some((o) => o.id === "danbooru.metarecall_v2"));
  checks.push("both schemes are independently registered");
  project = await engine.api("/v1/projects", "POST", {
    name: "MetaRecall v2 metadata integration",
  });
  base = "/v1/projects/" + project.id;
  source = await engine.api(base + "/sources", "POST", {
    name: "V2 fixture",
    kind: "danbooru",
    index_root: fixture.lake,
    media_root: fixture.lake,
  });
  scope = {
    project_id: project.id,
    target: { kind: "source", source_id: source.id, revision: source.revision },
  };
  const legacy = await finish(
    await submit({
      ratings: ["g", "s", "q", "e"],
      mode: "rank",
      cohort_minimum: 8,
    }),
  );
  const oldPaths = legacy.artifact.files.map((f) =>
    resolve(project.directory, f.path),
  );
  const oldHashes = await Promise.all(oldPaths.map(hash));
  const oldRows = await rows(legacy.artifact.id);
  assert.equal(legacy.artifact.schema_version, 1);
  assert.ok(oldRows.every((r) => !r.scores.v2 && r.input.tags == null));
  checks.push("v1 still publishes original-schema results without new fields");

  let first;
  for (const lambda of [0, 1]) {
    const result = await finish(await submit(parameters(lambda)));
    const values = await rows(result.artifact.id);
    assert.equal(result.artifact.schema_version, 2);
    assert.equal(result.summary.schema_version, 2);
    assert.ok(result.summary.ratings.every((r) => r.v2.metadata_only));
    for (const r of values.filter((r) => r.scores.eligibility === "eligible")) {
      assert.equal(typeof r.input.tags, "string");
      const v = r.scores.v2;
      assert.ok(v);
      assert.equal(
        r.scores.main_rank,
        lambda === 0 ? v.direct_rank : r.scores.rescue_rank,
      );
      assert.ok(Math.abs(r.scores.main_score - v.fused_score) < 1e-9);
    }
    if (lambda === 0) first = { ...result, values };
  }
  checks.push(
    "frozen tags survive API projection; fusion endpoints reproduce direct and era orders",
  );

  const p = parameters(0.4, "select");
  p.v2.comic_penalty = 3;
  p.v2.eras = [
    { from_year: 2024, through_year: 2024, bonus: 5, target_share: 500 },
  ];
  p.v2.strict_era_targets = true;
  const selected = await finish(await submit(p));
  const values = await rows(selected.artifact.id);
  const picked = (r) =>
    ["main", "rescue", "audit"].includes(r.scores.selected_route);
  for (const r of selected.summary.ratings) {
    const subset = values.filter(
      (v) => v.input.rating === r.rating && picked(v),
    );
    const budget = Math.floor(r.eligible / 2);
    assert.equal(subset.length, budget);
    assert.equal(
      subset.filter((v) => v.scores.v2.created_year === 2024).length,
      Math.ceil(budget / 2),
    );
    assert.equal(
      r.selected.reduce((a, b) => a + b, 0),
      budget,
    );
  }
  const eligible = values.filter((r) => r.scores.eligibility === "eligible");
  assert.ok(eligible.some((r) => r.scores.v2.type_penalty === 3));
  assert.ok(eligible.some((r) => r.scores.v2.layout_protected));
  for (const r of eligible) {
    const v = r.scores.v2;
    if (v.layout_protected) assert.equal(v.type_penalty, 0);
    assert.ok(
      Math.abs(
        r.scores.main_score - (v.fused_score - v.type_penalty + v.era_bonus),
      ) < 1e-8,
    );
  }
  checks.push(
    "exact per-rating budget and year targets; storyboard protection and one-time type penalty",
  );

  for (const order of ["direct", "fused", "rescue", "main"]) {
    const page = await rows(selected.artifact.id, {
      rating: "g",
      eligibility: "eligible",
      top: 7,
      order,
    });
    assert.equal(page.length, 7);
    const count = await engine.api(
      base + "/artifacts/" + selected.artifact.id + "/ranking/count",
      "POST",
      { filter: { rating: "g", eligibility: "eligible", top: 7, order } },
    );
    assert.equal(count.count, 7);
  }
  checks.push("all ranking orders honor top filters and bounded counts");

  const workset = await engine.api(
    base + "/artifacts/" + selected.artifact.id + "/ranking/worksets",
    "POST",
    {
      idempotency_key: crypto.randomUUID(),
      name: "v2 fixed direct prefix",
      filter: {
        rating: "g",
        eligibility: "eligible",
        order: "direct",
        top: 50,
      },
    },
  );
  const target = {
    project_id: project.id,
    target: { kind: "workset", collection_id: workset.id },
  };
  const members = eligible.filter(
    (r) => r.input.rating === "g" && r.scores.v2.direct_rank <= 50,
  );
  for (const order of ["main", "rescue", "direct", "fused", "input"]) {
    const position = (r) =>
      order === "input"
        ? r.input.ordinal
        : order === "direct"
          ? r.scores.v2.direct_rank
          : order === "fused"
            ? r.scores.v2.fused_rank
            : r.scores[order + "_rank"];
    const expected = [...members]
      .sort(
        (a, b) =>
          position(a) - position(b) || a.input.ordinal - b.input.ordinal,
      )
      .map((r) => r.input.ordinal);
    const forward = await browse(target, order);
    assert.deepEqual(
      forward.map((r) => r.ranking.ordinal),
      expected,
    );
    assert.ok(forward.every((r) => r.ranking.v2));
    const backward = await browse(target, order, true);
    assert.deepEqual(
      backward.map((r) => r.ranking.ordinal),
      expected.toReversed(),
    );
  }
  checks.push(
    "saved v2 worksets paginate in all five orders and both directions",
  );
  await engine.stop();
  await engine.start();
  await engine.api(base + "/open", "POST");
  assert.equal((await browse(target, "direct")).length, 50);
  checks.push("v2 materials and ordered scope indexes survive restart");

  const impossible = parameters(0.3, "select");
  impossible.v2.eras = [
    { from_year: 2199, through_year: 2199, bonus: 0, target_share: 1000 },
  ];
  impossible.v2.strict_era_targets = true;
  const failed = await finish(await submit(impossible), "failed");
  assert.match(failed.job.error ?? "", /年代/);
  checks.push(
    "infeasible strict year quotas fail explicitly instead of silently changing selection",
  );

  const delayed = await submit(parameters(0.3, "select"), 600);
  await engine.wait(
    base + "/jobs",
    (v) => v.items.some((j) => j.id === delayed.id && j.status === "running"),
    60000,
  );
  await engine.api(base + "/jobs/" + delayed.id + "/cancel", "POST");
  await engine.wait(
    base + "/jobs",
    (v) => v.items.some((j) => j.id === delayed.id && j.status === "cancelled"),
    10000,
  );
  await sleep(350);
  const retry = await engine.api(
    base + "/jobs/" + delayed.id + "/retry",
    "POST",
  );
  const recovered = await finish(retry);
  assert.equal(recovered.job.attempt, 2);
  assert.equal(recovered.summary.schema_version, 2);
  checks.push(
    "v2 cancellation and retry preserve typed input and resume the ranking",
  );

  assert.deepEqual(await Promise.all(oldPaths.map(hash)), oldHashes);
  assert.deepEqual(await rows(legacy.artifact.id), oldRows);
  assert.deepEqual(await Promise.all(sourcePaths.map(hash)), sourceHashes);
  const resources = await engine.api("/v1/resources");
  assert.equal(resources.previews.source_bytes, "0");
  assert.equal(resources.previews.pack_opens, 0);
  checks.push(
    "old results and source bytes are unchanged; metadata computation reads no images",
  );
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      { checks, run, summary: selected.summary, baseline: first.summary },
      null,
      2,
    ),
  );
  console.log(JSON.stringify({ checks: checks.length, run }));
} catch (error) {
  await writeFile(
    resolve(run, "failure.json"),
    JSON.stringify(
      { checks, run, error: String(error), stack: error.stack },
      null,
      2,
    ),
  );
  throw error;
} finally {
  await engine.stop();
}
