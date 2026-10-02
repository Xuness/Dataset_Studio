import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, readFile, writeFile, cp, rename } from "node:fs/promises";
import { resolve, basename, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { EngineFixture, sleep } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";
import { lakeWorkerPython } from "./lake-worker-runtime.mjs";
import { verifyLakeWorkerBundle } from "./lake-worker-bundle.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(root, ".local/test-runs/lake-recovery-" + Date.now());
const python = lakeWorkerPython(root);
assert.ok(python, "Lake worker test Python required");
await mkdir(run, { recursive: true });
const engine = new EngineFixture(root, resolve(run, "engine"));
const checks = [];
const crossVolume = process.env.STUDIO_RELOCATION_TEST_ROOT;
if (crossVolume) {
  assert.ok(basename(crossVolume).startsWith("r3-cross-volume-"));
  assert.equal(basename(dirname(crossVolume)), "test-runs");
  await mkdir(crossVolume, { recursive: true });
  await writeFile(
    resolve(run, "external-fixture.json"),
    JSON.stringify({ root: resolve(crossVolume) }),
  );
}
const execute = (file, args) =>
  promisify(execFile)(file, args, { windowsHide: true, timeout: 120000 });
try {
  await execute(python, [
    resolve(root, "tooling/lake-updates-fixture.py"),
    run,
    "--assets",
  ]);
  const targets = JSON.parse(
    await readFile(resolve(run, "targets.json"), "utf8"),
  );
  await engine.start();
  const { StudioClient } = await clientFixture(root, resolve(run, "client"));
  let client = new StudioClient(engine.connection);
  const bad = resolve(run, "invalid.exe");
  await writeFile(bad, "not an executable");
  const runtime = { python, state_root: resolve(run, "controller") };
  await assert.rejects(
    client.lakeUpdates.configure({ ...runtime, python: bad }),
  );
  assert.equal((await client.lakeUpdates.status()).configured, false);
  await execute(python, [
    "-m",
    "venv",
    "--without-pip",
    resolve(run, "empty-python"),
  ]);
  const empty = resolve(
    run,
    "empty-python",
    process.platform === "win32" ? "Scripts/python.exe" : "bin/python",
  );
  await assert.rejects(
    client.lakeUpdates.configure({ ...runtime, python: empty }),
  );
  assert.equal((await client.lakeUpdates.status()).configured, false);
  const foreign = resolve(run, "foreign-state"),
    foreignApp = resolve(run, "foreign-app");
  await mkdir(foreign, { recursive: true });
  await mkdir(foreignApp, { recursive: true });
  await writeFile(
    resolve(foreign, "studio-owner.json"),
    JSON.stringify({ registry: foreignApp }),
  );
  await assert.rejects(
    client.lakeUpdates.configure({ ...runtime, state_root: foreign }),
    (e) => e.code === "UPDATE_CONFLICT",
  );
  const brokenState = resolve(run, "broken-state");
  await mkdir(brokenState, { recursive: true });
  await writeFile(
    resolve(brokenState, "updates.sqlite"),
    "invalid state database",
  );
  await assert.rejects(
    client.lakeUpdates.configure({ ...runtime, state_root: brokenState }),
  );
  await assert.rejects(
    readFile(resolve(brokenState, "studio-owner.json")),
    (e) => e.code === "ENOENT",
  );
  await client.lakeUpdates.configure(runtime);
  assert.equal((await client.lakeUpdates.status()).runtime.state, "ready");
  checks.push(
    "invalid executable and missing dependencies never save a candidate; status and repair remain available",
  );
  const bundle = await verifyLakeWorkerBundle(root, engine.dataDir);
  await writeFile(
    resolve(run, "worker-bundle.json"),
    JSON.stringify(bundle, null, 2),
  );
  const project = await client.createProject({
    name: "Coordinated relocation",
    parent_directory: resolve(run, "projects"),
  });
  const second = await client.createProject({
    name: "Second location reader",
    parent_directory: resolve(run, "projects"),
  });
  for (const target of targets) {
    await client.lakeUpdates.register(target);
    for (const p of [project, second]) {
      await client.attachSource(p.id, {
        kind: "auto",
        name: target.site,
        index_root: target.index_root,
        media_root: target.media_root,
      });
    }
  }
  await client.lakeUpdates.setCredentials({
    site: "danbooru",
    login: "fixture",
    api_key: "only-a-fixture-secret",
  });
  const jobs = (await client.lakeUpdates.jobs({})).items
    .map((j) => j.id)
    .sort();
  await execute(python, [
    "-m",
    "venv",
    "--without-pip",
    "--system-site-packages",
    resolve(run, "replacement-python"),
  ]);
  const sitePaths = JSON.parse(
    (
      await execute(python, [
        "-c",
        "import site,json;print(json.dumps(site.getsitepackages()))",
      ])
    ).stdout,
  );
  await writeFile(
    resolve(run, "replacement-python/Lib/site-packages/studio-tests.pth"),
    sitePaths.join("\n") + "\n",
  );
  const replacement = resolve(
    run,
    "replacement-python",
    process.platform === "win32" ? "Scripts/python.exe" : "bin/python",
  );
  await Promise.all([
    client.lakeUpdates.configure({ ...runtime, python: replacement }),
    client.lakeUpdates.configure({ ...runtime, python: replacement }),
  ]);
  assert.deepEqual(
    (await client.lakeUpdates.jobs({})).items.map((j) => j.id).sort(),
    jobs,
  );
  assert.ok(
    (await client.lakeUpdates.status()).credentials.some(
      (c) => c.credential_set,
    ),
  );
  await assert.rejects(
    client.lakeUpdates.configure({
      ...runtime,
      state_root: resolve(run, "other-state"),
    }),
    (e) => e.code === "UPDATE_CONFLICT",
  );
  await assert.rejects(
    client.lakeUpdates.configure({ ...runtime, python: empty }),
  );
  assert.ok(
    (await client.lakeUpdates.status()).runtime.python
      .toLowerCase()
      .endsWith("python.exe") || process.platform !== "win32",
  );
  assert.equal((await client.lakeUpdates.lakes()).items.length, 3);
  checks.push(
    "serialized interpreter replacement preserves tasks, credentials and lake registrations; rejected candidates preserve working runtime",
  );
  const registered = targets[0];
  await assert.rejects(
    client.relinkSource(project.id, registered.library_id, {
      media_root: registered.media_root,
      index_root: registered.index_root,
    }),
    (e) => e.code === "UPDATE_CONFLICT",
  );
  for (let i = 0; i < targets.length; i++) {
    const target = targets[i];
    const mode = ["media", "index", "both"][i];
    const destination = mode === "both" && crossVolume ? crossVolume : run;
    // Index-only reconnect also works when its old directory is already offline.
    let move =
      mode === "index"
        ? null
        : await client.lakeUpdates.prepareRelocation(target.library_id);
    const media_root =
      mode === "index"
        ? target.media_root
        : resolve(destination, "new-media-" + i);
    const index_root =
      mode === "media"
        ? target.index_root
        : resolve(destination, "new-index-" + i);
    if (media_root !== target.media_root) {
      await cp(target.media_root, media_root, { recursive: true });
      await rename(target.media_root, target.media_root + "-offline");
    }
    if (index_root !== target.index_root) {
      await cp(target.index_root, index_root, { recursive: true });
      await rename(target.index_root, target.index_root + "-offline");
    }
    move ??= await client.lakeUpdates.prepareRelocation(target.library_id);
    assert.equal(move.phase, "prepared");
    if (mode === "both") {
      // Simulate the process ending after Python commits but before Rust updates readers.
      await engine.stop();
      await execute(python, [
        "-c",
        "import sys;sys.path.insert(0,sys.argv[1]);from studio_lake.updates.state import State;from studio_lake.updates.relocation import apply;apply(State(sys.argv[2]),*sys.argv[3:])",
        resolve(root, "services/lake-worker/src"),
        runtime.state_root,
        move.id,
        media_root,
        index_root,
      ]);
      await engine.start();
      client = new StudioClient(engine.connection);
      await engine.wait(
        "/v1/lake-updates/relocations",
        (r) => r.items.length === 0,
      );
      await client.openRecentProject(project.id);
      await client.openRecentProject(second.id);
    } else {
      const result = await client.lakeUpdates.applyRelocation(move.id, {
        media_root,
        index_root,
      });
      assert.equal(result.phase, "complete");
    }
    for (const p of [project, second]) {
      const source = (await client.sources(p.id)).items.find(
        (s) => s.id === target.library_id,
      );
      assert.equal(source.available, true);
      const registry = new DatabaseSync(
        resolve(engine.dataDir, "registry.sqlite"),
        { readOnly: true },
      );
      const location = JSON.parse(
        registry
          .prepare("SELECT json FROM source_locations WHERE id=?")
          .get(source.id).json,
      );
      registry.close();
      const plain = (v) =>
        v
          .replace(/^\\\\\?\\/, "")
          .replaceAll("\\", "/")
          .toLowerCase();
      assert.equal(plain(location.media_root), plain(media_root));
      assert.equal(plain(location.index_root), plain(index_root));
    }
    const fixed = await client.lakeUpdates.createInput({
      library_id: target.library_id,
      provenance: {},
    });
    await client.lakeUpdates.appendInput(fixed.id, { post_ids: [11] });
    await client.lakeUpdates.sealInput(fixed.id);
  }
  checks.push(
    "media-only, index-only and combined moves work with old paths offline; both open projects and sealed input preparation share new roots",
  );
  await engine.stop();
  const saved = JSON.parse(
    await readFile(resolve(engine.dataDir, "lake-update-runtime.json"), "utf8"),
  );
  await writeFile(
    resolve(engine.dataDir, "lake-update-runtime.json"),
    JSON.stringify({ ...saved, python: bad }),
  );
  await engine.start();
  client = new StudioClient(engine.connection);
  const broken = await client.lakeUpdates.status();
  assert.equal(broken.configured, true);
  assert.equal(broken.worker_recent, false);
  assert.equal(broken.runtime.state, "failed");
  assert.ok(broken.runtime.next_retry_ms > Date.now());
  await client.lakeUpdates.configure(runtime);
  assert.deepEqual(
    (await client.lakeUpdates.jobs({})).items.map((j) => j.id).sort(),
    jobs,
  );
  assert.equal((await client.lakeUpdates.relocations()).items.length, 0);
  assert.ok(
    !JSON.stringify(await client.lakeUpdates.status()).includes(
      "only-a-fixture-secret",
    ),
  );
  checks.push(
    "saved broken runtime recovers after engine restart without losing state; completed relocations persist",
  );
  await engine.stop();
  const legacy = new DatabaseSync(
    resolve(runtime.state_root, "updates.sqlite"),
  );
  try {
    legacy.exec(
      "BEGIN IMMEDIATE; DROP TABLE lake_dispatch; PRAGMA user_version=6; COMMIT;",
    );
  } finally {
    legacy.close();
  }
  const oldHeartbeat = await readFile(
    resolve(runtime.state_root, "heartbeat.json"),
    "utf8",
  );
  await engine.start();
  client = new StudioClient(engine.connection);
  await engine.wait(
    "/v1/lake-updates/status",
    (s) => s.runtime.state === "ready" && s.worker_recent,
  );
  // A successful handshake alone missed the original first-scheduler-tick crash.
  await sleep(2200);
  const repaired = await client.lakeUpdates.status();
  assert.equal(repaired.runtime.state, "ready");
  assert.equal(repaired.runtime.failures, 0);
  assert.equal(repaired.worker_recent, true);
  assert.notEqual(
    await readFile(resolve(runtime.state_root, "heartbeat.json"), "utf8"),
    oldHeartbeat,
  );
  assert.equal((await client.lakeUpdates.lakes()).items.length, 3);
  assert.equal((await client.lakeUpdates.relocations()).items.length, 0);
  assert.ok(repaired.credentials.some((c) => c.credential_set));
  const upgraded = new DatabaseSync(
    resolve(runtime.state_root, "updates.sqlite"),
    { readOnly: true },
  );
  try {
    assert.equal(upgraded.prepare("PRAGMA user_version").get().user_version, 7);
    assert.equal(
      upgraded.prepare("SELECT count(*) AS n FROM lake_dispatch").get().n,
      0,
    );
    assert.equal(
      upgraded
        .prepare("SELECT count(*) AS n FROM inputs WHERE state='sealed'")
        .get().n,
      3,
    );
  } finally {
    upgraded.close();
  }
  checks.push(
    "legacy v6 without lake_dispatch upgrades at startup; worker advances heartbeats and retains credentials, lakes, fixed inputs and completed relocations",
  );
} finally {
  await engine.stop();
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify({ checks }, null, 2),
  );
}
console.log(JSON.stringify({ run, checks }, null, 2));
