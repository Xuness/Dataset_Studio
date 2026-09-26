// Opt-in real-lake verification. Every write belongs to an isolated test application.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile, stat, readdir } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { performance } from "node:perf_hooks";
import { EngineFixture, sleep } from "./engine-fixture.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
assert.ok(process.argv[2], "Pass an explicit bounded lake/sample manifest");
const lakes = JSON.parse(await readFile(resolve(process.argv[2]), "utf8"));
assert.ok(lakes.length >= 1 && lakes.length <= 3);
assert.ok(
  lakes.every(
    (l) =>
      l.samples.length >= 1 &&
      l.samples.length <= 64 &&
      l.samples.filter((s) => s.preview).length <= 8,
  ),
);
const run = resolve(root, ".local/test-runs/real-multibooru-" + Date.now());
await mkdir(run, { recursive: true });
const engine = new EngineFixture(root, resolve(run, "state"));
const report = {
  status: "running",
  scope:
    "bounded API metadata and media samples; cold derived source indexes; no whole-media scan",
  lakes: [],
};
const digest = (text) => createHash("sha256").update(text).digest("hex");
async function progress(phase) {
  await writeFile(
    resolve(run, "progress.json"),
    JSON.stringify({ phase, at: new Date().toISOString(), report }, null, 2),
  );
  console.log(JSON.stringify({ phase, run }));
}
async function ready(path) {
  const start = performance.now();
  while (performance.now() - start < 20 * 60 * 1000) {
    const response = await engine.response(path);
    const value = await response.json();
    if (response.ok && !value.preparing) return value;
    assert.ok(
      response.ok ||
        ["SOURCE_INDEX_PREPARING", "SOURCE_BUSY"].includes(value.code),
      JSON.stringify(value),
    );
    await sleep(1500);
  }
  throw new Error("source preparation exceeded 20 minutes");
}
async function bytes(directory) {
  let total = 0;
  for (const e of await readdir(directory, { withFileTypes: true }).catch(
    () => [],
  )) {
    const p = resolve(directory, e.name);
    if (e.isDirectory()) total += await bytes(p);
    else if (e.isFile()) total += (await stat(p)).size;
  }
  return total;
}
try {
  const watched = lakes.flatMap((l) => [
    resolve(l.media_root, "library.json"),
    resolve(l.index_root, "CURRENT.json"),
    resolve(l.index_root, "cache_owner.json"),
    resolve(l.index_root, "indexes", l.generation, "catalog.sqlite"),
    resolve(l.index_root, "indexes", l.generation, "analysis.duckdb"),
  ]);
  const stamps = () =>
    Promise.all(
      watched.map(async (path) => {
        const s = await stat(path, { bigint: true });
        return [path, s.size.toString(), s.mtimeNs.toString()];
      }),
    );
  const before = await stamps();
  await engine.start();
  const project = await engine.api("/v1/projects", "POST", {
    name: "Isolated real lake verification",
    parent_directory: resolve(run, "projects"),
  });
  const pid = project.id;
  for (const lake of lakes) {
    const entry = {
      name: lake.name,
      records: 0,
      literal_tags: 0,
      previews: 0,
      query_seconds: [],
    };
    report.lakes.push(entry);
    await progress(lake.name + ": probe and registration");
    const body = {
      kind: "auto",
      index_root: lake.index_root,
      media_root: lake.media_root,
    };
    const probe = await engine.api("/v1/source-probes", "POST", body);
    assert.equal(probe.source_id, lake.library_id);
    assert.equal(probe.kind, lake.name.toLowerCase());
    const source = await engine.api(`/v1/projects/${pid}/sources`, "POST", {
      ...body,
      name: lake.name,
    });
    assert.equal(source.descriptor.backend_id, "canonical_lake_v1");
    const prefix = `/v1/projects/${pid}/sources/${source.id}`;
    let started = performance.now();
    await progress(lake.name + ": cold identity index");
    for (const sample of lake.samples) {
      const asset = prefix + `/assets/${sample.sha}`;
      const overview = await ready(asset + "/metadata");
      if (entry.records === 0)
        entry.identity_first_read_seconds =
          (performance.now() - started) / 1000;
      assert.equal(overview.stored_width, sample.stored_width);
      assert.equal(overview.stored_height, sample.stored_height);
      const observations = await engine.api(
        asset +
          `/records/${sample.record_id}/observations?observation_id=${sample.observation_id}&version=${encodeURIComponent(overview.version.token)}`,
      );
      const observation = observations.items[0];
      assert.equal(observation.post_id, String(sample.post_id));
      assert.deepEqual(
        observation.fields.find((f) => f.name === "tags").value.value,
        sample.tags,
      );
      const raw = await engine.api(
        asset +
          `/records/${sample.record_id}/observations/${sample.observation_id}/raw?version=${encodeURIComponent(overview.version.token)}`,
      );
      assert.equal(raw.status, "available");
      assert.equal(digest(raw.json), sample.raw_sha256);
      assert.equal(raw.schema_id, sample.schema_id);
      assert.equal(
        digest(Buffer.from(raw.schema.data, "hex")),
        sample.schema_id,
      );
      if (sample.preview) {
        const response = await engine.response(asset + "/media?edge=128");
        assert.equal(response.status, 200);
        assert.ok((await response.arrayBuffer()).byteLength > 100);
        entry.previews++;
      }
      entry.records++;
    }
    started = performance.now();
    await progress(lake.name + ": cold post-order index");
    const first = await ready(
      `/v1/projects/${pid}/assets?source_id=${source.id}&order=post_id_asc&limit=8`,
    );
    entry.browse_first_page_seconds = (performance.now() - started) / 1000;
    assert.equal(first.items.length, 8);
    if (first.next_cursor) {
      const next = await ready(
        `/v1/projects/${pid}/assets?source_id=${source.id}&order=post_id_asc&limit=8&cursor=${encodeURIComponent(first.next_cursor)}`,
      );
      assert.equal(next.items.length, 8);
      assert.ok(
        next.items.every(
          (n) => !first.items.some((f) => f.key.asset_id === n.key.asset_id),
        ),
      );
    }
    const selection = await engine.api(`/v1/projects/${pid}/selection`);
    const keys = [...new Set(lake.samples.map((s) => s.sha))].map((sha) => ({
      source_id: source.id,
      asset_id: sha,
    }));
    await engine.api(`/v1/projects/${pid}/selection`, "PATCH", {
      expected_revision: selection.revision,
      add: keys,
      remove: [],
      clear: true,
    });
    const collection = await engine.api(
      `/v1/projects/${pid}/collections`,
      "POST",
      { name: lake.name + " bounded examples" },
    );
    assert.equal(collection.count, keys.length);
    await progress(lake.name + ": bounded exact-tag queries");
    for (const sample of lake.samples) {
      for (const tag of sample.literal_tags) {
        started = performance.now();
        const q = await engine.api(
          `/v1/projects/${pid}/query-results`,
          "POST",
          {
            spec: {
              version: 3,
              source_ids: [source.id],
              conditions: [
                {
                  field: "post.id",
                  operator: "eq",
                  value: { type: "integer", value: String(sample.post_id) },
                },
                {
                  field: "tags",
                  operator: "has_tag",
                  value: { type: "text", value: tag },
                },
              ],
              observation_rule: "current_post",
              order: "asset_key_asc",
              input_scope: {
                project_id: pid,
                target: { kind: "workset", collection_id: collection.id },
              },
            },
          },
        );
        const done = await engine.wait(
          `/v1/projects/${pid}/query-results/${q.id}`,
          (r) => !["queued", "running"].includes(r.state),
          120000,
        );
        assert.equal(done.state, "ready", JSON.stringify(done));
        assert.equal(done.count, 1);
        const result = await engine.api(
          `/v1/projects/${pid}/query-results/${q.id}/assets?limit=8`,
        );
        assert.equal(result.page.items[0].key.asset_id, sample.sha);
        entry.literal_tags++;
        entry.query_seconds.push((performance.now() - started) / 1000);
      }
    }
    entry.index_bytes = await bytes(resolve(run, "state/browse-index"));
    await progress(lake.name + ": verified");
  }
  const metrics = await engine.api("/v1/resources");
  report.metrics = metrics;
  report.index_bytes = await bytes(resolve(run, "state/browse-index"));
  await engine.stop();
  await engine.start();
  await engine.api(`/v1/projects/${pid}/open`, "POST");
  for (const lake of lakes) {
    const start = performance.now();
    await ready(
      `/v1/projects/${pid}/sources/${lake.library_id}/assets/${lake.samples[0].sha}/metadata`,
    );
    report.lakes.find((e) => e.name === lake.name).restart_metadata_seconds =
      (performance.now() - start) / 1000;
  }
  assert.deepEqual(await stamps(), before);
  report.status = "passed";
  report.authoritative_stamps_unchanged = true;
  await writeFile(resolve(run, "report.json"), JSON.stringify(report, null, 2));
  await progress("complete");
} finally {
  await engine.stop();
}
