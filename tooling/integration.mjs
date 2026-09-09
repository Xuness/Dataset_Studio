import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, readFile, writeFile, open, rename } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { engineExecutable, engineProfile } from "./engine-profile.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const runDir = resolve(
  root,
  ".local",
  "test-runs",
  "integration-" + Date.now(),
);
await mkdir(runDir, { recursive: true });
const dataDir = resolve(runDir, "state");
const binary = engineExecutable(root, engineProfile([], process.env, "debug"));
let child;
let connection;
const checks = [];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function start() {
  await mkdir(dataDir, { recursive: true });
  const log = await open(resolve(runDir, "engine.log"), "a");
  child = spawn(binary, ["serve", "--data-dir", dataDir], {
    stdio: ["ignore", log.fd, log.fd],
    windowsHide: true,
  });
  await log.close();
  for (let attempt = 0; attempt < 80; attempt++) {
    try {
      const candidate = JSON.parse(
        await readFile(resolve(dataDir, "engine.json"), "utf8"),
      );
      if (candidate.pid === child.pid) {
        const response = await fetch(candidate.endpoint + "/v1/health", {
          headers: { Authorization: "Bearer " + candidate.token },
          signal: AbortSignal.timeout(700),
        });
        if (response.ok) {
          connection = candidate;
          return;
        }
      }
    } catch {
      /* startup */
    }
    if (child.exitCode !== null)
      throw new Error(
        "Engine exited: " +
          (await readFile(resolve(runDir, "engine.log"), "utf8")),
      );
    await sleep(100);
  }
  throw new Error("Engine startup timed out");
}
async function stop() {
  if (child && child.exitCode === null) {
    const exited = new Promise((resolve) => child.once("exit", resolve));
    child.kill();
    await exited;
  }
  child = undefined;
  await sleep(100);
}
async function api(path, method = "GET", body) {
  const response = await fetch(connection.endpoint + path, {
    method,
    headers: {
      Authorization: "Bearer " + connection.token,
      ...(body ? { "Content-Type": "application/json" } : {}),
    },
    ...(body ? { body: JSON.stringify(body) } : {}),
  });
  const value = await response.json();
  if (!response.ok)
    throw new Error(response.status + ": " + JSON.stringify(value));
  return value;
}
async function waitJob(pid, jid, predicate, timeout = 18000) {
  const until = Date.now() + timeout;
  while (Date.now() < until) {
    const job = (await api("/v1/projects/" + pid + "/jobs")).items.find(
      (j) => j.id === jid,
    );
    if (job && predicate(job)) return job;
    if (job?.status === "failed") throw new Error(JSON.stringify(job));
    await sleep(120);
  }
  throw new Error("Job wait timed out");
}
process.on("exit", () => child?.kill());
try {
  await start();
  assert.equal((await fetch(connection.endpoint + "/v1/projects")).status, 401);
  checks.push("API rejects unauthenticated requests");
  const duplicateLog = await open(resolve(runDir, "duplicate.log"), "w");
  const duplicate = spawn(binary, ["serve", "--data-dir", dataDir], {
    windowsHide: true,
    stdio: ["ignore", duplicateLog.fd, duplicateLog.fd],
  });
  await duplicateLog.close();
  const duplicateCode = await new Promise((resolve) =>
    duplicate.on("exit", resolve),
  );
  assert.notEqual(duplicateCode, 0);
  checks.push("single engine owner enforced");
  const p = await api("/v1/projects", "POST", {
    name: "恢复验证 · 中文项目",
    parent_directory: null,
  });
  const p2 = await api("/v1/projects", "POST", {
    name: "独立项目",
    parent_directory: null,
  });
  const sid = (
    await api("/v1/projects/" + p.id + "/sources", "POST", {
      kind: "demo",
      name: "参考资料",
      index_root: null,
      media_root: null,
    })
  ).id;
  const page1 = await api("/v1/projects/" + p.id + "/assets?limit=7");
  const page2 = await api(
    "/v1/projects/" +
      p.id +
      "/assets?limit=7&cursor=" +
      encodeURIComponent(page1.next_cursor),
  );
  assert.equal(page1.items.length, 7);
  assert.equal(
    new Set([...page1.items, ...page2.items].map((a) => a.key.asset_id)).size,
    14,
  );
  checks.push("bounded cursor paging has no duplicates");
  const media = await fetch(
    connection.endpoint +
      "/v1/projects/" +
      p.id +
      "/sources/" +
      sid +
      "/assets/" +
      page1.items[0].key.asset_id +
      "/media?edge=256",
    { headers: { Authorization: "Bearer " + connection.token } },
  );
  assert.equal(media.status, 200);
  assert.equal(media.headers.get("content-type"), "image/jpeg");
  assert.ok((await media.arrayBuffer()).byteLength > 100);
  checks.push("authenticated binary preview is decoded and resized");
  const foreign = await fetch(
    connection.endpoint +
      "/v1/projects/" +
      p2.id +
      "/sources/" +
      sid +
      "/assets/" +
      page1.items[0].key.asset_id +
      "/media",
    { headers: { Authorization: "Bearer " + connection.token } },
  );
  assert.equal(foreign.status, 404);
  checks.push("cross-project media access is scoped");
  const inspection =
    "/v1/projects/" + p.id + "/sources/" + sid + "/assets/sample-0001";
  const beforeInspection = await api("/v1/projects/" + p.id + "/selection");
  const beforeProject = await api("/v1/projects/" + p.id);
  const metadata = await api(inspection + "/metadata");
  assert.equal(metadata.stored_width, 640);
  assert.equal(metadata.records.length, 1);
  assert.equal("selected" in metadata.object, false);
  const observationPath = "/records/sample-0001/observations";
  const versionQuery = "?version=" + encodeURIComponent(metadata.version.token);
  const observations = await api(inspection + observationPath + versionQuery);
  assert.equal(observations.items[0].relation, "asset_origin");
  assert.equal(observations.items[0].fields[0].value.type, "integer");
  assert.equal(
    (
      await api(
        inspection + observationPath + "/sample-0001/raw" + versionQuery,
      )
    ).status,
    "missing",
  );
  assert.deepEqual(
    await api("/v1/projects/" + p.id + "/selection"),
    beforeInspection,
  );
  assert.equal(
    (await api("/v1/projects/" + p.id)).revision,
    beforeProject.revision,
  );
  checks.push(
    "metadata and raw inspection preserve project selection and revision",
  );
  for (const suffix of [
    "/metadata",
    observationPath + versionQuery,
    observationPath + "/sample-0001/raw" + versionQuery,
  ]) {
    const response = await fetch(
      connection.endpoint + inspection.replace(p.id, p2.id) + suffix,
      {
        headers: { Authorization: "Bearer " + connection.token },
      },
    );
    assert.equal(response.status, 404);
  }
  const changed = await fetch(
    connection.endpoint + inspection + "/metadata?version=old",
    {
      headers: { Authorization: "Bearer " + connection.token },
    },
  );
  assert.equal(changed.status, 409);
  assert.equal((await changed.json()).code, "SOURCE_CHANGED");
  checks.push(
    "all metadata endpoints enforce project scope and report changed versions",
  );
  const all = await api("/v1/projects/" + p.id + "/assets?limit=64");
  const selected = await api("/v1/projects/" + p.id + "/selection", "PATCH", {
    expected_revision: 0,
    add: all.items.map((a) => a.key),
    remove: [],
    clear: false,
  });
  assert.equal(selected.count, 32);
  assert.equal((await api("/v1/projects/" + p2.id + "/selection")).count, 0);
  const collection = await api(
    "/v1/projects/" + p.id + "/collections",
    "POST",
    { name: "固定工作集" },
  );
  const key = crypto.randomUUID();
  const job = await api("/v1/projects/" + p.id + "/jobs", "POST", {
    idempotency_key: key,
    selection_revision: selected.revision,
    delay_ms: 75,
  });
  const duplicateJob = await api("/v1/projects/" + p.id + "/jobs", "POST", {
    idempotency_key: key,
    selection_revision: selected.revision,
    delay_ms: 75,
  });
  assert.equal(duplicateJob.id, job.id);
  checks.push("job submission is idempotent");
  await api("/v1/projects/" + p.id + "/selection", "PATCH", {
    expected_revision: selected.revision,
    add: [],
    remove: [],
    clear: true,
  });
  assert.equal(
    (await api("/v1/projects/" + p.id + "/collections")).items[0].count,
    32,
  );
  await waitJob(
    p.id,
    job.id,
    (j) => j.status === "running" && j.completed >= 8,
  );
  const oldInstance = connection.instance_id;
  await stop();
  const moved = resolve(runDir, "项目 移动后", p.id);
  const plain = (path) =>
    resolve(path.startsWith("\\\\?\\") ? path.slice(4) : path).toLowerCase();
  assert.ok(plain(p.directory).startsWith(plain(runDir) + "\\"));
  assert.ok(plain(moved).startsWith(plain(runDir) + "\\"));
  await mkdir(resolve(moved, ".."), { recursive: true });
  await rename(p.directory, moved);
  p.directory = moved;
  await start();
  await api("/v1/projects/open", "POST", { directory: moved });
  assert.notEqual(connection.instance_id, oldInstance);
  const done = await waitJob(p.id, job.id, (j) => j.status === "succeeded");
  assert.ok(done.attempt >= 2);
  assert.equal(done.completed, 32);
  assert.equal((await api("/v1/projects/" + p.id + "/selection")).count, 0);
  checks.push("engine crash resumes independent worker from checkpoint");
  const output = await fetch(
    connection.endpoint +
      "/v1/projects/" +
      p.id +
      "/jobs/" +
      job.id +
      "/artifact",
    { headers: { Authorization: "Bearer " + connection.token } },
  );
  const rows = (await output.text()).trim().split("\n").map(JSON.parse);
  assert.equal(rows.length, 32);
  assert.equal(new Set(rows.map((r) => r.asset.key.asset_id)).size, 32);
  rows.forEach((row, i) => assert.equal(row.ordinal, i));
  checks.push("published artifact has exactly one row per frozen input");
  const workset = await api(
    "/v1/projects/" +
      p.id +
      "/assets?collection_id=" +
      collection.id +
      "&limit=48",
  );
  assert.equal(workset.items.length, 32);
  checks.push("workset and project reopen after restart");
  const state = await api("/v1/projects/" + p.id + "/selection");
  const again = await api("/v1/projects/" + p.id + "/selection", "PATCH", {
    expected_revision: state.revision,
    add: all.items.map((a) => a.key),
    remove: [],
    clear: false,
  });
  const cancellable = await api("/v1/projects/" + p.id + "/jobs", "POST", {
    idempotency_key: crypto.randomUUID(),
    selection_revision: again.revision,
    delay_ms: 150,
  });
  await waitJob(p.id, cancellable.id, (j) => j.status === "running");
  await api(
    "/v1/projects/" + p.id + "/jobs/" + cancellable.id + "/cancel",
    "POST",
  );
  await sleep(550);
  assert.equal(
    (await api("/v1/projects/" + p.id + "/jobs")).items.find(
      (j) => j.id === cancellable.id,
    ).status,
    "cancelled",
  );
  const noArtifact = await fetch(
    connection.endpoint +
      "/v1/projects/" +
      p.id +
      "/jobs/" +
      cancellable.id +
      "/artifact",
    { headers: { Authorization: "Bearer " + connection.token } },
  );
  assert.equal(noArtifact.status, 404);
  checks.push("cancellation never publishes an incomplete result");
  const abort = new AbortController();
  const stream = await fetch(
    connection.endpoint + "/v1/projects/" + p.id + "/events?after=0",
    {
      headers: { Authorization: "Bearer " + connection.token },
      signal: abort.signal,
    },
  );
  assert.ok(stream.headers.get("content-type").includes("text/event-stream"));
  const reader = stream.body.getReader();
  const part = await reader.read();
  assert.ok(new TextDecoder().decode(part.value).includes("project_id"));
  abort.abort();
  checks.push("project events support cursor replay");
  const report = {
    passed: true,
    checks,
    project_id: p.id,
    project_directory: p.directory,
    job_id: job.id,
    recovered_attempt: done.attempt,
  };
  await writeFile(
    resolve(runDir, "report.json"),
    JSON.stringify(report, null, 2),
  );
  console.log(
    JSON.stringify(
      { passed: true, checks, report: resolve(runDir, "report.json") },
      null,
      2,
    ),
  );
} finally {
  await stop();
}
