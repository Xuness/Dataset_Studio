import { pythonCommand } from "./platform.mjs";
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, readFile, writeFile, stat } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { EngineFixture, sleep } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const runDir = resolve(root, ".local/test-runs", "multibooru-" + Date.now());
await mkdir(runDir, { recursive: true });
const engine = new EngineFixture(root, resolve(runDir, "state"));
const checks = [];
let project;
const sourcePath = (id) => `/v1/projects/${project.id}/sources/${id}`;
async function ready(path) {
  for (let i = 0; i < 300; i++) {
    const response = await engine.response(path);
    const value = await response.json();
    if (response.ok && !value.preparing) return value;
    assert.ok(
      response.ok || value.code === "SOURCE_INDEX_PREPARING",
      JSON.stringify(value),
    );
    await sleep(40);
  }
  throw new Error("source preparation timed out");
}
async function query(ids, conditions) {
  const value = await engine.api(
    `/v1/projects/${project.id}/query-results`,
    "POST",
    {
      spec: {
        version: 3,
        source_ids: ids,
        conditions,
        observation_rule: "current_post",
        order: "asset_key_asc",
      },
    },
  );
  const result = await engine.wait(
    `/v1/projects/${project.id}/query-results/${value.id}`,
    (r) => !["queued", "running"].includes(r.state),
  );
  assert.equal(result.state, "ready", JSON.stringify(result));
  return (
    await engine.api(
      `/v1/projects/${project.id}/query-results/${value.id}/assets?limit=128`,
    )
  ).page;
}
const post = (id) => ({
  field: "post.id",
  operator: "eq",
  value: { type: "integer", value: String(id) },
});
try {
  await promisify(execFile)(
    pythonCommand(),
    [resolve(root, "tooling/multibooru-fixture.py"), runDir],
    { windowsHide: true },
  );
  const fixtures = JSON.parse(
    await readFile(resolve(runDir, "multibooru.json"), "utf8"),
  );
  const watched = Object.values(fixtures).flatMap((f) => [
    resolve(f.lake, "library.json"),
    resolve(f.lake, "indexes/gen-ui/catalog.sqlite"),
    resolve(f.lake, "indexes/gen-ui/analysis.duckdb"),
  ]);
  const before = await Promise.all(
    watched.map(async (file) => {
      const s = await stat(file);
      return [s.size, s.mtimeMs];
    }),
  );
  await engine.start();
  const { StudioClient } = await clientFixture(root, resolve(runDir, "client"));
  const client = new StudioClient(engine.connection);
  const adapters = await client.sourceAccess.adapters();
  assert.deepEqual(adapters.items.map((s) => s.kind).sort(), [
    "danbooru",
    "demo",
    "gelbooru",
    "pixiv",
    "yandere",
  ]);
  project = await engine.api("/v1/projects", "POST", {
    name: "Three sources",
    parent_directory: resolve(runDir, "projects"),
  });
  const sources = {};
  for (const [kind, f] of Object.entries(fixtures)) {
    const body = {
      kind: kind === "danbooru" ? kind : "auto",
      index_root: f.lake,
      media_root: f.lake,
    };
    const probe = await client.sourceAccess.probe(body);
    assert.equal(probe.kind, kind);
    assert.equal(probe.analysis_sequence, "1");
    sources[kind] = await client.attachSource(project.id, {
      ...body,
      name: kind,
    });
    assert.equal(sources[kind].descriptor.site_id, kind);
    assert.equal(sources[kind].descriptor.capabilities.media, true);
    const page = await ready(
      `/v1/projects/${project.id}/assets?source_id=${f.library_id}&order=post_id_asc&limit=8`,
    );
    assert.equal(page.items.length, 8);
    const fields = await client.queries.fields(project.id, f.library_id);
    assert.equal(
      fields.fields.some((v) => v.id === "fav_count"),
      kind === "danbooru",
    );
    if (kind === "danbooru") continue;
    const image = f.images[0];
    const base = sourcePath(f.library_id) + `/assets/${image.sha}`;
    const metadata = await ready(base + "/metadata");
    assert.equal(metadata.stored_width, 32);
    assert.equal(metadata.stored_height, 40);
    const record = metadata.records.find((r) => r.post_id === "10001");
    assert.ok(record);
    const observations = await engine.api(
      base +
        `/records/${record.record_id}/observations?version=${encodeURIComponent(metadata.version.token)}`,
    );
    const observation = observations.items.find(
      (o) => o.relation === "asset_origin",
    );
    assert.deepEqual(
      observation.fields.find((field) => field.name === "tags").value.value,
      f.raw.extra.tags,
    );
    assert.ok(
      observation.fields.some((field) => field.name === `${kind}.score`),
    );
    assert.ok(
      !observation.fields.some((field) => field.name.startsWith("danbooru.")),
    );
    const raw = await engine.api(
      base +
        `/records/${record.record_id}/observations/${observation.observation_id}/raw?version=${encodeURIComponent(metadata.version.token)}`,
    );
    assert.deepEqual(JSON.parse(raw.json), f.raw);
    assert.equal(raw.schema_id, f.schema_id);
    assert.equal(raw.schema.data.toLowerCase(), f.schema_hex);
    for (const tag of f.raw.extra.tags.slice(0, 3)) {
      const page = await query(
        [f.library_id],
        [
          post(10001),
          {
            field: "rating",
            operator: "eq",
            value: { type: "text", value: "g" },
          },
          {
            field: "tags",
            operator: "has_tag",
            value: { type: "text", value: tag },
          },
        ],
      );
      assert.deepEqual(
        page.items.map((v) => v.key.asset_id),
        [image.sha],
      );
    }
    assert.equal(
      (await query([f.library_id], [post(f.missing_post)])).items.length,
      0,
    );
    assert.equal(
      (
        await query(
          [f.library_id],
          [
            post(200007),
            {
              field: "rating",
              operator: "eq",
              value: { type: "text", value: "e" },
            },
          ],
        )
      ).items.length,
      0,
    );
    const media = await engine.response(base + "/media?edge=128");
    assert.equal(media.status, 200);
    assert.equal(media.headers.get("content-type"), "image/jpeg");
    assert.ok((await media.arrayBuffer()).byteLength > 100);
    const scope = {
      project_id: project.id,
      target: {
        kind: "source",
        source_id: f.library_id,
        revision: sources[kind].revision,
      },
    };
    const requirements = await client.sourceAccess.requirements(
      project.id,
      scope,
      ["danbooru_ranking_v1"],
    );
    assert.equal(requirements.supported, false);
    checks.push(
      `${kind}: registration, paging, fields, literal tags, raw/schema, dimensions, missing and duplicate records, preview, tool eligibility`,
    );
  }
  await engine.expectError(
    "/v1/source-probes",
    "POST",
    {
      kind: "danbooru",
      index_root: fixtures.yandere.lake,
      media_root: fixtures.yandere.lake,
    },
    "SOURCE_SITE_MISMATCH",
  );
  await engine.expectError(
    "/v1/source-probes",
    "POST",
    {
      kind: "auto",
      index_root: fixtures.yandere.lake,
      media_root: fixtures.gelbooru.lake,
    },
    "SOURCE_ID_MISMATCH",
  );
  const mixed = await query(
    [sources.yandere.id, sources.gelbooru.id],
    [post(10001)],
  );
  assert.equal(mixed.items.length, 2);
  assert.equal(mixed.items[0].key.asset_id, mixed.items[1].key.asset_id);
  assert.notEqual(mixed.items[0].key.source_id, mixed.items[1].key.source_id);
  checks.push(
    "shared bytes and overlapping post IDs remain separate source identities; mismatched site/roots rejected",
  );
  client.dispose();
  await engine.stop();
  await engine.start();
  await engine.api(`/v1/projects/${project.id}/open`, "POST");
  assert.equal(
    (await engine.api(`/v1/projects/${project.id}/sources`)).items.length,
    3,
  );
  await ready(
    `/v1/projects/${project.id}/assets?source_id=${sources.gelbooru.id}&order=post_id_asc&limit=8`,
  );
  const after = await Promise.all(
    watched.map(async (file) => {
      const s = await stat(file);
      return [s.size, s.mtimeMs];
    }),
  );
  assert.deepEqual(after, before);
  checks.push(
    "restart reuses registration/indexes; authoritative lake files unchanged",
  );
  async function cached(ids, inputScope) {
    const created = await engine.api(
      `/v1/projects/${project.id}/query-results`,
      "POST",
      {
        spec: {
          version: 3,
          source_ids: ids,
          observation_rule: "current_post",
          order: "asset_key_asc",
          conditions: [
            post(10001),
            {
              field: "rating",
              operator: "eq",
              value: { type: "text", value: "g" },
            },
          ],
          ...(inputScope ? { input_scope: inputScope } : {}),
        },
      },
    );
    const result = await engine.wait(
      `/v1/projects/${project.id}/query-results/${created.id}`,
      (r) => !["queued", "running"].includes(r.state),
    );
    assert.equal(result.state, "ready", JSON.stringify(result));
    return result;
  }
  const pair = [sources.yandere.id, sources.gelbooru.id];
  const singleY = await cached([pair[0]]);
  const singleG = await cached([pair[1]]);
  const together = await cached(pair);
  assert.equal(singleY.count, 1);
  assert.equal(singleG.count, 1);
  assert.equal(together.count, 2);
  assert.notEqual(together.cache.mode, "reused");
  assert.equal((await cached([...pair].reverse())).cache.mode, "reused");
  const workset = await engine.api(
    `/v1/projects/${project.id}/collections`,
    "POST",
    {
      name: "Gelbooru only",
      scope: {
        project_id: project.id,
        target: { kind: "query_result", result_id: singleG.id },
      },
    },
  );
  const worksetScope = {
    project_id: project.id,
    target: { kind: "workset", collection_id: workset.id },
  };
  const scopeSources = await engine.api(
    `/v1/projects/${project.id}/source-requirements`,
    "POST",
    { scope: worksetScope, projections: [] },
  );
  assert.deepEqual(
    scopeSources.sources.map((s) => s.source_id),
    [sources.gelbooru.id],
  );
  const limited = await cached(
    scopeSources.sources.map((s) => s.source_id),
    worksetScope,
  );
  assert.equal(limited.count, 1);
  assert.notEqual(limited.cache.mode, "reused");
  const basesBefore = await engine.api("/v1/cache/rating-bases");
  for (const id of pair) {
    assert.ok(
      basesBefore.items.some((b) => b.source_id === id && b.rating === "g"),
    );
  }
  await engine.api(`/v1/cache/rating-bases/${pair[0]}/g/release`, "POST");
  const basesAfter = await engine.api("/v1/cache/rating-bases");
  assert.ok(
    !basesAfter.items.some((b) => b.source_id === pair[0] && b.rating === "g"),
  );
  assert.deepEqual(
    basesAfter.items.filter((b) => b.source_id === pair[1]),
    basesBefore.items.filter((b) => b.source_id === pair[1]),
  );
  checks.push(
    "single and mixed query caches are distinct, reordered source sets reuse, fixed scopes resolve actual sources, and clearing one lake's rating basis preserves the other",
  );

  // Mutate only this owned fixture after the read-only phase's stamp comparison.
  await promisify(execFile)(
    pythonCommand(),
    [
      resolve(root, "tooling/query-fixture-update.py"),
      resolve(runDir, "yandere"),
      "rebuild",
    ],
    { windowsHide: true },
  );
  const refreshed = await cached(pair);
  assert.notEqual(refreshed.cache.mode, "reused");
  assert.equal(refreshed.count, 2);
  const gReused = await cached([pair[1]]);
  assert.equal(gReused.cache.mode, "reused");
  assert.deepEqual(gReused.source_versions, singleG.source_versions);
  assert.equal((await cached([pair[1]], worksetScope)).cache.mode, "reused");
  const current = await engine.api(
    `/v1/projects/${project.id}/query-results/${together.id}/validity`,
  );
  assert.equal(current.current, false);
  checks.push(
    "rebuilding one lake invalidates its mixed query while unrelated single-source and fixed-scope caches remain reusable",
  );
  await writeFile(
    resolve(runDir, "report.json"),
    JSON.stringify({ status: "passed", checks }, null, 2),
  );
  console.log(
    JSON.stringify({
      status: "passed",
      report: resolve(runDir, "report.json"),
      checks: checks.length,
    }),
  );
} finally {
  await engine.stop();
}
