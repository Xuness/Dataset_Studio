import { lakeWorkerPython } from "./lake-worker-runtime.mjs";
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { EngineFixture, sleep } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const runDir = resolve(root, ".local/test-runs", "lake-updates-" + Date.now());
const python = lakeWorkerPython(root);
await mkdir(runDir, { recursive: true });
const engine = new EngineFixture(root, resolve(runDir, "engine"));
const checks = [];
try {
  await engine.start();
  const { StudioClient } = await clientFixture(root, resolve(runDir, "client"));
  let client = new StudioClient(engine.connection);
  assert.equal((await client.lakeUpdates.status()).configured, false);
  checks.push("optional runtime does not affect browsing engine");
  if (python) {
    await promisify(execFile)(
      python,
      [resolve(root, "tooling/lake-updates-fixture.py"), runDir],
      { windowsHide: true },
    );
    const targets = JSON.parse(
      await readFile(resolve(runDir, "targets.json"), "utf8"),
    );
    await client.lakeUpdates.configure({
      python,
      store_root: resolve(runDir, "missing-legacy-store-checkout"),
      state_root: resolve(runDir, "controller"),
    });
    assert.equal((await client.lakeUpdates.capabilities()).items.length, 3);
    checks.push(
      "embedded Studio worker runs when legacy Store checkout path does not exist",
    );
    for (const target of targets) await client.lakeUpdates.register(target);
    assert.equal((await client.lakeUpdates.lakes()).items.length, 3);
    const secret = "integration-only-not-a-real-api-key";
    const credential = await client.lakeUpdates.setCredentials({
      site: "danbooru",
      login: "fixture",
      api_key: secret,
    });
    assert.equal(credential.credential_set, true);
    const status = await client.lakeUpdates.status();
    assert.ok(!JSON.stringify(status).includes(secret));
    const database = await readFile(
      resolve(runDir, "controller/updates.sqlite"),
    );
    assert.ok(!database.includes(Buffer.from(secret)));
    await client.lakeUpdates.clearCredentials("danbooru");
    checks.push(
      "registered three lakes; credentials encrypted and excluded from status",
    );
    const tasks = [];
    for (const target of targets) {
      const definition = {
        library_id: target.library_id,
        range: { kind: "local", start_id: 1, end_id: 10, missing_media: true },
      };
      const preview = await client.lakeUpdates.preview(definition);
      assert.equal(preview.known_candidates, null);
      const key = crypto.randomUUID();
      const task = await client.lakeUpdates.create(definition, key);
      assert.equal(
        (await client.lakeUpdates.create(definition, key)).id,
        task.id,
      );
      tasks.push(task);
    }
    for (const task of tasks) {
      const done = await engine.wait(
        `/v1/lake-updates/jobs/${task.id}`,
        (j) => !["queued", "running"].includes(j.state),
        60000,
      );
      assert.equal(done.state, "completed", JSON.stringify(done));
      assert.equal(done.cursor.metadata_complete, true);
      assert.deepEqual((await client.lakeUpdates.items(task.id)).items, []);
      assert.equal(
        (await client.lakeUpdates.coverage(task.id)).coverage.metadata_complete,
        1,
      );
    }
    checks.push(
      "three concurrent local refresh jobs, bounded empty scopes, idempotent submissions and coverage",
    );
    const frozen = await client.lakeUpdates.createInput({
      library_id: targets[0].library_id,
      provenance: { scope: "integration fixed input" },
    });
    assert.equal(frozen.state, "draft");
    await client.lakeUpdates.appendInput(frozen.id, { post_ids: [11, 11, 12] });
    const sealed = await client.lakeUpdates.sealInput(frozen.id);
    assert.equal(sealed.count, 2);
    assert.equal(sealed.state, "sealed");
    await assert.rejects(
      client.lakeUpdates.appendInput(frozen.id, { post_ids: [13] }),
      (e) => e.code === "UPDATE_CONFLICT",
    );
    const definition = {
      library_id: targets[0].library_id,
      range: { kind: "local", start_id: 1, end_id: 10 },
    };
    const schedule = await client.lakeUpdates.saveSchedule({
      spec: definition,
      every_seconds: 86400,
      first_run_at: "2099-01-01T00:00:00Z",
      enabled: false,
    });
    assert.equal((await client.lakeUpdates.schedules()).items.length, 1);
    await assert.rejects(
      client.lakeUpdates.removeSchedule(schedule.id, schedule.revision + 1),
      (e) => e.code === "REVISION_CONFLICT",
    );
    await engine.stop();
    await engine.start();
    client = new StudioClient(engine.connection);
    assert.equal(
      (await client.lakeUpdates.job(tasks[0].id)).state,
      "completed",
    );
    assert.equal((await client.lakeUpdates.input(frozen.id)).state, "sealed");
    assert.equal(
      (await client.lakeUpdates.schedules()).items[0].enabled,
      false,
    );
    await client.lakeUpdates.removeSchedule(schedule.id, schedule.revision);
    for (
      let i = 0;
      i < 15 && !(await client.lakeUpdates.status()).worker_recent;
      i++
    )
      await sleep(200);
    assert.equal((await client.lakeUpdates.status()).worker_recent, true);
    checks.push(
      "immutable input, schedule revision checks, restart recovery and worker supervision",
    );
  } else {
    checks.push(
      "Worker integration skipped: run setup-lake-worker.ps1 or set STUDIO_LAKE_TEST_PYTHON",
    );
  }
  await writeFile(
    resolve(runDir, "result.json"),
    JSON.stringify({ checks }, null, 2),
  );
  console.log(JSON.stringify({ runDir, checks }));
} finally {
  await engine.stop();
}
