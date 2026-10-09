import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { EngineFixture } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";
import { lakeWorkerPython } from "./lake-worker-runtime.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(root, ".local/test-runs", `pinterest-${Date.now()}`);
await mkdir(run, { recursive: true });
const python = lakeWorkerPython(root);
assert.ok(python);
await promisify(execFile)(
  python,
  [resolve(root, "tooling/pinterest-fixture.py"), run],
  { windowsHide: true },
);
const refs = JSON.parse(await readFile(resolve(run, "pinterest.json"), "utf8"));
const engine = new EngineFixture(root, resolve(run, "engine"));
const checks = [];
try {
  await engine.start();
  const { StudioClient } = await clientFixture(root, resolve(run, "client"));
  const client = new StudioClient(engine.connection);
  await client.lakeUpdates.configure({
    python,
    state_root: resolve(run, "controller"),
  });
  const api = client.pinterestCollections;
  const capabilities = await api.capabilities();
  const activity = await api.status();
  assert.equal(activity.counts.waiting_budget, 1);
  assert.equal(activity.active[0].id, refs.budget_job.id);
  assert.equal(capabilities.contract_version, 1);
  assert.deepEqual(capabilities.seed_kinds, ["pin"]);
  assert.equal(capabilities.discovery, false);
  assert.equal((await api.job(refs.job.id)).state, "completed");
  assert.equal(
    (await client.sourceCollections.workspaceLakes()).items.length,
    0,
  );
  assert.equal(
    (await client.sourceCollections.workspaceJobs()).items.length,
    0,
  );
  assert.equal(
    (await client.sourceCollections.workspaceLakes({ include_pinterest: true }))
      .items[0].site,
    "pinterest",
  );
  const jobs = await client.sourceCollections.workspaceJobs({
    include_pinterest: true,
    limit: 1,
  });
  assert.equal(jobs.items[0].family, "pinterest");
  assert.ok(jobs.next_cursor);
  await assert.rejects(
    client.sourceCollections.workspaceJobs({ cursor: jobs.next_cursor }),
    (e) => e.code === "INVALID_INPUT",
  );
  checks.push(
    "Pinterest commands and bounded opt-in workspace lists preserve legacy families",
  );

  const project = await client.createProject({ name: "Pinterest 集成验证" });
  const source = await client.sourceAccess.attach(project.id, {
    kind: "auto",
    name: "Pinterest 样本",
    media_root: refs.lake.media_root,
    index_root: refs.lake.index_root,
  });
  assert.equal(source.kind, "pinterest");
  assert.equal(source.descriptor.capabilities.post_order, false);
  assert.equal(source.descriptor.capabilities.identity_summaries, true);
  const summaries = await client.assetSummaries(project.id, [
    { source_id: source.id, asset_id: refs.sha256 },
  ]);
  assert.equal(summaries.items[0].summary.status, "available");
  assert.equal(summaries.items[0].summary.site_name, "Pinterest");
  assert.ok(
    summaries.items[0].summary.post_ids.includes(refs.records[0].pin_id),
  );
  assert.equal(source.descriptor.capabilities.author_metadata, false);
  const base = `/v1/projects/${project.id}/sources/${source.id}/assets/${refs.sha256}`;
  const metadata = await engine.api(
    base +
      `/metadata?limit=1&version=${encodeURIComponent(refs.first_version)}`,
  );
  assert.equal(metadata.stored_width, 8);
  assert.equal(metadata.stored_height, 6);
  assert.equal(metadata.records.length, 1);
  assert.ok(metadata.next_cursor);
  const second = await engine.api(
    base +
      `/metadata?limit=1&version=${encodeURIComponent(metadata.version.token)}&cursor=${encodeURIComponent(metadata.next_cursor)}`,
  );
  const record = metadata.records[0];
  assert.notEqual(
    record.pin_origin.pin_id,
    second.records[0].pin_origin.pin_id,
  );
  assert.equal(record.work_id, null);
  assert.equal(record.source_md5, null);
  const obs = await engine.api(
    base +
      `/records/${record.record_id}/observations?version=${encodeURIComponent(metadata.version.token)}`,
  );
  const pin = obs.items.find((v) => v.source_kind === "pinterest_pin_detail");
  assert.ok(pin);
  const field = (name) => pin.fields.find((v) => v.name === name);
  assert.equal(field("pinterest.description").missing_reason, "explicit_null");
  assert.equal(field("pinterest.link").missing_reason, "not_in_response");
  assert.equal(field("pinterest.is_ai_generated").value.value, false);
  assert.equal(field("pinterest.repin_count").value.value, "0");
  assert.match(field("pinterest.pinner").value.value, /account 2001/);
  const raw = await engine.api(
    base +
      `/records/${record.record_id}/observations/${pin.observation_id}/raw?version=${encodeURIComponent(metadata.version.token)}`,
  );
  assert.equal(
    JSON.parse(raw.json).resource_response.data.id,
    record.pin_origin.pin_id,
  );
  const wrong = refs.records.find((v) => v.asset_id !== record.record_id);
  await engine.expectError(
    base +
      `/records/${record.record_id}/observations/${wrong.observation_id}/raw?version=${encodeURIComponent(metadata.version.token)}`,
    "GET",
    undefined,
    "NOT_FOUND",
  );
  const image = await engine.response(base + "/media?edge=96");
  assert.equal(image.ok, true);
  assert.match(image.headers.get("content-type"), /^image\//);
  const original = await engine.response(base + "/original");
  assert.equal(original.ok, true);
  assert.equal(
    Buffer.from(await original.arrayBuffer()).toString("hex"),
    refs.original_hex,
  );
  const spec = {
    version: 3,
    source_ids: [source.id],
    conditions: [],
    observation_rule: "any_observation",
    order: "asset_key_asc",
  };
  const view = await client.queries.browse(project.id, spec);
  assert.equal(
    (await client.queries.assets(project.id, view.id, { limit: 10 })).page.items
      .length,
    1,
  );
  const filtered = await client.queries.browse(project.id, {
    ...spec,
    conditions: [
      {
        field: "stored.width",
        operator: "eq",
        value: { type: "integer", value: "8" },
      },
    ],
  });
  assert.equal(
    (await client.queries.assets(project.id, filtered.id, { limit: 10 })).page
      .items.length,
    1,
  );
  await assert.rejects(
    client.queries.browse(project.id, { ...spec, order: "post_id_desc" }),
    (e) => e.code === "QUERY_UNSUPPORTED",
  );
  checks.push(
    "shared bytes retain separate Pin origins, bounded metadata/raw ownership and native image/storage queries",
  );

  const preview = await api.preview(refs.budget_job.definition);
  assert.equal(preview.network_requests, 0);
  const key = crypto.randomUUID();
  const created = await api.create(preview.definition, key);
  assert.equal(created.id, refs.budget_job.id);
  assert.equal((await api.create(preview.definition, key)).id, created.id);
  await assert.rejects(
    api.create(
      { ...preview.definition, seeds: [{ kind: "pin", id: "999" }] },
      key,
    ),
    (e) => e.code === "PINTEREST_IDEMPOTENCY_CONFLICT",
  );
  const paused = await api.action(created.id, {
    action: "pause",
    expected_revision: created.revision,
  });
  assert.equal(paused.desired_state, "paused");
  await assert.rejects(
    api.action(created.id, {
      action: "cancel",
      expected_revision: created.revision,
    }),
    (e) => e.code === "PINTEREST_CONFLICT",
  );
  const settled = await engine.wait(
    `/v1/pinterest-collections/jobs/${created.id}`,
    (v) => v.state === "paused",
  );
  await api.action(created.id, {
    action: "cancel",
    expected_revision: settled.revision,
  });
  await engine.wait(
    `/v1/pinterest-collections/jobs/${created.id}`,
    (v) => v.state === "cancelled",
  );
  const itemPage = await api.items(refs.job.id, { limit: 1 });
  assert.ok(itemPage.next_cursor);
  const itemNext = await api.items(refs.job.id, {
    cursor: itemPage.next_cursor,
    limit: 1,
  });
  assert.notEqual(itemPage.items[0].task_id, itemNext.items[0].task_id);
  checks.push(
    "preview, idempotent create, optimistic pause/cancel and task paging round-trip without source requests",
  );

  const lake = await api.createLake({
    request_key: crypto.randomUUID(),
    site: "pinterest",
    media_root: resolve(run, "empty-media"),
    index_root: resolve(run, "empty-index"),
  });
  const empty = await client.sourceAccess.attach(project.id, {
    kind: "auto",
    name: "Pinterest 空湖",
    media_root: lake.media_root,
    index_root: lake.index_root,
  });
  assert.equal(empty.kind, "pinterest");
  const emptyView = await client.queries.browse(project.id, {
    ...spec,
    source_ids: [empty.id],
  });
  assert.equal(
    (await client.queries.assets(project.id, emptyView.id, { limit: 10 })).page
      .items.length,
    0,
  );
  await engine.stop();
  await engine.start();
  const restarted = new StudioClient(engine.connection);
  await restarted.openProject(project.directory);
  assert.equal(
    (await restarted.pinterestCollections.job(refs.job.id)).state,
    "completed",
  );
  assert.equal(
    (
      await engine.api(
        base + `/metadata?version=${encodeURIComponent(refs.first_version)}`,
      )
    ).records.length,
    2,
  );
  checks.push(
    "empty lakes and retained source attachments survive engine restart",
  );
  await writeFile(
    resolve(run, "summary.json"),
    JSON.stringify({ ok: true, checks, run, project }, null, 2),
  );
  console.log(JSON.stringify({ ok: true, checks, run }));
} finally {
  await engine.stop();
}
