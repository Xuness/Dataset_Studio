import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { DatabaseSync } from "node:sqlite";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { EngineFixture } from "./engine-fixture.mjs";

const { values } = parseArgs({
  options: {
    connection: { type: "string" },
    "project-id": { type: "string" },
    "artifact-id": { type: "string" },
    label: { type: "string", default: "live" },
    "keep-workset": { type: "boolean", default: false },
  },
});
for (const key of ["connection", "project-id", "artifact-id"])
  assert.ok(values[key], `--${key} is required`);
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local/test-runs",
  "ranked-workset-benchmark-" + Date.now(),
);
await mkdir(run, { recursive: true });
const state = dirname(resolve(values.connection));
const engine = new EngineFixture(root, state, run);
engine.connection = JSON.parse(
  await readFile(resolve(values.connection), "utf8"),
);
assert.ok(
  ["127.0.0.1", "localhost", "[::1]"].includes(
    new URL(engine.connection.endpoint).hostname,
  ),
);
const health = await engine.api("/v1/health");
assert.equal(health.instance_id, engine.connection.instance_id);
const build = JSON.parse(
  await readFile(resolve(state, "development-build.json"), "utf8"),
);
const executableHash = createHash("sha256")
  .update(await readFile(resolve(root, "target/release/studio-engine.exe")))
  .digest("hex");
assert.equal(
  build.fingerprint,
  executableHash,
  "Benchmark requires the current release engine",
);
assert.equal(build.instance_id, health.instance_id);
const pid = values["project-id"],
  aid = values["artifact-id"],
  base = "/v1/projects/" + pid;
const project = await engine.api(base + "/open", "POST");
const artifact = await engine.api(base + "/artifacts/" + aid);
const scoreFile = artifact.files.find((f) =>
  f.path.endsWith(".ranking.sqlite"),
);
assert.ok(scoreFile);
const scores = new DatabaseSync(resolve(project.directory, scoreFile.path), {
  readOnly: true,
});
const control = new DatabaseSync(resolve(project.directory, "project.sqlite"), {
  readOnly: true,
});
const summary = await engine.api(base + "/artifacts/" + aid + "/ranking");
const report = {
  label: values.label,
  startedAt: new Date().toISOString(),
  executableHash,
  projectId: pid,
  artifactId: aid,
  boundary:
    "Existing immutable ranking; includes each API request through ready response and its first 48 ranked records. OS cache not cleared. No image decoding, reranking, or prebuilt per-workset index.",
  before: await engine.api("/v1/resources"),
  samples: [],
  worksets: [],
  results: [],
  cleanup: [],
};
const save = () =>
  writeFile(resolve(run, "report.json"), JSON.stringify(report, null, 2));
