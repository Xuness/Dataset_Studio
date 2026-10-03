import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { EngineFixture } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";
import { lakeWorkerPython } from "./lake-worker-runtime.mjs";
import { verifyLakeWorkerBundle } from "./lake-worker-bundle.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(root, ".local/test-runs", `collections-${Date.now()}`);
await mkdir(run, { recursive: true });
const python = lakeWorkerPython(root);
assert.ok(python, "Install the owned lake-worker development environment");
await promisify(execFile)(
  python,
  [resolve(root, "tooling/collections-fixture.py"), run],
  { windowsHide: true },
);
const refs = JSON.parse(
  await readFile(resolve(run, "collections.json"), "utf8"),
);
const engine = new EngineFixture(root, resolve(run, "engine"));
const checks = [];
try {
  await engine.start();
  const { StudioClient } = await clientFixture(root, resolve(run, "client"));
  const client = new StudioClient(engine.connection);
  assert.equal((await client.sourceCollections.status()).configured, false);
  await client.lakeUpdates.configure({
    python,
    state_root: resolve(run, "control"),
  });
  await verifyLakeWorkerBundle(root, engine.dataDir);
  assert.equal(
    (await client.sourceCollections.capabilities()).contract_version,
    2,
  );
  assert.equal((await client.lakeUpdates.lakes()).items.length, 0);
  assert.equal(
    (await client.sourceCollections.lakes()).items[0].library_id,
    refs.lake.library_id,
  );
  assert.equal(
    (await client.sourceCollections.job(refs.job.id)).progress.media.published,
    4,
  );
  checks.push(
    "one owned worker serves collection control without leaking Pixiv through legacy site enums",
  );
  const project = await engine.api("/v1/projects", "POST", {
    name: "Pixiv integration",
  });
  const source = await client.sourceAccess.attach(project.id, {
    kind: "auto",
    name: "Pixiv fixture",
    media_root: refs.lake.media_root,
    index_root: refs.lake.index_root,
  });
  assert.equal(source.kind, "pixiv");
  assert.equal(source.descriptor.capabilities.literal_tags, true);
  const first = await client.sourceAccess.workMedia(
    project.id,
    source.id,
    "12345",
    { version: refs.first_version, limit: 1 },
  );
  const second = await client.sourceAccess.workMedia(
    project.id,
    source.id,
    "12345",
    { version: first.version, cursor: first.next_cursor, limit: 1 },
  );
  assert.equal(
    first.items[0].bindings[0].object_sha256,
    second.items[0].bindings[0].object_sha256,
  );
  assert.notEqual(
    first.items[0].bindings[0].record_id,
    second.items[0].bindings[0].record_id,
  );
  const latest = await client.sourceAccess.workMedia(
    project.id,
    source.id,
    "12345",
  );
  assert.ok(latest.items.every((item) => item.bindings.length === 0));
  await assert.rejects(
    client.sourceAccess.workMedia(project.id, source.id, "12345", {
      cursor: first.next_cursor,
    }),
    (e) => e.code === "SOURCE_CHANGED",
  );
  checks.push(
    "Python archives are read natively with independent page origins, retained versions and no old-page fallback",
  );
  const author = await client.sourceAccess.author(
    project.id,
    source.id,
    "10109777",
  );
  assert.equal(author.observation.display_name, "紺屋 fixture");
  const works = await client.sourceAccess.authorWorks(
    project.id,
    source.id,
    "10109777",
    { limit: 1 },
  );
  assert.ok(works.next_cursor);
  const works2 = await client.sourceAccess.authorWorks(
    project.id,
    source.id,
    "10109777",
    { version: works.version, cursor: works.next_cursor, limit: 1 },
  );
  assert.notEqual(works.items[0].work_id, works2.items[0].work_id);
  const base = `/v1/projects/${project.id}/sources/${source.id}/assets/${refs.sha256}`;
  const metadata = await engine.api(base + "/metadata");
  assert.equal(metadata.stored_width, 12);
  assert.equal(metadata.records.length, 4);
  const record = metadata.records[0];
  const observations = await engine.api(
    base +
      `/records/${record.record_id}/observations?version=${encodeURIComponent(metadata.version.token)}`,
  );
  assert.ok(observations.items.some((o) => o.relation === "asset_origin"));
  assert.ok(observations.items.some((o) => o.relation === "same_work"));
  const observation = observations.items.find(
    (o) => o.relation === "asset_origin",
  );
  const raw = await engine.api(
    base +
      `/records/${record.record_id}/observations/${observation.observation_id}/raw?version=${encodeURIComponent(metadata.version.token)}`,
  );
  assert.equal(raw.status, "available");
  assert.equal(JSON.parse(raw.json).error, false);
  const image = await engine.response(base + "/media?edge=96");
  assert.equal(image.ok, true);
  assert.match(image.headers.get("content-type"), /^image\//);
  checks.push(
    "existing image, metadata, observation and raw APIs plus author pagination use the new source",
  );
  const spec = (conditions, observation_rule = "any_observation") => ({
    version: 3,
    source_ids: [source.id],
    conditions,
    observation_rule,
    order: "asset_key_asc",
  });
  async function query(conditions, rule) {
    const view = await client.queries.browse(
      project.id,
      spec(conditions, rule),
    );
    return (await client.queries.assets(project.id, view.id, { limit: 10 }))
      .page.items;
  }
  const tag = {
    field: "tags",
    operator: "has_tag",
    value: { type: "text", value: "blue hair" },
  };
  const bookmarks = {
    field: "pixiv.bookmark_count",
    operator: "eq",
    value: { type: "integer", value: "99" },
  };
  assert.equal((await query([tag])).length, 1);
  assert.equal((await query([tag, bookmarks])).length, 0);
  assert.equal(
    (
      await query([
        { ...tag, value: { type: "text", value: "red hair" } },
        bookmarks,
      ])
    ).length,
    1,
  );
  assert.equal((await query([tag], "current_post")).length, 0);
  assert.equal(
    (
      await query([
        {
          field: "stored.width",
          operator: "eq",
          value: { type: "integer", value: "12" },
        },
      ])
    ).length,
    1,
  );
  checks.push(
    "literal-space tags, stored dimensions and all predicates stay on one work/media origin",
  );
  assert.equal(
    (
      await query([
        {
          field: "work.type",
          operator: "eq",
          value: { type: "text", value: "illustration" },
        },
      ])
    ).length,
    1,
  );
  assert.equal(
    (
      await query([
        {
          field: "author.id",
          operator: "eq",
          value: { type: "text", value: "10109777" },
        },
      ])
    ).length,
    1,
  );
  const accountId = crypto.randomUUID();
  const secret = "fixture-cookie-not-a-real-session";
  const account = await client.sourceCollections.saveAccount({
    request_key: crypto.randomUUID(),
    expected_revision: null,
    account_id: accountId,
    label: "unverified fixture",
    mode: "session",
    cookies: [
      {
        name: "PHPSESSID",
        value: secret,
        domain: ".pixiv.net",
        path: "/",
        secure: true,
        http_only: true,
        expires_unix: null,
      },
    ],
  });
  assert.equal(account.state, "unverified");
  await assert.rejects(
    client.sourceCollections.authenticateAccount({
      account_id: accountId,
      request_key: crypto.randomUUID(),
      expected_revision: account.revision,
      mode: "session",
      label: "rejected browser candidate",
      cookies: [],
    }),
    (e) => e.code === "INVALID_INPUT",
  );
  assert.equal(
    (await client.sourceCollections.accounts()).items.find(
      (a) => a.id === accountId,
    ).revision,
    account.revision,
  );
  const accountsFirst = await client.sourceCollections.accounts({ limit: 1 });
  assert.ok(accountsFirst.next_cursor);
  const accountsSecond = await client.sourceCollections.accounts({
    limit: 1,
    cursor: accountsFirst.next_cursor,
  });
  assert.notEqual(accountsFirst.items[0].id, accountsSecond.items[0].id);
  await assert.rejects(
    client.sourceCollections.lakes({ cursor: accountsFirst.next_cursor }),
    (e) => e.code === "INVALID_INPUT",
  );
  const definition = { ...refs.job.definition, account_id: accountId };
  const requestKey = crypto.randomUUID();
  const created = await client.sourceCollections.create(definition, requestKey);
  assert.equal(created.job.state, "waiting_credentials");
  const coalesced = await client.sourceCollections.create(definition);
  assert.equal(coalesced.job.id, created.job.id);
  assert.equal(coalesced.coalesced, true);
  const workspaceFirst = await client.sourceCollections.workspaceJobs({
    limit: 1,
  });
  const workspaceSecond = await client.sourceCollections.workspaceJobs({
    limit: 1,
    cursor: workspaceFirst.next_cursor,
  });
  assert.notEqual(
    workspaceFirst.items[0].job.id,
    workspaceSecond.items[0].job.id,
  );
  assert.equal(
    (await client.sourceCollections.workspaceLakes()).items[0].site,
    "pixiv",
  );
  const scheduleRequest = {
    id: crypto.randomUUID(),
    request_key: crypto.randomUUID(),
    expected_revision: 0,
    definition,
    every_seconds: 86400,
    first_run_at: "2099-01-01T00:00:00Z",
    enabled: false,
  };
  const schedule = await client.sourceCollections.saveSchedule(scheduleRequest);
  assert.equal(
    (await client.sourceCollections.saveSchedule(scheduleRequest)).revision,
    schedule.revision,
  );
  assert.equal(
    (await client.sourceCollections.workspaceSchedules()).items[0].family,
    "collection",
  );
  await client.sourceCollections.removeSchedule(schedule.id, {
    request_key: crypto.randomUUID(),
    expected_revision: schedule.revision,
  });
  assert.equal((await client.sourceCollections.schedules()).items.length, 0);
  checks.push(
    "unified workspace pagination, same-intent coalescing and revisioned recurring-snapshot CRUD use public SDK contracts",
  );
  assert.equal(
    (await client.sourceCollections.create(definition, requestKey)).replayed,
    true,
  );
  const pause = {
    request_key: crypto.randomUUID(),
    expected_revision: created.job.revision,
    action: "pause",
  };
  assert.equal(
    (await client.sourceCollections.action(created.job.id, pause)).job.state,
    "paused",
  );
  assert.equal(
    (await client.sourceCollections.action(created.job.id, pause)).replayed,
    true,
  );
  await assert.rejects(
    client.sourceCollections.action(created.job.id, {
      ...pause,
      request_key: crypto.randomUUID(),
      action: "cancel",
    }),
    (e) => e.code === "REVISION_CONFLICT",
  );
  assert.ok(
    !JSON.stringify(await client.sourceCollections.accounts()).includes(secret),
  );
  const db = await readFile(resolve(run, "control/updates.sqlite"));
  assert.ok(!db.includes(Buffer.from(secret)));
  checks.push(
    "SDK create/action idempotency, revision conflicts, waiting credentials and protected cookie storage round-trip",
  );
  await engine.stop();
  await engine.start();
  const restarted = new StudioClient(engine.connection);
  await restarted.openProject(project.directory);
  assert.equal(
    (
      await restarted.sourceAccess.workMedia(project.id, source.id, "12345", {
        version: refs.first_version,
      })
    ).items[0].bindings[0].object_sha256,
    refs.sha256,
  );
  checks.push(
    "source attachments and retained media snapshots survive an engine restart",
  );
  await writeFile(
    resolve(run, "summary.json"),
    JSON.stringify({ ok: true, checks, run }, null, 2),
  );
  console.log(JSON.stringify({ ok: true, checks, run }));
} finally {
  await engine.stop();
}
