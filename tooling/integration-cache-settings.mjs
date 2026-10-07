import { pythonCommand } from "./platform.mjs";
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { createHash, randomUUID } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { clientFixture } from "./client-fixture.mjs";
import { EngineFixture, sleep } from "./engine-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local",
  "test-runs",
  "integration-cache-settings-" + Date.now(),
);
await mkdir(run, { recursive: true });
const execute = promisify(execFile);
class SessionFixture extends EngineFixture {
  session = randomUUID();
  async response(path, method = "GET", body) {
    return fetch(this.connection.endpoint + path, {
      method,
      headers: {
        Authorization: "Bearer " + this.connection.token,
        "x-studio-session": this.session,
        ...(body === undefined ? {} : { "Content-Type": "application/json" }),
      },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      signal: AbortSignal.timeout(30000),
    });
  }
}
const engine = new SessionFixture(root, resolve(run, "state"));
const checks = [];
const digest = async (path) =>
  createHash("sha256")
    .update(await readFile(path))
    .digest("hex");
const oracle = async () =>
  JSON.parse(await readFile(resolve(run, "fixture.json"), "utf8"));
async function python(script, ...args) {
  await execute(
    pythonCommand(),
    [resolve(root, "tooling", script), run, ...args],
    {
      cwd: root,
      windowsHide: true,
    },
  );
}
function inspect(project, sql, parameters = [], writable = false) {
  const db = new DatabaseSync(resolve(project.directory, "project.sqlite"), {
    readOnly: !writable,
  });
  try {
    return writable
      ? db.prepare(sql).run(...parameters)
      : db.prepare(sql).all(...parameters);
  } finally {
    db.close();
  }
}
let source;
function specification(rating, tag) {
  return {
    version: 3,
    source_ids: [source.id],
    observation_rule: "current_post",
    order: "post_id_desc",
    conditions: [
      {
        field: "rating",
        operator: "in",
        value: { type: "text_list", value: [rating] },
      },
      ...(tag
        ? [
            {
              field: "tags",
              operator: "has_all_tags",
              value: { type: "text_list", value: [tag] },
            },
          ]
        : []),
    ],
  };
}
async function build(project, rating, tag) {
  const path = "/v1/projects/" + project.id + "/query-results";
  const result = await engine.api(path, "POST", {
    spec: specification(rating, tag),
  });
  const ready = await engine.wait(
    path + "/" + result.id,
    (r) => ["ready", "failed", "cancelled", "interrupted"].includes(r.state),
    60000,
  );
  assert.equal(ready.state, "ready", JSON.stringify(ready));
  return ready;
}
async function members(project, result) {
  const values = [];
  let cursor = null;
  do {
    const page = (
      await engine.api(
        "/v1/projects/" +
          project.id +
          "/query-results/" +
          result.id +
          "/assets?limit=48" +
          (cursor ? "&cursor=" + encodeURIComponent(cursor) : ""),
      )
    ).page;
    values.push(...page.items.map((a) => a.key.asset_id));
    cursor = page.next_cursor;
  } while (cursor);
  return values.sort();
}
function expected(reference, rating, tag) {
  return reference.images
    .filter((image) =>
      image.current.some(
        (row) => row.rating === rating && (!tag || row.tags?.includes(tag)),
      ),
    )
    .map((image) => image.sha)
    .sort();
}
const resultPath = (project, result) =>
  "/v1/projects/" + project.id + "/query-results/" + result.id;
