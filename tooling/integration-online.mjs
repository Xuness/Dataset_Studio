import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { EngineFixture } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(root, ".local/test-runs", "online-" + Date.now());
await mkdir(run, { recursive: true });
const fixture = (action = "create") =>
  promisify(execFile)(
    process.env.PYTHON ?? "python",
    [resolve(root, "tooling/online-fixture.py"), run, action],
    { windowsHide: true },
  );
const engine = new EngineFixture(root, resolve(run, "state"));
const checks = [];
try {
  await fixture();
  const refs = JSON.parse(
    await readFile(resolve(run, "multibooru.json"), "utf8"),
  );
  await engine.start();
  const { StudioClient } = await clientFixture(root, resolve(run, "client"));
  const client = new StudioClient(engine.connection);
  const project = await engine.api("/v1/projects", "POST", {
    name: "Online three lakes",
  });
  const base = "/v1/projects/" + project.id;
  const ids = [];
  for (const [site, ref] of Object.entries(refs)) {
    const source = await client.attachSource(project.id, {
      kind: "auto",
      name: site,
      index_root: ref.lake,
      media_root: ref.lake,
    });
    ids.push(source.id);
    assert.equal(source.kind, site);
    assert.equal(
      (await client.queries.fields(project.id, source.id)).direct_query,
      true,
    );
  }
  checks.push(
    "auto-detect all three online lakes without a native DuckDB file",
  );
  const spec = (order = "post_id_asc", conditions = [], source_ids = ids) => ({
    version: 3,
    source_ids,
    conditions,
    observation_rule: "current_post",
    order,
  });
  async function pages(result, limit = 7) {
    const items = [];
    let cursor;
    for (let i = 0; i < 200; i++) {
      const response = await client.queries.assets(project.id, result.id, {
        limit,
        ...(cursor ? { cursor } : {}),
      });
      assert.equal(
        response.count ?? null,
        result.cache.mode === "view" ? null : result.count,
      );
      items.push(...response.page.items);
      cursor = response.page.next_cursor;
      if (!cursor) return items;
    }
    throw new Error("pagination did not finish");
  }
  for (const order of [
    "asset_key_asc",
    "asset_key_desc",
    "post_id_asc",
    "post_id_desc",
  ]) {
    const view = await client.queries.browse(project.id, spec(order));
    assert.equal(view.state, "ready");
    assert.equal(view.count, null);
    const items = await pages(view);
    const identity = (v) => v.key.source_id + "/" + v.key.asset_id;
    assert.equal(new Set(items.map(identity)).size, items.length);
    assert.equal(
      items.length,
      Object.values(refs).reduce((n, r) => n + r.images.length, 0),
    );
    const ordered = [...items].sort((a, b) => {
      const pa = Math.min(...(a.summary?.post_ids ?? []).map(Number));
      const pb = Math.min(...(b.summary?.post_ids ?? []).map(Number));
      if (
        order.startsWith("post") &&
        Number.isFinite(pa) !== Number.isFinite(pb)
      )
        return Number.isFinite(pa) ? -1 : 1;
      const cmp =
        order.startsWith("post") && pa !== pb
          ? pa - pb
          : identity(a).localeCompare(identity(b));
      return order.endsWith("desc") ? -cmp : cmp;
    });
    assert.deepEqual(items.map(identity), ordered.map(identity), order);
  }
  checks.push(
    "multi-lake keyset merge preserves all four orders and equal-post ties",
  );
  const tag = refs.gelbooru.raw.extra.tags[1];
  const literal = await client.queries.browse(
    project.id,
    spec("post_id_asc", [
      {
        field: "tags",
        operator: "has_tag",
        value: { type: "text", value: tag },
      },
    ]),
  );
  assert.equal((await pages(literal)).length, 2);
  checks.push("literal tab-bearing tags remain exact across lakes");
  const post = {
    field: "post.id",
    operator: "eq",
    value: { type: "integer", value: "10001" },
  };
  const old = await client.queries.browse(
    project.id,
    spec("post_id_asc", [post], [refs.danbooru.library_id]),
  );
  const first = (await pages(old))[0];
  async function score(version) {
    const overview = await client.metadata(project.id, first.key, { version });
    const record = overview.records.find((r) => r.post_id === "10001");
    const obs = await client.observations(
      project.id,
      first.key,
      record.record_id,
      { version: overview.version.token },
    );
    return obs.items.map((o) =>
      Number(o.fields.find((f) => f.name === "danbooru.score")?.value?.value),
    );
  }
  const before = await score(first.summary.version);
  await fixture("stage");
  assert.deepEqual(await score(first.summary.version), before);
  assert.equal(
    (await client.queries.validity(project.id, old.id)).newer_available,
    false,
  );
  await fixture("publish");
  assert.deepEqual(await score(first.summary.version), before);
  const validity = await client.queries.validity(project.id, old.id);
  assert.equal(validity.current, true);
  assert.equal(validity.newer_available, true);
  const fresh = await client.queries.browse(project.id, old.spec);
  const freshAsset = (await pages(fresh))[0];
  assert.ok((await score(freshAsset.summary.version)).includes(9999));
  checks.push(
    "staged writes stay invisible, old details survive publication, latest view sees update",
  );
  const collection = await client.createCollection(
    project.id,
    "Captured old view",
    {
      project_id: project.id,
      target: { kind: "query_result", result_id: old.id },
    },
  );
  assert.equal(collection.count, 1);
  const fixed = await client.queries.run(project.id, spec("post_id_desc"));
  const ready = await engine.wait(
    base + "/query-results/" + fixed.id,
    (r) => !["running", "queued"].includes(r.state),
  );
  assert.equal(ready.state, "ready", JSON.stringify(ready));
  assert.equal((await pages(ready)).length, ready.count);
  const scopedJob = await engine.api(base + "/jobs", "POST", {
    idempotency_key: crypto.randomUUID(),
    delay_ms: 0,
    scope: {
      project_id: project.id,
      target: { kind: "query_result", result_id: old.id },
    },
  });
  await engine.wait(base + "/jobs", (r) =>
    r.items.some(
      (j) =>
        j.id === scopedJob.id && ["failed", "succeeded"].includes(j.status),
    ),
  );
  const job = (await engine.api(base + "/jobs")).items.find(
    (j) => j.id === scopedJob.id,
  );
  assert.equal(job.status, "succeeded", JSON.stringify(job));
  const control = new DatabaseSync(
    resolve(project.directory, "project.sqlite"),
    { readOnly: true },
  );
  assert.equal(
    control.prepare("SELECT count(*) n FROM query_member_data").get().n,
    0,
  );
  assert.equal(
    control.prepare("SELECT count(*) n FROM collection_member_legacy").get().n,
    0,
  );
  assert.equal(
    control.prepare("SELECT count(*) n FROM job_input_legacy").get().n,
    0,
  );
  control.close();
  checks.push(
    "capture, fixed post order, workset and job share sealed members outside control DB",
  );
  for (const variant of [1, 2]) {
    const rank = await engine.api(base + "/tools/jobs", "POST", {
      idempotency_key: crypto.randomUUID(),
      delay_ms: 0,
      scope: {
        project_id: project.id,
        target: { kind: "query_result", result_id: old.id },
      },
      run: {
        operator_id:
          variant === 1 ? "danbooru.metarecall" : "danbooru.metarecall_v2",
        operator_version: 1,
        parameters_version: 1,
        parameters: {
          ratings: ["g", "s", "q", "e"],
          cohort_minimum: 8,
          artist_enabled: false,
          ...(variant === 2
            ? {
                v2: {
                  profiles: Object.fromEntries(
                    ["g", "s", "q", "e"].map((rating) => [
                      rating,
                      {
                        time_up: 0.3,
                        time_down: 0,
                        vote_weight: 0.08,
                        era_weight: 0.3,
                      },
                    ]),
                  ),
                  feather_days: 90,
                  minimum_effective: 2,
                  comic_penalty: 0,
                  keep_per_mille: 500,
                  direct_rescue: 50,
                  era_rescue: 50,
                  audit: 30,
                  eras: [],
                  strict_era_targets: false,
                },
              }
            : {}),
        },
      },
    });
    const ranked = await engine.wait(
      base + "/jobs",
      (r) =>
        r.items.some(
          (j) => j.id === rank.id && ["failed", "succeeded"].includes(j.status),
        ),
      60000,
    );
    assert.equal(
      ranked.items.find((j) => j.id === rank.id).status,
      "succeeded",
      JSON.stringify(ranked),
    );
    const frozen = await engine.api(base + "/jobs/" + rank.id + "/run");
    assert.equal(
      frozen.source_versions[0].catalog_revision,
      old.source_versions[0].catalog_revision,
    );
  }
  checks.push(
    "ranking v1 and v2 capture retained SQLite facts without native lake DuckDB",
  );
  await engine.stop();
  await engine.start();
  await engine.api("/v1/projects/open", "POST", {
    directory: project.directory,
  });
  assert.equal(
    (await engine.api(base + "/query-results/" + ready.id + "/assets")).page
      .items.length > 0,
    true,
  );
  checks.push("sealed results survive engine restart");
  await writeFile(
    resolve(run, "result.json"),
    JSON.stringify({ checks, project, fixtures: refs }, null, 2),
  );
  console.log(JSON.stringify({ run, checks }));
} finally {
  await engine.stop();
}
