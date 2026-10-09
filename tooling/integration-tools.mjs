import assert from "node:assert/strict";
import { mkdir, readFile, writeFile, unlink } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { EngineFixture, within } from "./engine-fixture.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const runDir = resolve(
  root,
  ".local",
  "test-runs",
  "integration-tools-" + Date.now(),
);
await mkdir(runDir, { recursive: true });
const engine = new EngineFixture(root, resolve(runDir, "state"));
const checks = [];
const path = (id) => "/v1/projects/" + id;
const run = (operator_id, parameters) => ({
  operator_id,
  operator_version: 1,
  parameters_version: 1,
  parameters,
});
async function done(pid, jid) {
  const value = await engine.wait(
    path(pid) + "/jobs",
    (r) =>
      r.items.some(
        (j) =>
          j.id === jid &&
          ["succeeded", "failed", "cancelled"].includes(j.status),
      ),
    30000,
  );
  const job = value.items.find((j) => j.id === jid);
  assert.equal(job.status, "succeeded", JSON.stringify(job));
  return job;
}
process.on("exit", () => engine.child?.kill());
try {
  await engine.start();
  const preflight = await fetch(
    engine.connection.endpoint + "/v1/preferences/studio.layout",
    {
      method: "OPTIONS",
      headers: {
        Origin: "http://127.0.0.1:1420",
        "Access-Control-Request-Method": "PUT",
        "Access-Control-Request-Headers": "authorization,content-type",
      },
    },
  );
  assert.match(
    preflight.headers.get("access-control-allow-methods") ?? "",
    /PUT/,
  );
  const operators = await engine.api("/v1/operators");
  assert.deepEqual(
    operators.items.map((o) => o.id),
    [
      "core.export_files",
      "core.manifest",
      "core.scalar",
      "danbooru.metarecall",
      "danbooru.metarecall_v2",
    ],
  );
  assert.equal(
    operators.items.find((o) => o.id === "core.scalar").outputs.length,
    2,
  );
  const p = await engine.api("/v1/projects", "POST", {
    name: "工具与成果验证",
  });
  const p2 = await engine.api("/v1/projects", "POST", { name: "隔离草稿验证" });
  const base = path(p.id),
    other = path(p2.id);
  const source = await engine.api(base + "/sources", "POST", {
    kind: "demo",
    name: "确定性输入",
  });
  const page = await engine.api(base + "/assets?limit=16");
  const keys = page.items.slice(0, 12).map((a) => a.key);
  const selection = await engine.api(base + "/selection", "PATCH", {
    expected_revision: 0,
    add: keys,
    remove: [],
    clear: false,
  });
  const scope = {
    project_id: p.id,
    target: { kind: "selection", revision: selection.revision },
  };
  const request = {
    idempotency_key: crypto.randomUUID(),
    scope,
    run: run("core.scalar", { input: { kind: "stored_bytes" }, addend: "7" }),
  };
  for (const [change, code] of [
    [{ operator_version: 99 }, "OPERATOR_UNAVAILABLE"],
    [{ parameters_version: 99 }, "PARAMETERS_VERSION_UNSUPPORTED"],
    [
      { parameters: { input: { kind: "stored_bytes" }, multiplier: "oops" } },
      "INVALID_INPUT",
    ],
  ]) {
    await engine.expectError(
      base + "/tools/jobs",
      "POST",
      { ...request, run: { ...request.run, ...change } },
      code,
    );
  }
  assert.equal((await engine.api(base + "/jobs")).items.length, 0);
  const job = await engine.api(base + "/tools/jobs", "POST", request);
  await done(p.id, job.id);
  assert.equal(
    (await engine.api(base + "/tools/jobs", "POST", request)).id,
    job.id,
  );
  await engine.expectError(
    base + "/tools/jobs",
    "POST",
    {
      ...request,
      run: run("core.scalar", { input: { kind: "stored_bytes" }, addend: "8" }),
    },
    "IDEMPOTENCY_CONFLICT",
  );
  const fixed = await engine.api(base + "/jobs/" + job.id + "/run");
  assert.equal(fixed.run.parameters.multiplier, "1");
  assert.equal(fixed.fields[0].kind, "stored_bytes");
  const artifacts = (await engine.api(base + "/artifacts")).items;
  assert.equal(artifacts.length, 2);
  assert.ok(artifacts.every((a) => a.state === "ready"));
  const data = artifacts.find((a) => a.output_id === "data");
  assert.equal(data.count, 12);
  assert.equal(data.provenance.fields_frozen, true);
  assert.equal(data.provenance.run.parameters.addend, "7");
  assert.equal(data.files.length, 2);
  assert.equal(artifacts.find((a) => a.output_id === "failures").count, 0);
  let cursor,
    rows = [];
  do {
    const page = await engine.api(
      base +
        "/artifacts/" +
        data.id +
        "/rows?limit=5" +
        (cursor ? "&cursor=" + encodeURIComponent(cursor) : ""),
    );
    rows.push(...page.items);
    cursor = page.next_cursor;
  } while (cursor);
  assert.equal(rows.length, 12);
  assert.equal(new Set(rows.map((r) => r.key.asset_id)).size, 12);
  assert.ok(
    rows.every(
      (r) => r.scalar.status === "available" && r.scalar.value === "7",
    ),
  );
  await engine.expectError(
    other + "/artifacts/" + data.id,
    "GET",
    undefined,
    "NOT_FOUND",
  );
  checks.push(
    "two registered operators; invalid versions/parameters rejected before tasks; canonical idempotency; atomic multi-output publication and bounded typed paging",
  );
  const fields = await engine.api(base + "/sources/" + source.id + "/fields");
  const field = "project." + data.id + ".value";
  assert.ok(
    fields.fields.some((f) => f.id === field && f.field_type === "integer"),
  );
  const spec = {
    version: 1,
    source_ids: [source.id],
    conditions: [
      { field, operator: "gte", value: { type: "integer", value: "7" } },
    ],
    observation_rule: "any_observation",
    order: "asset_key_desc",
  };
  const query = await engine.api(base + "/queries", "POST", {
    name: "项目派生字段",
    spec,
  });
  const result = await engine.api(
    base + "/queries/" + query.id + "/results",
    "POST",
    { expected_revision: 1 },
  );
  const ready = await engine.wait(base + "/query-results/" + result.id, (r) =>
    ["ready", "failed"].includes(r.state),
  );
  assert.equal(ready.state, "ready", JSON.stringify(ready));
  assert.equal(ready.count, 12);
  const input = { kind: "artifact", artifact_id: data.id };
  const consume = await engine.api(base + "/tools/jobs", "POST", {
    idempotency_key: crypto.randomUUID(),
    scope,
    run: run("core.scalar", { input, multiplier: "2" }),
  });
  await done(p.id, consume.id);
  const reused = await engine.api(base + "/artifacts/" + consume.id + "/rows");
  assert.ok(reused.items.every((r) => r.scalar.value === "14"));
  const manifest = await engine.api(base + "/tools/jobs", "POST", {
    idempotency_key: crypto.randomUUID(),
    scope,
    run: run("core.manifest", { fields: [input] }),
  });
  await done(p.id, manifest.id);
  const exported = await engine.api(
    base + "/artifacts/" + manifest.id + "/rows",
  );
  assert.equal(exported.items[0].data.fields[0].value.value, "7");
  await engine.expectError(
    base + "/artifacts/" + data.id + "/release",
    "POST",
    undefined,
    "ARTIFACT_IN_USE",
  );
  checks.push(
    "immutable scalar artifacts consumed by scalar and manifest; namespaced derived fields queried with coverage exclusion; cross-project access and referenced release rejected",
  );
  const selection2 = await engine.api(base + "/selection", "PATCH", {
    expected_revision: selection.revision,
    add: [page.items[15].key],
    remove: [],
    clear: false,
  });
  const wider = {
    project_id: p.id,
    target: { kind: "selection", revision: selection2.revision },
  };
  const failed = await engine.api(base + "/tools/jobs", "POST", {
    idempotency_key: crypto.randomUUID(),
    scope: wider,
    run: run("core.scalar", { input, multiplier: "9223372036854775807" }),
  });
  await done(p.id, failed.id);
  const failureRows = await engine.api(
    base + "/artifacts/" + failed.id + "/rows",
  );
  assert.equal(
    failureRows.items.filter((r) => r.scalar.status === "failed").length,
    12,
  );
  assert.equal(
    failureRows.items.filter((r) => r.scalar.status === "uncomputed").length,
    1,
  );
  const failureArtifact = (await engine.api(base + "/artifacts")).items.find(
    (a) => a.job_id === failed.id && a.output_id === "failures",
  );
  assert.equal(failureArtifact.count, 12);
  assert.equal(
    (await engine.api(base + "/artifacts/" + failureArtifact.id + "/rows"))
      .items.length,
    12,
  );
  checks.push(
    "integer overflow is per-item failure; outside artifact coverage remains uncomputed; second output contains exactly failed items",
  );
  const draftPath = base + "/drafts/core.query/default";
  const retryJob = await engine.api(base + "/tools/jobs", "POST", {
    idempotency_key: crypto.randomUUID(),
    scope: wider,
    run: run("core.manifest", { fields: [input] }),
    delay_ms: 80,
  });
  await engine.wait(base + "/jobs", (r) =>
    r.items.some(
      (j) => j.id === retryJob.id && j.status === "running" && j.completed >= 8,
    ),
  );
  await engine.api(base + "/jobs/" + retryJob.id + "/cancel", "POST");
  await engine.api(base + "/jobs/" + retryJob.id + "/retry", "POST");
  const retried = await done(p.id, retryJob.id);
  assert.equal(retried.total, 13);
  assert.equal(retried.attempt, 2);
  assert.equal(
    (await engine.api(base + "/artifacts/" + retryJob.id + "/rows")).items
      .length,
    13,
  );
  checks.push(
    "explicit retry resumes cancelled registered task checkpoint with the same logical identity and fixed fields",
  );
  const value = {
    name: "未提交条件",
    conditions: [{ text: "unfinished >" }],
    scope,
  };
  const draft = await engine.api(draftPath, "PUT", {
    schema_version: 1,
    expected_revision: 0,
    value,
  });
  await engine.expectError(
    draftPath,
    "PUT",
    { schema_version: 1, expected_revision: 0, value: {} },
    "REVISION_CONFLICT",
  );
  assert.equal(
    (await engine.api(other + "/drafts/core.query/default")).draft,
    null,
  );
  await engine.api(other + "/drafts/core.query/default", "PUT", {
    schema_version: 1,
    expected_revision: 0,
    value: { name: "另一项目" },
  });
  const pref = await engine.api("/v1/preferences/studio.layout", "PUT", {
    schema_version: 1,
    expected_revision: 0,
    value: { inspector: false, density: "compact" },
  });
  assert.equal(pref.revision, 1);
  const countBefore = (await engine.api(base + "/jobs")).items.length;
  await engine.stop(true);
  await engine.start();
  await engine.api(base + "/open", "POST");
  await engine.api(other + "/open", "POST");
  assert.deepEqual((await engine.api(draftPath)).draft, draft);
  assert.equal(
    (await engine.api("/v1/preferences/studio.layout")).preference.value
      .inspector,
    false,
  );
  assert.equal((await engine.api(base + "/jobs")).items.length, countBefore);
  assert.equal(
    (await engine.api(base + "/selection")).revision,
    selection2.revision,
  );
  assert.equal(
    (await engine.api(base + "/artifacts/" + data.id + "/rows")).items.length,
    12,
  );
  checks.push(
    "project drafts and global preferences survive forced engine exit; CAS conflicts visible; restored drafts do not rebind selection or submit tasks",
  );
  // Recreate publication boundaries on stopped fixture files, never the daily runtime.
  await engine.stop();
  const directory = within(runDir, p.directory);
  const database = resolve(directory, "project.sqlite");
  const db = new DatabaseSync(database);
  db.prepare("UPDATE jobs SET status='running' WHERE id=?").run(failed.id);
  db.prepare("UPDATE artifacts SET status='publishing' WHERE job_id=?").run(
    failed.id,
  );
  db.prepare("DELETE FROM artifact_rows WHERE artifact_id=? AND ordinal>3").run(
    failed.id,
  );
  db.close();
  await unlink(resolve(directory, failureArtifact.files[0].path));
  await engine.start();
  await engine.api(base + "/open", "POST");
  await done(p.id, failed.id);
  const rebuilt = (await engine.api(base + "/artifacts")).items.filter(
    (a) => a.job_id === failed.id,
  );
  assert.equal(rebuilt.length, 2);
  assert.ok(rebuilt.every((a) => a.state === "ready"));
  assert.equal(
    rebuilt.find((a) => a.output_id === "failures").id,
    failureArtifact.id,
  );
  assert.equal(
    (await engine.api(base + "/artifacts/" + failed.id + "/rows")).items.length,
    13,
  );
  checks.push(
    "restart repairs final-file-before-registration, partial row index and absent secondary file; stable output IDs prevent duplicate publication",
  );
  const released = await engine.api(
    base + "/artifacts/" + failureArtifact.id + "/release",
    "POST",
  );
  assert.equal(released.state, "released");
  await engine.expectError(
    base + "/artifacts/" + failureArtifact.id + "/rows",
    "GET",
    undefined,
    "ARTIFACT_NOT_READY",
  );
  const manifestArtifact = await engine.api(base + "/artifacts/" + manifest.id);
  await writeFile(
    resolve(directory, manifestArtifact.files[0].path),
    "corrupt\n",
  );
  await engine.expectError(
    base + "/artifacts/" + manifest.id + "/verify",
    "POST",
    undefined,
    "ARTIFACT_INVALID",
  );
  assert.equal(
    (await engine.api(base + "/artifacts/" + manifest.id)).state,
    "unavailable",
  );
  assert.equal(
    (await engine.api(base + "/artifacts/" + data.id)).state,
    "ready",
  );
  checks.push(
    "unreferenced secondary artifact released; corruption isolated to its artifact without blocking project or neighboring results",
  );
  const exportDir = resolve(runDir, "export");
  await mkdir(exportDir, { recursive: true });
  const exportScope = {
    project_id: p.id,
    target: {
      kind: "selection",
      revision: (await engine.api(base + "/selection")).revision,
    },
  };
  await engine.expectError(
    base + "/tools/jobs",
    "POST",
    {
      idempotency_key: crypto.randomUUID(),
      scope: exportScope,
      run: run("core.export_files", { destination: resolve(runDir, "nope") }),
    },
    "DESTINATION_UNAVAILABLE",
  );
  const exportSubmitted = await engine.api(base + "/tools/jobs", "POST", {
    idempotency_key: crypto.randomUUID(),
    scope: exportScope,
    run: run("core.export_files", { destination: exportDir, metadata: "tags" }),
  });
  const exportJob = await done(p.id, exportSubmitted.id);
  const exportRows = (
    await readFile(resolve(exportDir, "manifest.jsonl"), "utf8")
  )
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
  assert.equal(exportRows.length, exportJob.total);
  assert.ok(
    exportRows.every(
      (row, i) =>
        row.status === "written" &&
        row.file.startsWith(String(i + 1).padStart(6, "0") + "_"),
    ),
  );
  // The demo lake has no tags, so no sidecar is written and nothing fails.
  assert.ok(exportRows.every((row) => !row.metadata_file && !row.error));
  const exportedFile = await readFile(resolve(exportDir, exportRows[0].file));
  assert.equal(exportRows[0].bytes, exportedFile.length);
  assert.equal(exportedFile.subarray(1, 4).toString(), "PNG");
  const assetPath =
    base +
    "/sources/" +
    exportRows[0].asset.source_id +
    "/assets/" +
    exportRows[0].asset.asset_id;
  const original = await fetch(
    engine.connection.endpoint + assetPath + "/original",
    { headers: { Authorization: "Bearer " + engine.connection.token } },
  );
  assert.equal(original.headers.get("content-type"), "image/png");
  assert.ok(Buffer.from(await original.arrayBuffer()).equals(exportedFile));
  const saved = await engine.api(assetPath + "/original/save", "POST", {
    path: resolve(runDir, "saved.png"),
  });
  assert.equal(saved.bytes, exportedFile.length);
  await engine.expectError(
    assetPath + "/original/save",
    "POST",
    { path: "relative.png" },
    "INVALID_INPUT",
  );
  const exportPlan = (await engine.api(base + "/artifacts")).items.find(
    (a) => a.job_id === exportSubmitted.id,
  );
  assert.equal(exportPlan.kind, "file_export");
  assert.equal(exportPlan.state, "ready");
  assert.equal(exportPlan.count, exportJob.total);
  checks.push(
    "file export writes numbered originals and manifest outside the lake; images without tags get no sidecar; original read/save match exported bytes",
  );
  await writeFile(
    resolve(runDir, "report.json"),
    JSON.stringify({ passed: true, checks }, null, 2),
  );
  console.log(
    JSON.stringify(
      { passed: true, checks, report: resolve(runDir, "report.json") },
      null,
      2,
    ),
  );
} catch (error) {
  await writeFile(
    resolve(runDir, "failure.json"),
    JSON.stringify({ error: String(error), checks }, null, 2),
  );
  throw error;
} finally {
  await engine.stop();
}
