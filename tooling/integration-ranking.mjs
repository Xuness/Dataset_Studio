import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile, rename } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { EngineFixture, sleep, within } from "./engine-fixture.mjs";
const execute = promisify(execFile);
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local",
  "test-runs",
  "integration-ranking-" + Date.now(),
);
await mkdir(run, { recursive: true });
const fixtureRoot = process.argv[2]
  ? resolve(process.argv[2])
  : resolve(run, "fixture");
if (!process.argv[2])
  await execute(
    "python",
    [resolve(root, "tooling/ranking-fixture.py"), fixtureRoot, "1024"],
    { cwd: root, windowsHide: true },
  );
const fixture = JSON.parse(
  await readFile(resolve(fixtureRoot, "fixture.json"), "utf8"),
);
const engine = new EngineFixture(root, resolve(run, "state"));
const checks = [];
const digest = async (path) =>
  createHash("sha256")
    .update(await readFile(path))
    .digest("hex");
const sourceFiles = ["catalog.sqlite", "analysis.duckdb"].map((n) =>
  resolve(fixture.lake, "indexes/gen-ranking", n),
);
const before = await Promise.all(sourceFiles.map(digest));
const parameters = {
  ratings: ["g", "s", "q", "e"],
  mode: "select",
  quotas: [280, 43, 10],
  seed: "integration-ranking-v1",
  cohort_minimum: 8,
  artist_enabled: true,
};
let base, projectId, source;
const scope = (target) => ({ project_id: projectId, target });
async function submit(input, p = parameters, delay = 0) {
  return engine.api(base + "/tools/jobs", "POST", {
    idempotency_key: crypto.randomUUID(),
    scope: input,
    delay_ms: delay,
    run: {
      operator_id: "danbooru.metarecall",
      operator_version: 1,
      parameters_version: 1,
      parameters: p,
    },
  });
}
async function complete(job, timeout = 90000) {
  const jobs = await engine.wait(
    base + "/jobs",
    (r) =>
      r.items.some(
        (j) =>
          j.id === job.id &&
          ["succeeded", "failed", "cancelled"].includes(j.status),
      ),
    timeout,
  );
  const current = jobs.items.find((j) => j.id === job.id);
  if (current.status !== "succeeded") {
    const logs = await readFile(
      resolve(
        currentProjectDirectory,
        ".staging",
        job.id,
        `attempt-${current.attempt}.log`,
      ),
      "utf8",
    ).catch(() => "");
    throw new Error(JSON.stringify(current) + "\n" + logs.slice(-6000));
  }
  const artifact = await engine.api(base + "/jobs/" + job.id + "/ranking");
  const summary = await engine.api(
    base + "/artifacts/" + artifact.id + "/ranking",
  );
  return { job: current, artifact, summary };
}
async function rows(aid, filter = {}) {
  let cursor = null;
  const items = [];
  do {
    const page = await engine.api(
      base + "/artifacts/" + aid + "/ranking/rows",
      "POST",
      { filter, cursor, limit: 97 },
    );
    assert.ok(page.items.length <= 97);
    items.push(...page.items);
    cursor = page.next_cursor;
  } while (cursor);
  return items;
}
async function query(conditions, rule = "current_post") {
  const definition = await engine.api(base + "/queries", "POST", {
    name: "排名输入",
    spec: {
      version: 3,
      source_ids: [source.id],
      conditions,
      observation_rule: rule,
      order: "asset_key_asc",
    },
  });
  const queued = await engine.api(
    base + "/queries/" + definition.id + "/results",
    "POST",
    { expected_revision: definition.revision },
  );
  const result = await engine.wait(
    base + "/query-results/" + queued.id,
    (r) => ["ready", "failed", "cancelled"].includes(r.state),
    60000,
  );
  assert.equal(result.state, "ready", JSON.stringify(result));
  return result;
}
let currentProjectDirectory;
process.on("exit", () => engine.child?.kill());
try {
  await engine.start();
  const project = await engine.api("/v1/projects", "POST", {
    name: "元数据排名集成验证",
  });
  projectId = project.id;
  currentProjectDirectory = project.directory;
  base = "/v1/projects/" + projectId;
  source = await engine.api(base + "/sources", "POST", {
    kind: "danbooru",
    name: "排名夹具",
    index_root: fixture.lake,
    media_root: fixture.lake,
  });
  const full = scope({
    kind: "source",
    source_id: source.id,
    revision: source.revision,
  });
  const original = await complete(await submit(full));
  const all = await rows(original.artifact.id, { order: "input" });
  assert.equal(all.length, fixture.objects.length);
  assert.equal(original.summary.input_count, fixture.objects.length);
  assert.equal(original.artifact.files.length, 3);
  const image = (n) =>
    all.find((r) => r.input.asset_id === fixture.objects[n].sha);
  assert.equal(image(1).input.post_id, "20001");
  assert.equal(image(1).input.fav_count, "999");
  assert.equal(image(1).input.rating_conflict, true);
  assert.equal(image(3).scores.eligibility, "metadata_unavailable");
  assert.equal(image(15).scores.eligibility, "metadata_unavailable");
  assert.equal(image(14).scores.eligibility, "rating_unknown");
  assert.equal(image(4).scores.g, 0.5);
  assert.equal(image(5).scores.time_reason, "time_invalid");
  assert.ok(
    !image(6).scores.missing_flags.includes("observation_time_invalid"),
  );
  assert.equal(image(9).scores.t, 0);
  assert.equal(image(10).scores.t, 0.08);
  assert.equal(image(11).input.dimension_basis, "not_requested");
  for (const r of original.summary.ratings) {
    assert.deepEqual(r.selected, r.quotas);
    assert.equal(
      r.selected.reduce((a, b) => a + b, 0),
      Math.floor((r.eligible * 333) / 1000),
    );
    const group = all.filter(
      (v) =>
        v.scores.eligibility === "eligible" && v.scores.rating === r.rating,
    );
    assert.equal(group.length, r.eligible);
    assert.equal(
      new Set(group.map((v) => v.scores.main_rank)).size,
      group.length,
    );
  }
  checks.push(
    "source range freezes one complete representative, keeps unknowns, validates token and date semantics, and assigns exact quotas",
  );
  const rerun = await complete(await submit(full));
  const replay = await rows(rerun.artifact.id, { order: "input" });
  assert.deepEqual(all, replay);
  checks.push(
    "identical snapshot, parameters and seed reproduce every score, rank, and channel",
  );

  const tagged = await query([
    { field: "rating", operator: "eq", value: { type: "text", value: "q" } },
    {
      field: "tags",
      operator: "has_tag",
      value: { type: "text", value: "alpha" },
    },
  ]);
  const taggedScope = scope({ kind: "query_result", result_id: tagged.id });
  const taggedRun = await complete(await submit(taggedScope));
  const taggedRows = await rows(taggedRun.artifact.id);
  assert.equal(
    taggedRows.find((r) => r.input.asset_id === fixture.objects[2].sha).input
      .post_id,
    "10002",
  );
  assert.ok(taggedRows.every((r) => r.input.rating === "q"));
  const evidence = await engine.api(
    base + "/artifacts/" + taggedRun.artifact.id + "/ranking/evidence",
  );
  assert.ok(evidence.bases.some((b) => b.result_id === tagged.id));
  checks.push(
    "query criteria constrain the representative; a newer out-of-scope post cannot change its rating or tags",
  );

  const history = await query(
    [
      {
        field: "tags",
        operator: "has_tag",
        value: { type: "text", value: "alpha" },
      },
    ],
    "any_observation",
  );
  const historyRun = await complete(
    await submit(scope({ kind: "query_result", result_id: history.id }), {
      ...parameters,
      mode: "rank",
    }),
  );
  const historyRows = await rows(historyRun.artifact.id);
  assert.equal(
    historyRows.find((r) => r.input.asset_id === fixture.objects[3].sha).scores
      .eligibility,
    "eligible",
  );
  assert.equal(
    historyRows.find(
      (r) =>
        r.input.asset_id === fixture.objects[fixture.objects.length - 1].sha,
    ).scores.eligibility,
    "metadata_unavailable",
  );
  checks.push(
    "historical matching requires the same source image content, including when a post replaces its image",
  );

  const strict = await complete(
    await submit(full, {
      ...parameters,
      minimum_stored_side: 768,
      exclude_banned: true,
    }),
  );
  const strictRows = await rows(strict.artifact.id);
  const si = (n) =>
    strictRows.find((r) => r.input.asset_id === fixture.objects[n].sha);
  assert.equal(si(11).scores.eligibility, "dimensions_unknown");
  assert.equal(si(12).scores.eligibility, "dimensions_excluded");
  assert.equal(si(9).input.dimension_basis, "asset_origin_raw_metadata");
  assert.equal(si(31).scores.eligibility, "policy_excluded");
  checks.push(
    "stored dimensions use the asset origin fallback and never source dimensions; purpose states stay separate",
  );

  const wsBody = {
    idempotency_key: crypto.randomUUID(),
    name: "排名输出工作集",
    filter: { rating: "q", selected_only: true },
  };
  const ws = await engine.api(
    base + "/artifacts/" + taggedRun.artifact.id + "/ranking/worksets",
    "POST",
    wsBody,
  );
  assert.equal(
    (
      await engine.api(
        base + "/artifacts/" + taggedRun.artifact.id + "/ranking/worksets",
        "POST",
        wsBody,
      )
    ).id,
    ws.id,
  );
  const wsRun = await complete(
    await submit(scope({ kind: "workset", collection_id: ws.id }), {
      ...parameters,
      mode: "rank",
    }),
  );
  assert.equal(wsRun.summary.input_count, ws.count);
  const wsRows = await rows(wsRun.artifact.id);
  assert.ok(wsRows.every((r) => r.input.rating === "q"));
  await engine.expectError(
    base + "/artifacts/" + taggedRun.artifact.id + "/release",
    "POST",
    undefined,
    "ARTIFACT_IN_USE",
  );
  checks.push(
    "ranked results become idempotent ordinary worksets, preserving query basis and artifact references",
  );

  const selection = await engine.api(base + "/selection");
  const keys = taggedRows
    .slice(0, 12)
    .map((r) => ({ source_id: r.input.source_id, asset_id: r.input.asset_id }));
  const selected = await engine.api(base + "/selection", "PATCH", {
    expected_revision: selection.revision,
    add: keys,
    remove: [],
    clear: true,
  });
  const fixedSelection = scope({
    kind: "selection",
    revision: selected.revision,
  });
  const selectedJob = await submit(fixedSelection, {
    ...parameters,
    mode: "rank",
  });
  await engine.api(base + "/selection", "PATCH", {
    expected_revision: selected.revision,
    add: [],
    remove: [],
    clear: true,
  });
  const selectedRun = await complete(selectedJob);
  assert.equal(selectedRun.summary.input_count, 12);
  checks.push(
    "selection changes after submission leave the fixed task input unchanged",
  );

  const firstPage = await engine.api(
    base + "/artifacts/" + original.artifact.id + "/ranking/rows",
    "POST",
    { filter: { rating: "q" }, limit: 5 },
  );
  assert.ok(firstPage.next_cursor);
  await engine.expectError(
    base + "/artifacts/" + original.artifact.id + "/ranking/rows",
    "POST",
    { filter: { rating: "e" }, cursor: firstPage.next_cursor },
    "INVALID_INPUT",
  );
  await engine.expectError(
    base + "/artifacts/" + taggedRun.artifact.id + "/ranking/rows",
    "POST",
    { filter: { rating: "q" }, cursor: firstPage.next_cursor },
    "INVALID_INPUT",
  );
  checks.push(
    "rank cursors are bounded and bound to both the artifact and filter",
  );

  const delayed = await submit(full, { ...parameters, mode: "rank" }, 600);
  await engine.wait(
    base + "/jobs",
    (r) => r.items.some((j) => j.id === delayed.id && j.status === "running"),
    60000,
  );
  await engine.api(base + "/jobs/" + delayed.id + "/cancel", "POST");
  await engine.wait(
    base + "/jobs",
    (r) => r.items.some((j) => j.id === delayed.id && j.status === "cancelled"),
    10000,
  );
  await sleep(350);
  const retry = await engine.api(
    base + "/jobs/" + delayed.id + "/retry",
    "POST",
  );
  const resumed = await complete(retry);
  assert.equal(resumed.job.attempt, 2);
  checks.push(
    "cancellation and retry retain the immutable input and resume completed rating checkpoints",
  );

  const interrupted = await submit(full, parameters, 1000);
  const checkpoint = resolve(
    currentProjectDirectory,
    ".staging",
    interrupted.id,
    "checkpoint.json",
  );
  let saved;
  for (let i = 0; i < 900; i++) {
    saved = await readFile(checkpoint, "utf8")
      .then(JSON.parse)
      .catch(() => null);
    if (saved?.finished?.length > 0) break;
    await sleep(40);
  }
  assert.ok(
    saved?.finished?.length > 0 && saved.finished.length < 4,
    "Crash must interrupt a partially checkpointed run",
  );
  await engine.stop(true);
  const pointer = within(fixtureRoot, resolve(fixture.lake, "CURRENT.json"));
  const offline = within(fixtureRoot, pointer + ".offline");
  await rename(pointer, offline);
  try {
    await engine.start();
    await engine.api(base + "/open", "POST");
    const recovered = await complete(interrupted);
    assert.deepEqual(
      await rows(recovered.artifact.id, { order: "input" }),
      all,
    );
    assert.equal(
      (await engine.api(base + "/artifacts")).items.filter(
        (a) => a.job_id === interrupted.id,
      ).length,
      1,
    );
  } finally {
    await rename(offline, pointer);
  }
  checks.push(
    "engine restart resumes a partially completed checkpoint while the source is offline, reproducing every row without duplicate publication",
  );

  const resources = await engine.api("/v1/resources");
  assert.equal(resources.previews.source_bytes, "0");
  assert.equal(resources.previews.pack_opens, 0);
  checks.push("ranking and recovery perform no preview or image pack reads");

  const after = await Promise.all(sourceFiles.map(digest));
  assert.deepEqual(after, before);
  checks.push(
    "source catalog and analysis database remain byte-for-byte unchanged",
  );
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      {
        checks,
        projectId,
        sourceId: source.id,
        original: original.summary,
        sourceHashes: before,
        run,
      },
      null,
      2,
    ),
  );
  console.log(JSON.stringify({ checks: checks.length, run }));
} finally {
  await engine.stop();
}
