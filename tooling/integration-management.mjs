import assert from "node:assert/strict";
import { execFile, spawn } from "node:child_process";
import { promisify } from "node:util";
import { createHash, randomUUID } from "node:crypto";
import { mkdir, readFile, writeFile, access } from "node:fs/promises";
import { DatabaseSync } from "node:sqlite";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { EngineFixture, sleep, within } from "./engine-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const runDir = resolve(
  root,
  ".local/test-runs/integration-management-" + Date.now(),
);
await mkdir(runDir, { recursive: true });
const engine = new EngineFixture(root, resolve(runDir, "state"));
const execute = promisify(execFile);
const checks = [];
const pp = (pid) => "/v1/projects/" + pid;
const objectPath = (pid, kind, id) => pp(pid) + "/objects/" + kind + "/" + id;
const describe = (pid, kind, id) => engine.api(objectPath(pid, kind, id));
const hash = async (path) =>
  createHash("sha256")
    .update(await readFile(path))
    .digest("hex");
async function action(pid, kind, id, verb = "remove") {
  const current = await describe(pid, kind, id);
  return engine.api(objectPath(pid, kind, id) + "/actions", "POST", {
    action: verb,
    expected_revision: current.object.revision,
  });
}
async function rename(pid, kind, id, name, notes = "验收备注") {
  const current = await describe(pid, kind, id);
  return engine.api(objectPath(pid, kind, id), "PATCH", {
    expected_revision: current.object.revision,
    name,
    notes,
  });
}
async function change(pid, add = [], remove = [], clear = false) {
  const current = await engine.api(pp(pid) + "/selection");
  return engine.api(pp(pid) + "/selection", "PATCH", {
    expected_revision: current.revision,
    add,
    remove,
    clear,
  });
}
async function restore(pid, verb) {
  const current = await engine.api(pp(pid) + "/selection");
  return engine.api(pp(pid) + "/selection/history", "POST", {
    action: verb,
    expected_revision: current.revision,
  });
}
const operator = (operator_id, parameters = {}) => ({
  operator_id,
  operator_version: 1,
  parameters_version: 1,
  parameters,
});
async function submit(pid, run, scope, delay_ms = 0) {
  const body = {
    idempotency_key: randomUUID(),
    run,
    scope: scope ?? {
      project_id: pid,
      target: {
        kind: "selection",
        revision: (await engine.api(pp(pid) + "/selection")).revision,
      },
    },
    delay_ms,
  };
  return { job: await engine.api(pp(pid) + "/tools/jobs", "POST", body), body };
}
async function finish(pid, id) {
  const jobs = await engine.wait(
    pp(pid) + "/jobs",
    (r) =>
      r.items.some(
        (j) =>
          j.id === id &&
          ["succeeded", "failed", "cancelled"].includes(j.status),
      ),
    60000,
  );
  const job = jobs.items.find((j) => j.id === id);
  assert.equal(job.status, "succeeded", JSON.stringify(job));
  return job;
}
async function artifacts(pid, jid) {
  return (await engine.api(pp(pid) + "/artifacts?limit=128")).items.filter(
    (item) => item.job_id === jid,
  );
}
async function errorAction(pid, kind, id, code) {
  const value = await describe(pid, kind, id);
  return engine.expectError(
    objectPath(pid, kind, id) + "/actions",
    "POST",
    { action: "remove", expected_revision: value.object.revision },
    code,
  );
}
let fileLock;
async function lockFile(file) {
  within(runDir, file);
  const helper = resolve(runDir, "hold-file.ps1");
  const ready = resolve(runDir, "file-lock.ready");
  const release = resolve(runDir, "file-lock.release");
  await writeFile(
    helper,
    [
      "param([string]$Path,[string]$ReadyPath,[string]$ReleasePath)",
      "$handle = [System.IO.File]::Open($Path,[System.IO.FileMode]::Open,[System.IO.FileAccess]::Read,[System.IO.FileShare]::Read)",
      "try {",
      "  [System.IO.File]::WriteAllText($ReadyPath,'ready')",
      "  $until = [DateTime]::UtcNow.AddSeconds(30)",
      "  while (-not (Test-Path -LiteralPath $ReleasePath) -and [DateTime]::UtcNow -lt $until) { Start-Sleep -Milliseconds 50 }",
      "} finally { $handle.Dispose() }",
    ].join("\n"),
    "utf8",
  );
  fileLock = spawn(
    "pwsh",
    [
      "-NoLogo",
      "-NoProfile",
      "-File",
      helper,
      "-Path",
      file,
      "-ReadyPath",
      ready,
      "-ReleasePath",
      release,
    ],
    { windowsHide: true, stdio: "ignore" },
  );
  for (let i = 0; i < 100; i++) {
    if (
      await access(ready).then(
        () => true,
        () => false,
      )
    )
      break;
    if (fileLock.exitCode !== null) throw new Error("File-lock helper exited");
    await sleep(50);
  }
  assert.equal(
    await access(ready).then(
      () => true,
      () => false,
    ),
    true,
  );
  return async () => {
    const stopped = new Promise((done) => fileLock.once("exit", done));
    await writeFile(release, "release");
    await stopped;
    fileLock = null;
  };
}
let project, base;
try {
  await engine.start();
  const settings = await engine.api("/v1/settings/editing");
  assert.deepEqual(settings, { undo_limit: 50, revision: 0 });
  await engine.expectError(
    "/v1/settings/editing",
    "PUT",
    { undo_limit: 201, expected_revision: 0 },
    "INVALID_INPUT",
  );
  await engine.api("/v1/settings/editing", "PUT", {
    undo_limit: 3,
    expected_revision: 0,
  });
  checks.push("editing defaults, hard limit and persisted configuration");

  project = await engine.api("/v1/projects", "POST", { name: "对象管理集成" });
  base = pp(project.id);
  const other = await engine.api("/v1/projects", "POST", { name: "来源隔离" });
  const source = await engine.api(base + "/sources", "POST", {
    name: "参考资料甲",
    kind: "demo",
  });
  const otherSource = await engine.api(pp(other.id) + "/sources", "POST", {
    name: "参考资料乙",
    kind: "demo",
  });
  assert.equal(source.id, otherSource.id);
  const manifest = await readFile(
    within(runDir, resolve(project.directory, "project.json")),
    "utf8",
  );
  await rename(project.id, "project", project.id, "管理验收项目", "项目备注");
  assert.equal((await engine.api(base)).name, "管理验收项目");
  assert.equal(
    (await engine.api("/v1/projects")).items.find((p) => p.id === project.id)
      .name,
    "管理验收项目",
  );
  assert.equal(
    await readFile(resolve(project.directory, "project.json"), "utf8"),
    manifest,
  );
  checks.push(
    "project labels and notes persist without changing identity manifest or directory",
  );

  const page = await engine.api(
    base + "/assets?source_id=" + source.id + "&order=asset_key_asc&limit=12",
  );
  assert.ok(page.items.length >= 4);
  const keys = page.items.map((item) => item.key);
  await change(project.id, keys.slice(0, 3));
  await change(project.id, [keys[3]], [keys[0]]);
  await change(project.id, [], [], true);
  assert.equal((await restore(project.id, "undo")).selection.count, 3);
  const old = await restore(project.id, "undo");
  assert.equal(old.selection.count, 3);
  await restore(project.id, "redo");
  await engine.expectError(
    base + "/selection/history",
    "POST",
    { action: "undo", expected_revision: old.selection.revision },
    "REVISION_CONFLICT",
  );
  const selected = await engine.api(
    base + "/assets?selection=true&order=asset_key_asc&limit=12",
  );
  assert.deepEqual(
    selected.items.map((item) => item.key.asset_id).sort(),
    keys
      .slice(1, 4)
      .map((key) => key.asset_id)
      .sort(),
  );
  const beforeRestart = await engine.api(base + "/selection/history");
  await engine.stop();
  await engine.start();
  await engine.api(base + "/open", "POST");
  await engine.api(pp(other.id) + "/open", "POST");
  assert.deepEqual(
    await engine.api(base + "/selection/history"),
    beforeRestart,
  );
  checks.push(
    "point selection, clear, undo, redo, CAS and engine restart preserve exact membership",
  );

  await engine.api("/v1/settings/editing", "PUT", {
    undo_limit: 1,
    expected_revision: 1,
  });
  const limited = await engine.api(base + "/selection/history");
  assert.ok(limited.undo_steps + limited.redo_steps <= 1);
  await engine.api("/v1/settings/editing", "PUT", {
    undo_limit: 0,
    expected_revision: 2,
  });
  await change(project.id, [keys[0]], [], true);
  assert.equal((await engine.api(base + "/selection/history")).undo_steps, 0);
  await engine.api("/v1/settings/editing", "PUT", {
    undo_limit: 50,
    expected_revision: 3,
  });
  checks.push(
    "shrinking and disabling history bounds stored steps without changing selected members",
  );

  const workset = await engine.api(base + "/collections", "POST", {
    name: "临时工作集",
  });
  const renamed = await rename(
    project.id,
    "workset",
    workset.id,
    "候选 100%_工作集",
    "唯一查找备注",
  );
  const listed = await engine.api(
    base +
      "/objects/workset?search=" +
      encodeURIComponent("100%_") +
      "&order=name_asc",
  );
  assert.equal(listed.items.length, 1);
  assert.equal(listed.items[0].id, workset.id);
  await engine.expectError(
    objectPath(project.id, "workset", workset.id),
    "PATCH",
    { name: "冲突覆盖", notes: "", expected_revision: renamed.revision - 1 },
    "REVISION_CONFLICT",
  );
  await engine.expectError(
    objectPath(other.id, "workset", workset.id),
    "GET",
    undefined,
    "NOT_FOUND",
  );
  checks.push(
    "workset names, notes, literal wildcard search and project isolation",
  );

  await rename(project.id, "source", source.id, "当前项目别名");
  const mediaPath =
    base +
    "/sources/" +
    source.id +
    "/assets/" +
    keys[0].asset_id +
    "/media?edge=128";
  assert.equal((await engine.response(mediaPath)).ok, true);
  await action(project.id, "source", source.id);
  assert.equal((await engine.api(base + "/sources")).items.length, 0);
  await engine.expectError(mediaPath, "GET", undefined, "SOURCE_DETACHED");
  assert.equal(
    (await engine.api(pp(other.id) + "/sources")).items[0].name,
    "参考资料乙",
  );
  assert.equal(
    (await engine.api(base + "/collections")).items[0].count,
    workset.count,
  );
  assert.equal((await engine.api(base + "/selection")).count, 1);
  await action(project.id, "source", source.id, "reconnect");
  assert.equal(
    (await engine.api(base + "/sources")).items[0].name,
    "当前项目别名",
  );
  assert.equal((await engine.response(mediaPath)).ok, true);
  checks.push(
    "source unlink fences cached media, preserves worksets, and leaves another project unchanged",
  );

  const spec = {
    version: 2,
    source_ids: [source.id],
    conditions: [],
    observation_rule: "current_post",
    order: "asset_key_asc",
    input_scope: {
      project_id: project.id,
      target: { kind: "workset", collection_id: workset.id },
    },
  };
  const definition = await engine.api(base + "/queries", "POST", {
    name: "工作集依赖查询",
    spec,
  });
  const result = await engine.api(
    base + "/queries/" + definition.id + "/results",
    "POST",
    { expected_revision: definition.revision },
  );
  const ready = await engine.wait(
    base + "/query-results/" + result.id,
    (r) => r.state === "ready",
  );
  assert.equal(
    (await describe(project.id, "workset", workset.id)).incoming.some(
      (link) =>
        link.id === definition.id &&
        link.name === "工作集依赖查询" &&
        link.blocking,
    ),
    true,
  );
  await errorAction(project.id, "workset", workset.id, "OBJECT_IN_USE");
  await action(project.id, "query", definition.id);
  await engine.expectError(
    base + "/queries/" + definition.id,
    "GET",
    undefined,
    "OBJECT_REMOVED",
  );
  await action(project.id, "workset", workset.id);
  assert.deepEqual(
    (await engine.api(base + "/query-results/" + result.id)).spec,
    ready.spec,
  );
  assert.equal(
    (
      await engine.api(
        base + "/query-results/" + result.id + "/assets?limit=12",
      )
    ).page.items.length,
    1,
  );
  checks.push(
    "dependency details block deletion and old query results survive definition and input-workset removal",
  );

  const producer = await submit(
    project.id,
    operator("core.scalar", {
      input: { kind: "stored_bytes" },
      multiplier: "2",
      addend: "1",
    }),
  );
  await finish(project.id, producer.job.id);
  const producerArtifacts = await artifacts(project.id, producer.job.id);
  const primary = producerArtifacts.find((a) => a.output_id === "data");
  const originalHashes = await Promise.all(
    primary.files.map((f) =>
      hash(within(runDir, resolve(project.directory, f.path))),
    ),
  );
  await rename(project.id, "artifact", primary.id, "首轮评分", "用于复用参数");
  assert.deepEqual(
    await Promise.all(
      primary.files.map((f) => hash(resolve(project.directory, f.path))),
    ),
    originalHashes,
  );
  const detail = await describe(project.id, "artifact", primary.id);
  assert.equal(detail.run.parameters.multiplier, "2");
  assert.equal(detail.object.name, "首轮评分");
  const location = await engine.api(
    objectPath(project.id, "artifact", primary.id) + "/reveal",
    "POST",
    { file_index: 0, open: false },
  );
  assert.equal(location.opened, false);
  within(runDir, location.path);
  checks.push(
    "renaming an actual computed artifact preserves file hashes and exposes its original parameters and location",
  );

  const preset = await engine.api(base + "/presets", "POST", {
    id: null,
    name: "标量参数预设",
    notes: "只保存参数",
    expected_revision: 0,
    run: detail.run,
  });
  await engine.expectError(
    base + "/presets",
    "POST",
    {
      id: preset.id,
      name: "错误版本",
      notes: "",
      expected_revision: 0,
      run: detail.run,
    },
    "REVISION_CONFLICT",
  );
  assert.deepEqual(
    (await engine.api(base + "/presets?operator_id=core.scalar")).items[0].run,
    detail.run,
  );
  await engine.api(base + "/presets/" + preset.id + "/delete", "POST", {
    expected_revision: preset.revision,
  });
  assert.equal(
    (await engine.api(base + "/presets?operator_id=core.scalar")).items.length,
    0,
  );
  checks.push(
    "named presets preserve normalized parameters and enforce update/delete revisions",
  );

  const consumer = await submit(
    project.id,
    operator("core.scalar", {
      input: { kind: "artifact", artifact_id: primary.id },
    }),
  );
  await finish(project.id, consumer.job.id);
  await rename(project.id, "job", consumer.job.id, "下游计算任务");
  await errorAction(project.id, "artifact", primary.id, "OBJECT_IN_USE");
  assert.equal(
    (await describe(project.id, "artifact", primary.id)).incoming.some(
      (link) => link.kind === "job" && link.name === "下游计算任务",
    ),
    true,
  );
  await action(project.id, "job", consumer.job.id, "archive");
  assert.equal(
    (await engine.api(base + "/job-history?search=" + consumer.job.id)).items
      .length,
    0,
  );
  assert.equal(
    (
      await engine.api(
        base + "/job-history?state=archived&include_archived=true",
      )
    ).items[0].object.id,
    consumer.job.id,
  );
  await action(project.id, "job", consumer.job.id, "unarchive");
  await errorAction(project.id, "job", consumer.job.id, "OBJECT_IN_USE");
  for (const item of await artifacts(project.id, consumer.job.id))
    await action(project.id, "artifact", item.id);
  await action(project.id, "job", consumer.job.id);
  await engine.expectError(
    base + "/tools/jobs",
    "POST",
    consumer.body,
    "OBJECT_REMOVED",
  );
  assert.equal(
    (await describe(project.id, "artifact", primary.id)).can_remove,
    true,
  );
  checks.push(
    "archive keeps dependencies; removing downstream outputs and retired task releases them without resubmission resurrection",
  );

  if (process.platform === "win32") {
    const unlock = await lockFile(
      resolve(project.directory, primary.files[0].path),
    );
    try {
      await errorAction(
        project.id,
        "artifact",
        primary.id,
        "ARTIFACT_CLEANUP_PENDING",
      );
    } finally {
      await unlock();
    }
    const pending = await describe(project.id, "artifact", primary.id);
    assert.equal(pending.object.state, "released");
    assert.ok(Number(pending.object.bytes) > 0);
  }
  await action(project.id, "artifact", primary.id);
  assert.equal(
    (await describe(project.id, "artifact", primary.id)).object.bytes,
    "0",
  );
  assert.equal(
    (await engine.api(base + "/job-history?search=" + producer.job.id)).items[0]
      .result_available,
    false,
  );
  checks.push(
    "partial file deletion remains inspectable and retryable; task history reports released output",
  );

  const lakeRoot = resolve(runDir, "ranking-fixture");
  await execute(
    "python",
    [resolve(root, "tooling/ranking-fixture.py"), lakeRoot, "1024"],
    { cwd: root, windowsHide: true },
  );
  const fixture = JSON.parse(
    await readFile(resolve(lakeRoot, "fixture.json"), "utf8"),
  );
  const sourceFiles = ["catalog.sqlite", "analysis.duckdb"].map((name) =>
    resolve(fixture.lake, "indexes/gen-ranking", name),
  );
  const sourceHashes = await Promise.all(sourceFiles.map(hash));
  const lake = await engine.api(base + "/sources", "POST", {
    kind: "danbooru",
    name: "排名数据湖",
    index_root: fixture.lake,
    media_root: fixture.lake,
  });
  const rank = await submit(
    project.id,
    operator("danbooru.metarecall", {
      ratings: ["g", "s", "q", "e"],
      mode: "rank",
      artist_enabled: false,
      cohort_minimum: 8,
    }),
    {
      project_id: project.id,
      target: { kind: "source", source_id: lake.id, revision: lake.revision },
    },
  );
  await finish(project.id, rank.job.id);
  const ranking = (await artifacts(project.id, rank.job.id)).find(
    (a) => a.kind === "ranking_table",
  );
  const filter = {
    rating: "g",
    eligibility: "eligible",
    top: 4,
    order: "main",
  };
  const rowsPath = base + "/artifacts/" + ranking.id + "/ranking/rows";
  const rows = await engine.api(rowsPath, "POST", { filter, limit: 4 });
  const body = {
    idempotency_key: randomUUID(),
    name: "排名保存的候选",
    filter,
  };
  const wsPath = base + "/artifacts/" + ranking.id + "/ranking/worksets";
  const ws = await engine.api(wsPath, "POST", body);
  assert.equal(
    (await describe(project.id, "workset", ws.id)).provenance.ranking_artifact,
    ranking.id,
  );
  await errorAction(project.id, "artifact", ranking.id, "OBJECT_IN_USE");
  await rename(project.id, "workset", ws.id, "重命名的排名候选");
  assert.equal(
    (await engine.api(wsPath, "POST", body)).name,
    "重命名的排名候选",
  );
  await action(project.id, "source", lake.id);
  assert.deepEqual(
    (await engine.api(rowsPath, "POST", { filter, limit: 4 })).items,
    rows.items,
  );
  await action(project.id, "source", lake.id, "reconnect");
  await action(project.id, "workset", ws.id);
  await engine.expectError(wsPath, "POST", body, "OBJECT_REMOVED");
  await action(project.id, "artifact", ranking.id);
  assert.deepEqual(await Promise.all(sourceFiles.map(hash)), sourceHashes);
  checks.push(
    "real ranking worksets expose filters, preserve offline scores and idempotency tombstones, and never modify lake files",
  );

  const db = new DatabaseSync(
    within(runDir, resolve(project.directory, "project.sqlite")),
    { readOnly: true },
  );
  assert.equal(db.prepare("PRAGMA user_version").get().user_version, 9);
  assert.equal(db.prepare("PRAGMA foreign_key_check").all().length, 0);
  assert.equal(
    db
      .prepare("SELECT COUNT(*) AS n FROM job_inputs WHERE job_id=?")
      .get(consumer.job.id).n,
    0,
  );
  db.close();
  checks.push(
    "database v9 retains valid foreign keys after combined lifecycle operations",
  );
  await writeFile(
    resolve(runDir, "report.json"),
    JSON.stringify({ passed: true, checks, project: project.id }, null, 2),
  );
  console.log(
    JSON.stringify({
      passed: true,
      checks: checks.length,
      report: resolve(runDir, "report.json"),
    }),
  );
} catch (error) {
  await writeFile(
    resolve(runDir, "report.json"),
    JSON.stringify({ passed: false, checks, error: String(error) }, null, 2),
  );
  throw error;
} finally {
  fileLock?.kill();
  await engine.stop();
}
