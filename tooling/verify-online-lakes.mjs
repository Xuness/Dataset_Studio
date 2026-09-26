import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { performance } from "node:perf_hooks";
import { EngineFixture } from "./engine-fixture.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const config = JSON.parse(await readFile(resolve(process.argv[2]), "utf8"));
const run = resolve(root, ".local/test-runs", "formal-online-" + Date.now());
await mkdir(run, { recursive: true });
const engine = new EngineFixture(root, resolve(run, "state"));
const results = [];
const timed = async (fn) => {
  const t = performance.now();
  const value = await fn();
  return { value, ms: Math.round((performance.now() - t) * 1000) / 1000 };
};
try {
  await engine.start();
  const project = await engine.api("/v1/projects", "POST", {
    name: "正式湖只读验收",
  });
  const base = "/v1/projects/" + project.id;
  for (const source of config) {
    const attached = await engine.api(base + "/sources", "POST", {
      kind: "auto",
      name: source.site,
      index_root: source.index,
      media_root: source.media,
    });
    assert.equal(attached.kind, source.site);
    const first = await timed(() =>
      engine.api(
        base +
          "/assets?source_id=" +
          attached.id +
          "&order=post_id_desc&limit=48",
      ),
    );
    assert.equal(first.value.preparing ?? null, null);
    assert.ok(first.value.items.length > 0);
    assert.ok(
      first.value.items.every(
        (v) =>
          v.summary?.status === "available" || v.summary?.status === "unlinked",
      ),
    );
    const warm = [];
    for (let n = 0; n < 10; n++)
      warm.push(
        (
          await timed(() =>
            engine.api(
              base +
                "/assets?source_id=" +
                attached.id +
                "&order=post_id_desc&limit=48",
            ),
          )
        ).ms,
      );
    const second = await timed(() =>
      engine.api(
        base +
          "/assets?source_id=" +
          attached.id +
          "&order=post_id_desc&limit=48&cursor=" +
          encodeURIComponent(first.value.next_cursor),
      ),
    );
    assert.equal(
      new Set(
        [...first.value.items, ...second.value.items].map(
          (v) => v.key.asset_id,
        ),
      ).size,
      first.value.items.length + second.value.items.length,
    );
    const item = first.value.items.find((v) => v.summary?.post_ids?.length);
    const path =
      base + "/sources/" + attached.id + "/assets/" + item.key.asset_id;
    const meta = await timed(() =>
      engine.api(
        path + "/metadata?version=" + encodeURIComponent(item.summary.version),
      ),
    );
    const record = meta.value.records[0];
    const token = meta.value.version.token;
    const observations = await engine.api(
      path +
        "/records/" +
        record.record_id +
        "/observations?version=" +
        encodeURIComponent(token),
    );
    assert.ok(observations.items.length);
    const raw = await timed(() =>
      engine.api(
        path +
          "/records/" +
          record.record_id +
          "/observations/" +
          observations.items[0].observation_id +
          "/raw?version=" +
          encodeURIComponent(token),
      ),
    );
    assert.ok(
      ["available", "too_large", "missing"].includes(raw.value.status),
      JSON.stringify(raw.value),
    );
    const query = {
      version: 3,
      source_ids: [attached.id],
      conditions: [
        {
          field: "tags",
          operator: "has_tag",
          value: { type: "text", value: "1girl" },
        },
      ],
      observation_rule: "current_post",
      order: "post_id_desc",
    };
    const view = await engine.api(base + "/query-views", "POST", {
      spec: query,
    });
    assert.equal(view.count, null);
    const filtered = await timed(() =>
      engine.api(base + "/query-results/" + view.id + "/assets?limit=48"),
    );
    const sorted = [...warm].sort((a, b) => a - b);
    const candidate = first.value.items.find(
      (v) =>
        ["jpg", "jpeg", "png", "webp"].includes(v.extension) &&
        Number(v.bytes) <= 8 * 1024 * 1024,
    );
    const media = [];
    if (candidate)
      for (let i = 0; i < 2; i++) {
        const preview = await timed(async () => {
          const response = await engine.response(
            base +
              "/sources/" +
              attached.id +
              "/assets/" +
              candidate.key.asset_id +
              "/media?edge=256",
          );
          if (!response.ok) throw new Error(await response.text());
          return {
            bytes: (await response.arrayBuffer()).byteLength,
            cache: response.headers.get("x-studio-cache"),
          };
        });
        assert.ok(preview.value.bytes > 0);
        media.push({ ms: preview.ms, ...preview.value });
      }
    results.push({
      site: source.site,
      id: attached.id,
      revision: attached.revision,
      count: attached.count,
      first_page_ms: first.ms,
      warm_p50_ms: sorted[5],
      warm_p95_ms: sorted[9],
      next_page_ms: second.ms,
      metadata_ms: meta.ms,
      raw_ms: raw.ms,
      raw_status: raw.value.status,
      tag_page_ms: filtered.ms,
      tag_items: filtered.value.page.items.length,
      scan: filtered.value.page.scan ?? null,
      preview: media,
    });
    await engine.api(base + "/query-results/" + view.id + "/release", "POST");
  }
  const result = {
    run,
    conditions:
      "48 rows, API over loopback, live metadata SSDs, first request plus 10 warm repeats; concurrent migration verification may affect latency",
    results,
  };
  await writeFile(resolve(run, "result.json"), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result));
} finally {
  await engine.stop();
}