const scope = (id, kind = "workset") => ({
  project_id: pid,
  target:
    kind === "workset" ? { kind, collection_id: id } : { kind, result_id: id },
});
async function timed(kind, path, method = "GET", body) {
  const started = performance.now();
  const result = await engine.api(path, method, body);
  const sample = { kind, ms: performance.now() - started };
  if (result.page || result.items) {
    const page = result.page ?? result;
    sample.items = page.items.length;
    sample.preparing = page.preparing ?? null;
  }
  report.samples.push(sample);
  await save();
  return result;
}
async function ranked(kind, target, extra = {}) {
  const page = await timed(kind, base + "/ranking-browse/assets", "POST", {
    scope: target,
    limit: 48,
    ...extra,
  });
  assert.equal(
    page.preparing ?? null,
    null,
    kind + " must not build a scope index",
  );
  return page;
}
async function workset(kind, filter) {
  const created = await timed(
    "save-" + kind,
    base + "/artifacts/" + aid + "/ranking/worksets",
    "POST",
    {
      idempotency_key: crypto.randomUUID(),
      name: "排名读取验证 · " + kind,
      filter,
    },
  );
  report.worksets.push({ ...created, kind, filter });
  await save();
  assert.equal(
    control
      .prepare(
        "SELECT count(*) AS n FROM collection_member_legacy WHERE collection_id=?",
      )
      .get(created.id).n,
    0,
  );
  assert.equal(
    control
      .prepare(
        "SELECT count(*) AS n FROM collection_bases b JOIN ranking_memberships m ON m.result_id=b.result_id WHERE b.collection_id=?",
      )
      .get(created.id).n,
    1,
  );
  const first = await ranked("first-" + kind, scope(created.id));
  const clauses = ["1=1"],
    parameters = [];
  if (filter.eligibility) {
    clauses.push("eligibility=?");
    parameters.push(filter.eligibility);
  }
  if (filter.selected_only)
    clauses.push("selected_route IN ('main','rescue','audit')");
  if (filter.top) {
    clauses.push("main_rank IS NOT NULL AND main_rank<=?");
    parameters.push(filter.top);
  }
  const exact = scores
    .prepare("SELECT count(*) n FROM scores WHERE " + clauses.join(" AND "))
    .get(...parameters).n;
  assert.equal(created.count, exact, kind + " independent member count");
  for (const item of first.items) {
    const row = scores
      .prepare(
        "SELECT ordinal FROM scores WHERE ordinal=? AND " +
          clauses.join(" AND "),
      )
      .get(item.ranking.ordinal, ...parameters);
    assert.ok(row, kind + " first-page membership");
  }
  report.worksets.at(-1).recipeBytes = control
    .prepare(
      "SELECT length(m.recipe_json) n FROM collection_bases b JOIN ranking_memberships m ON m.result_id=b.result_id WHERE b.collection_id=?",
    )
    .get(created.id).n;
  return created;
}
function expected(rating, order = "main", descending = false) {
  const column =
    order === "input"
      ? "ordinal"
      : `coalesce(${order}_rank,9223372036854775807)`;
  const direction = descending ? "DESC" : "ASC";
  return scores
    .prepare(
      `SELECT ordinal FROM scores WHERE rating=? ORDER BY ${column} ${direction},ordinal ${direction} LIMIT 48`,
    )
    .all(rating)
    .map((r) => r.ordinal);
}
async function removeWorkset(id) {
  const path = base + "/objects/workset/" + id;
  const detail = await engine.api(path);
  await engine.api(path + "/actions", "POST", {
    action: "remove",
    expected_revision: detail.object.revision,
  });
  report.cleanup.push({ kind: "workset", id, released: true });
}
let full;
try {
  console.log(JSON.stringify({ run, projectId: pid, artifactId: aid }));
  full = await workset("全范围", { order: "main" });
  assert.equal(full.count, summary.input_count);
  const info = await timed(
    "open-info",
    base + "/ranking-browse?collection_id=" + full.id,
  );
  assert.equal(info.ranking.count, full.count);
  const sid = JSON.parse(
    control
      .prepare(
        "SELECT m.recipe_json FROM collection_bases b JOIN ranking_memberships m ON m.result_id=b.result_id WHERE b.collection_id=?",
      )
      .get(full.id).recipe_json,
  ).source_ids[0];
  for (let attempt = 0; attempt < 3; attempt++) {
    await timed(
      "lake-first-page",
      base + "/assets?source_id=" + sid + "&order=post_id_desc&limit=48",
    );
  }
  for (const rating of ["g", "s", "q", "e", "g"]) {
    const result = await timed(
      "filter-" + rating,
      base + "/query-results",
      "POST",
      {
        spec: {
          version: 3,
          source_ids: [sid],
          conditions: [
            {
              field: `project.${aid}.rating`,
              operator: "in",
              value: { type: "text_list", value: [rating] },
            },
          ],
          observation_rule: "current_post",
          order: "post_id_desc",
          input_scope: scope(full.id),
        },
      },
    );
    report.results.push(result.id);
    await save();
    assert.equal(result.state, "ready");
    assert.equal(
      control
        .prepare("SELECT storage_kind FROM query_results WHERE id=?")
        .get(result.id).storage_kind,
      "ranking",
    );
    const total = scores
      .prepare("SELECT count(*) AS n FROM scores WHERE rating=?")
      .get(rating).n;
    assert.equal(result.count, total);
    const target = scope(result.id, "query_result");
    const first = await ranked("rating-first-" + rating, target);
    assert.deepEqual(
      first.items.map((r) => r.ranking.ordinal),
      expected(rating),
    );
    if (first.next_cursor)
      await ranked("rating-next-" + rating, target, {
        cursor: first.next_cursor,
      });
    if (rating === "g") {
      for (const order of ["rescue", "direct", "fused", "input"]) {
        const page = await ranked("rating-order-" + order, target, {
          order,
          descending: true,
        });
        assert.deepEqual(
          page.items.map((r) => r.ranking.ordinal),
          expected(rating, order, true),
        );
      }
      const position = Math.floor(result.count * 0.8);
      const anchor = await ranked("rating-deep-position", target, {
        start_rank: String(position),
      });
      assert.equal(anchor.items.length, 48);
      const anchored = anchor.items[0].ranking;
      const rankedBefore = scores
        .prepare(
          "SELECT count(*) AS n FROM scores WHERE rating=? AND coalesce(main_rank,9223372036854775807)<?",
        )
        .get(rating, anchored.main_rank ?? 9223372036854775807n).n;
      const tiedBefore = scores
        .prepare(
          "SELECT count(*) AS n FROM scores WHERE rating=? AND coalesce(main_rank,9223372036854775807)=? AND ordinal<=?",
        )
        .get(
          rating,
          anchored.main_rank ?? 9223372036854775807n,
          anchored.ordinal,
        ).n;
      assert.equal(
        rankedBefore + tiedBefore,
        position,
        "actual full-data anchor position",
      );
      if (anchor.next_cursor)
        await ranked("rating-deep-next", target, {
          start_rank: String(position),
          cursor: anchor.next_cursor,
        });
      const natural = await timed(
        "rating-natural-post-order",
        base +
          "/query-results/" +
          result.id +
          "/assets?order=post_id_desc&limit=48",
      );
      assert.equal(natural.page.preparing ?? null, null);
      assert.equal(natural.page.items.length, 48);
    }
  }
  await workset("合格范围", { eligibility: "eligible", order: "main" });
  await workset("已入选", { selected_only: true, order: "main" });
  const top = await workset("各分级前100", { top: 100, order: "main" });
  await ranked("sparse-opposite-order", scope(top.id), {
    order: "rescue",
    descending: true,
  });
  const finalHealth = await engine.api("/v1/health");
  assert.equal(
    finalHealth.instance_id,
    health.instance_id,
    "Engine changed during timing",
  );
  report.after = await engine.api("/v1/resources");
  assert.equal(
    report.after.query_cache.ranked_index_builds,
    report.before.query_cache.ranked_index_builds,
  );
  report.passed = true;
} catch (error) {
  report.error = String(error?.stack ?? error);
  report.passed = false;
  throw error;
} finally {
  scores.close();
  control.close();
  for (const id of report.results) {
    try {
      const released = await engine.api(
        base + "/query-results/" + id + "/release",
        "POST",
      );
      assert.equal(released.state, "released");
      report.cleanup.push({ kind: "query_result", id, released: true });
    } catch (error) {
      report.cleanup.push({ kind: "query_result", id, error: String(error) });
    }
  }
  for (const entry of report.worksets) {
    if (values["keep-workset"] && report.passed && entry.id === full?.id) {
      report.keptWorkset = entry;
      continue;
    }
    try {
      await removeWorkset(entry.id);
    } catch (error) {
      report.cleanup.push({
        kind: "workset",
        id: entry.id,
        error: String(error),
      });
    }
  }
  const closed = new DatabaseSync(
    resolve(project.directory, "project.sqlite"),
    { readOnly: true },
  );
  report.internalOrphans = closed
    .prepare(
      "SELECT count(*) n FROM query_results r WHERE r.internal=1 AND r.storage_kind='ranking' AND r.status='ready' AND NOT EXISTS(SELECT 1 FROM result_references x WHERE x.result_id=r.id)",
    )
    .get().n;
  closed.close();
  if (
    report.cleanup.some((entry) => entry.error) ||
    report.internalOrphans !== 0
  ) {
    report.passed = false;
    process.exitCode = 1;
  }
  report.finishedAt = new Date().toISOString();
  await save();
  console.log(
    JSON.stringify({
      passed: report.passed,
      run,
      samples: report.samples,
      keptWorkset: report.keptWorkset?.id,
    }),
  );
}