async function clear(tier) {
  await engine.api("/v1/settings/cache/clear", "POST", { tier });
  await engine.wait("/v1/settings", (s) => !s.storage.cleanup_pending, 15000);
}
try {
  await python("ui-fixture.py");
  let reference = await oracle();
  await engine.start();
  const initial = await engine.api("/v1/settings");
  assert.equal(initial.cache.total_mib, 65536);
  assert.equal(initial.cache.temporary_idle_hours, 24);
  assert.equal(initial.cache.temporary_session_only, false);
  assert.equal(initial.cache.long_term_idle_days, null);
  const settings = { ...initial.cache, total_mib: 102400 };
  await engine.api("/v1/settings/cache", "PUT", settings);
  await engine.expectError(
    "/v1/settings/cache",
    "PUT",
    { ...settings, total_mib: 1024 },
    "INVALID_INPUT",
  );
  assert.deepEqual((await engine.api("/v1/settings")).cache, settings);
  checks.push(
    "Settings work before opening a project; 100 GiB persists and invalid category overcommit changes nothing",
  );
  const project = await engine.api("/v1/projects", "POST", {
    name: "缓存设置验收 A",
  });
  source = await engine.api("/v1/projects/" + project.id + "/sources", "POST", {
    name: "分级缓存夹具",
    kind: "danbooru",
    index_root: reference.lake,
    media_root: reference.lake,
  });
  const sourceFiles = ["catalog.sqlite", "analysis.duckdb"].map((name) =>
    resolve(reference.lake, "indexes/gen-ui", name),
  );
  const before = await Promise.all(sourceFiles.map(digest));
  await engine.api(
    "/v1/projects/" + project.id + "/sources/" + source.id + "/rating-bases",
    "POST",
  );
  let bases = await engine.wait(
    "/v1/cache/rating-bases",
    (value) =>
      value.builds.some(
        (b) =>
          b.source_id === source.id && ["ready", "failed"].includes(b.state),
      ),
    60000,
  );
  assert.equal(bases.builds[0].state, "ready", JSON.stringify(bases));
  assert.deepEqual(bases.items.map((b) => b.rating).sort(), [
    "e",
    "g",
    "q",
    "s",
  ]);
  assert.ok(bases.items.every((b) => !b.fixed && b.sequence === 1));
  const g = await build(project, "g");
  const e = await build(project, "e");
  const gTag = await build(project, "g", "solo");
  const eTag = await build(project, "e", "blue");
  for (const [result, rating, tag] of [
    [g, "g"],
    [e, "e"],
    [gTag, "g", "solo"],
    [eTag, "e", "blue"],
  ]) {
    assert.deepEqual(
      await members(project, result),
      expected(reference, rating, tag),
    );
    assert.deepEqual(result.cache.basis_ratings, [rating]);
    assert.ok(result.cache.candidate_records > 0);
    assert.equal(result.cache.tier, tag ? "temporary" : "long_term");
  }
  const redE = await build(project, "e", "red");
  assert.deepEqual(await members(project, redE), []);
  checks.push(
    "Four shared rating bases feed G/E and Tag combinations; same-image different-post tags never create a false match",
  );
  const basisFile = resolve(run, "state/rating-cache", source.id + "-g.sqlite");
  const basisBefore = await digest(basisFile);
  const projectB = await engine.api("/v1/projects", "POST", {
    name: "缓存设置验收 B",
  });
  const secondSource = await engine.api(
    "/v1/projects/" + projectB.id + "/sources",
    "POST",
    {
      name: "另一个项目中的同一湖",
      kind: "danbooru",
      index_root: reference.lake,
      media_root: reference.lake,
    },
  );
  assert.equal(secondSource.id, source.id);
  const shared = await build(projectB, "g", "solo");
  assert.deepEqual(
    await members(projectB, shared),
    expected(reference, "g", "solo"),
  );
  assert.equal(await digest(basisFile), basisBefore);
  assert.equal((await engine.api("/v1/cache/rating-bases")).items.length, 4);
  assert.deepEqual(await Promise.all(sourceFiles.map(digest)), before);
  assert.equal((await engine.api("/v1/resources")).previews.source_bytes, "0");
  checks.push(
    "A second project reuses the unchanged physical basis; all archive metadata bytes remain unchanged and no original image payload is read",
  );
  const countBefore = inspect(
    project,
    "SELECT count(*) n FROM query_member_data",
  )[0].n;
  await engine.api(resultPath(project, gTag) + "/retention", "PUT", {
    tier: "long_term",
    fixed: false,
  });
  assert.equal(
    inspect(project, "SELECT count(*) n FROM query_member_data")[0].n,
    countBefore,
  );
  assert.equal(
    (await engine.api(resultPath(project, gTag))).cache.tier,
    "long_term",
  );
  const repeated = await build(project, "g", "solo");
  assert.equal(repeated.cache.mode, "reused");
  assert.equal(repeated.cache.tier, "long_term");
  const scope = {
    project_id: project.id,
    target: { kind: "query_result", result_id: eTag.id },
  };
  const workset = await engine.api(
    "/v1/projects/" + project.id + "/collections",
    "POST",
    { name: "固定筛选成员", scope },
  );
  await engine.api(resultPath(project, redE) + "/retention", "PUT", {
    tier: "temporary",
    fixed: true,
  });
  await sleep(11000);
  await clear("temporary");
  assert.equal((await engine.api(resultPath(project, g))).state, "ready");
  assert.equal((await engine.api(resultPath(project, gTag))).state, "ready");
  assert.equal((await engine.api(resultPath(project, eTag))).state, "ready");
  assert.equal((await engine.api(resultPath(project, redE))).state, "ready");
  assert.equal(
    (await engine.api(resultPath(projectB, shared))).state,
    "released",
  );
  assert.equal(
    (await engine.api("/v1/projects/" + project.id + "/collections")).items[0]
      .count,
    workset.count,
  );
  checks.push(
    "Promotion shares member storage; temporary cleanup preserves long-term, explicitly fixed and workset-referenced members across projects",
  );
  const expiring = await build(project, "s", "portrait");
  inspect(
    project,
    "UPDATE query_families SET touched_at=? WHERE id=(SELECT family_id FROM query_results WHERE id=?)",
    [Date.now() - 25 * 3600000, expiring.id],
    true,
  );
  await sleep(11000);
  await engine.api("/v1/settings/cache", "PUT", settings);
  await engine.wait(
    resultPath(project, expiring),
    (value) => value.state === "released",
    15000,
  );
  assert.equal((await engine.api(resultPath(project, g))).state, "ready");
  checks.push(
    "The default one-day idle policy expires temporary results without aging permanent long-term results",
  );
  await engine.api("/v1/settings/cache", "PUT", {
    ...settings,
    temporary_session_only: true,
  });
  const sessionResult = await build(project, "q", "solo");
  assert.equal(sessionResult.cache.session_only, true);
  const oldSession = engine.session;
  const otherSession = randomUUID();
  engine.session = otherSession;
  const sharedSessionResult = await build(project, "q", "solo");
  assert.equal(sharedSessionResult.cache.mode, "reused");
  engine.session = oldSession;
  const firstClose = await engine.api(
    "/v1/projects/" + project.id + "/close",
    "POST",
  );
  assert.equal(
    firstClose.state,
    "open",
    "Another live client keeps the project open",
  );
  engine.session = otherSession;
  assert.equal(
    (await engine.api(resultPath(project, sessionResult))).state,
    "ready",
    "The remaining session retains the shared family",
  );
  await engine.api("/v1/projects/" + project.id + "/close", "POST");
  engine.session = randomUUID();
  await engine.api("/v1/projects/open", "POST", {
    directory: project.directory,
  });
  assert.equal(
    (await engine.api(resultPath(project, sessionResult))).state,
    "released",
    "An old session result cannot be revived by restoring a saved browser scope",
  );
  await engine.expectError(
    resultPath(project, sessionResult) + "/leases/" + randomUUID(),
    "POST",
    undefined,
    "RESULT_NOT_READY",
  );
  const nextSession = await build(project, "q", "solo");
  assert.notEqual(nextSession.cache.mode, "reused");
  assert.notEqual(engine.session, oldSession);
  await sleep(11000);
  await clear("temporary");
  assert.equal(
    (await engine.api(resultPath(project, sessionResult))).state,
    "released",
  );
  const keepAcross = await build(project, "q", "portrait");
  await engine.stop();
  engine.session = randomUUID();
  await engine.start();
  assert.equal(
    (await engine.api("/v1/settings")).cache.temporary_session_only,
    true,
  );
  await engine.api("/v1/projects/open", "POST", {
    directory: project.directory,
  });
  const restarted = await build(project, "q", "portrait");
  assert.notEqual(restarted.cache.mode, "reused");
  assert.notEqual(restarted.id, keepAcross.id);
  checks.push(
    "One client closing preserves another active session; after the last session closes, a new client or engine cannot resurrect session-only membership",
  );
  await engine.api("/v1/settings/cache", "PUT", settings);
  await python("query-fixture-update.py", "advance");
  reference = await oracle();
  const refreshed = await build(project, "g", "solo");
  assert.equal(refreshed.cache.mode, "incremental");
  assert.deepEqual(
    await members(project, refreshed),
    expected(reference, "g", "solo"),
  );
  bases = await engine.api("/v1/cache/rating-bases");
  assert.equal(bases.items.find((b) => b.rating === "g").sequence, 2);
  assert.equal(bases.items.find((b) => b.rating === "g").incremental, true);
  assert.equal(
    (await engine.api("/v1/projects/" + project.id + "/collections")).items[0]
      .count,
    workset.count,
  );
  const overview = await engine.api("/v1/settings");
  assert.ok(
    Number(overview.storage.total_bytes) >=
      Number(overview.storage.rating_basis_bytes) +
        Number(overview.storage.source_index_bytes) +
        Number(overview.storage.preview_bytes),
  );
  checks.push(
    "Daily source changes update shared bases and existing families incrementally while saved worksets stay fixed; aggregate accounting includes bases and previews",
  );
  const sdkDirectory = resolve(run, "sdk");
  const { StudioClient } = await clientFixture(root, sdkDirectory);
  const firstClient = new StudioClient(engine.connection);
  let reconnectedClient;
  try {
    await firstClient.settings.configureCache({
      ...settings,
      temporary_session_only: true,
    });
    const sdkProject = await firstClient.createProject({
      name: "SDK 项目会话验收",
    });
    const demo = await firstClient.attachSource(sdkProject.id, {
      name: "参考资料",
      kind: "demo",
    });
    const demoSpec = {
      version: 1,
      source_ids: [demo.id],
      conditions: [],
      observation_rule: "current_post",
      order: "asset_key_asc",
    };
    const runSdk = async (client) => {
      const request = await client.queries.run(sdkProject.id, demoSpec);
      return engine.wait(
        "/v1/projects/" + sdkProject.id + "/query-results/" + request.id,
        (r) => r.state === "ready",
        10000,
      );
    };
    await runSdk(firstClient);
    assert.equal((await runSdk(firstClient)).cache.mode, "reused");
    await firstClient.closeProject(sdkProject.id);
    await firstClient.openRecentProject(sdkProject.id);
    assert.notEqual((await runSdk(firstClient)).cache.mode, "reused");
    reconnectedClient = new StudioClient(engine.connection);
    reconnectedClient.preserveEdits(firstClient);
    firstClient.dispose();
    assert.equal(
      (await runSdk(reconnectedClient)).cache.mode,
      "reused",
      "Reconnecting a live client keeps its project session",
    );
    await reconnectedClient.releaseProjectViews();
  } finally {
    firstClient.dispose();
    reconnectedClient?.dispose();
  }
  await engine.api("/v1/settings/cache", "PUT", settings);
  checks.push(
    "The actual public SDK rotates session identity on project reopen and preserves it during a live connection rebind",
  );
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      {
        passed: true,
        checks,
        project,
        projectB,
        source,
        results: { g, e, gTag, eTag, refreshed },
        settings: overview,
      },
      null,
      2,
    ),
  );
  console.log(
    JSON.stringify({
      passed: true,
      checks: checks.length,
      report: resolve(run, "report.json"),
    }),
  );
} finally {
  await engine.stop();
}
