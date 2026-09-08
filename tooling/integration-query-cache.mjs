import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { EngineFixture, sleep } from "./engine-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const runDir = resolve(root, ".local", "integration-query-cache-" + Date.now());
await mkdir(runDir, { recursive: true });
const engine = new EngineFixture(root, resolve(runDir, "state"));
const execute = promisify(execFile);
const checks = [];
const reference = async () =>
  JSON.parse(await readFile(resolve(runDir, "fixture.json"), "utf8"));
async function python(script, ...args) {
  await execute("python", [resolve(root, "tooling", script), runDir, ...args], {
    cwd: root,
    windowsHide: true,
  });
}
function inspect(project, sql) {
  const db = new DatabaseSync(resolve(project.directory, "project.sqlite"), {
    readOnly: true,
  });
  try {
    return db.prepare(sql).all();
  } finally {
    db.close();
  }
}
let project, path, spec;
async function build(input = spec) {
  const result = await engine.api(path + "/query-results", "POST", {
    spec: input,
  });
  return engine.wait(
    path + "/query-results/" + result.id,
    (r) => ["ready", "failed", "interrupted", "cancelled"].includes(r.state),
    60000,
  );
}
async function members(result, order = "post_id_desc") {
  assert.equal(result.state, "ready", JSON.stringify(result));
  let cursor = null;
  const values = [];
  do {
    const page = (
      await engine.api(
        path +
          "/query-results/" +
          result.id +
          "/assets?limit=48&order=" +
          order +
          (cursor ? "&cursor=" + encodeURIComponent(cursor) : ""),
      )
    ).page;
    values.push(...page.items.map((a) => a.key.asset_id));
    cursor = page.next_cursor;
  } while (cursor);
  return values;
}
async function browse(params) {
  let cursor = null;
  const values = [];
  let first = true;
  do {
    const endpoint =
      path +
      "/assets?limit=48&" +
      params +
      (cursor ? "&cursor=" + encodeURIComponent(cursor) : "");
    const page = await engine.wait(endpoint, (p) => !p.preparing, 60000);
    values.push(...page.items.map((a) => a.key.asset_id));
    cursor = page.next_cursor;
    if (first) {
      first = false;
      assert.ok(page.revision);
    }
  } while (cursor);
  return values;
}
function expected(ref, filter = true, descending = true) {
  return ref.images
    .filter((i) => !filter || i.current.some((c) => c.rating === "e"))
    .sort((a, b) => {
      const ap = a.post_ids.length ? Math.min(...a.post_ids) : null,
        bp = b.post_ids.length ? Math.min(...b.post_ids) : null;
      if (ap === null && bp !== null) return 1;
      if (ap !== null && bp === null) return -1;
      return (
        ((ap ?? 0) - (bp ?? 0) || a.sha.localeCompare(b.sha)) *
        (descending ? -1 : 1)
      );
    })
    .map((i) => i.sha);
}
process.on("exit", () => engine.child?.kill());
try {
  await python("ui-fixture.py");
  await engine.start();
  project = await engine.api("/v1/projects", "POST", {
    name: "每日增量协议验收",
  });
  path = "/v1/projects/" + project.id;
  const ref = await reference();
  const source = await engine.api(path + "/sources", "POST", {
    name: "隔离 Danbooru",
    kind: "danbooru",
    index_root: ref.lake,
    media_root: ref.lake,
  });
  spec = {
    version: 3,
    source_ids: [source.id],
    conditions: [
      {
        field: "rating",
        operator: "in",
        value: { type: "text_list", value: ["e"] },
      },
    ],
    observation_rule: "current_post",
    order: "post_id_desc",
  };
  assert.deepEqual(await browse("order=post_id_desc"), expected(ref, false));
  assert.deepEqual(
    await browse("order=post_id_asc"),
    expected(ref, false, false),
  );
  checks.push(
    "Cold source sorting prepares asynchronously; bounded pages globally sort ID with deterministic missing IDs last",
  );
  const first = await build();
  const baseline = await members(first);
  assert.deepEqual(baseline, expected(ref));
  const countBefore = inspect(
    project,
    "SELECT count(*) AS n FROM query_member_data",
  )[0].n;
  const repeated = await build({ ...spec, order: "post_id_asc" });
  assert.equal(repeated.cache.mode, "reused");
  assert.equal(
    inspect(project, "SELECT count(*) AS n FROM query_member_data")[0].n,
    countBefore,
  );
  assert.deepEqual(
    await members(repeated, "post_id_asc"),
    expected(ref, true, false),
  );
  checks.push("Equivalent query and reverse order share one member version");
  const scope = {
    project_id: project.id,
    target: { kind: "query_result", result_id: first.id },
  };
  const selection = await engine.api(path + "/selection/scope", "POST", {
    expected_revision: 0,
    scope,
    operation: "replace",
  });
  const workset = await engine.api(path + "/collections", "POST", {
    name: "固定第一天",
    scope,
  });
  assert.equal(selection.count, baseline.length);
  assert.equal(workset.count, baseline.length);
  await browse("selection=true&order=post_id_desc");
  await browse("collection_id=" + workset.id + "&order=post_id_asc");
  const history = await engine.api(path + "/query-results");
  assert.equal(
    history.items.length,
    2,
    "Internal sorting does not fill query history",
  );
  checks.push(
    "Selection and workset sort correctly and internal builds stay outside query history",
  );
  const days = [];
  for (let day = 0; day < 4; day++) {
    await python("query-fixture-update.py", "advance");
    const next = await build();
    assert.equal(next.cache.mode, "incremental", JSON.stringify(next));
    const updated = await reference();
    assert.deepEqual(await members(next), expected(updated));
    assert.ok(
      next.cache.evaluated_objects < updated.images.length / 2,
      JSON.stringify(next.cache),
    );
    assert.deepEqual(
      await browse("order=post_id_desc"),
      expected(updated, false),
    );
    const fixed = await browse(
      "collection_id=" + workset.id + "&order=post_id_desc",
    );
    assert.deepEqual([...fixed].sort(), [...baseline].sort());
    days.push({
      sequence: updated.sequence,
      cache: next.cache,
      count: next.count,
    });
  }
  checks.push(
    "Four daily commits update old ratings, replace old physical associations and insert lower post IDs; every result equals independent fixture truth",
  );
  for (const mode of ["gap", "rebuild"]) {
    await python("query-fixture-update.py", mode);
    const next = await build();
    assert.equal(next.cache.mode, "rebuilt", JSON.stringify(next));
    assert.deepEqual(await members(next), expected(await reference()));
  }
  checks.push(
    "Missing commit continuity and generation replacement trigger full evaluation with correct memberships",
  );
  const live = await build();
  const lease = crypto.randomUUID();
  await engine.api(
    path + "/query-results/" + live.id + "/leases/" + lease,
    "POST",
  );
  await engine.api("/v1/resources/query-cache", "PUT", {
    quota_mib: 0,
    max_age_days: 1,
  });
  await engine.api("/v1/resources/query-cache/clear", "POST");
  await sleep(11000);
  await engine.api("/v1/resources/query-cache/clear", "POST");
  await sleep(800);
  assert.equal(
    (await engine.api(path + "/query-results/" + live.id)).state,
    "ready",
  );
  assert.equal(
    (await engine.api(path + "/query-results/" + first.id)).state,
    "ready",
  );
  await engine.api(
    path + "/query-results/" + live.id + "/leases/" + lease + "/release",
    "POST",
  );
  await engine.api("/v1/resources/query-cache/clear", "POST");
  await engine.wait(
    path + "/query-results/" + live.id,
    (r) => r.state === "released",
    10000,
  );
  assert.equal(
    (await engine.api(path + "/query-results/" + first.id)).state,
    "ready",
  );
  checks.push(
    "Clear and zero quota preserve active view and durable workset/selection references, reclaiming the unreferenced view after release",
  );
  const noCache = await build();
  assert.equal(noCache.cache.mode, "full");
  await engine.api(path + "/close", "POST");
  await sleep(11000);
  await engine.api("/v1/resources/query-cache/clear", "POST");
  await engine.wait(
    "/v1/resources",
    (s) => !s.query_cache.cleanup_pending,
    10000,
  );
  await engine.api("/v1/projects/open", "POST", {
    directory: project.directory,
  });
  await engine.wait(
    path + "/query-results/" + noCache.id,
    (r) => r.state === "released",
    10000,
  );
  assert.equal(
    (await engine.api(path + "/collections")).items[0].count,
    baseline.length,
  );
  await engine.stop();
  await engine.start();
  await engine.api("/v1/projects/open", "POST", {
    directory: project.directory,
  });
  assert.equal((await engine.api(path + "/selection")).count, baseline.length);
  assert.equal(inspect(project, "PRAGMA auto_vacuum")[0].auto_vacuum, 2);
  checks.push(
    "Disabled reuse is reclaimed in closed projects; restart preserves fixed membership and incremental free-page reclamation",
  );
  await writeFile(
    resolve(runDir, "report.json"),
    JSON.stringify({ checks, days, project, source, first, live }, null, 2),
  );
  console.log(
    JSON.stringify(
      {
        ok: true,
        checks: checks.length,
        report: resolve(runDir, "report.json"),
      },
      null,
      2,
    ),
  );
} finally {
  await engine.stop();
}
