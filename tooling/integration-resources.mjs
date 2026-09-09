import assert from "node:assert/strict";
import { mkdir, writeFile, readFile, rename } from "node:fs/promises";
import { createHash, randomUUID } from "node:crypto";
import { deflateSync } from "node:zlib";
import { DatabaseSync } from "node:sqlite";
import { resolve } from "node:path";
import { fileURLToPath, URLSearchParams } from "node:url";
import { performance } from "node:perf_hooks";
import { EngineFixture, within } from "./engine-fixture.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const runDir = resolve(
  root,
  ".local",
  "test-runs",
  "integration-resources-" + Date.now(),
);
await mkdir(runDir, { recursive: true });
const engine = new EngineFixture(root, resolve(runDir, "state"));
const checks = [],
  timings = {},
  samples = {};
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
const json = (path, value) => writeFile(path, JSON.stringify(value, null, 2));
function crc32(bytes) {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let i = 0; i < 8; i++) crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
  }
  return (crc ^ 0xffffffff) >>> 0;
}
function png(seed, width = 160, height = 200) {
  const chunk = (name, data) => {
    const kind = Buffer.from(name),
      size = Buffer.alloc(4),
      crc = Buffer.alloc(4);
    size.writeUInt32BE(data.length);
    crc.writeUInt32BE(crc32(Buffer.concat([kind, data])));
    return Buffer.concat([size, kind, data, crc]);
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header[8] = 8;
  header[9] = 2;
  const raw = Buffer.alloc((width * 3 + 1) * height);
  let state = seed + 1;
  for (let y = 0; y < height; y++)
    for (let x = 0; x < width * 3; x++) {
      state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
      raw[y * (width * 3 + 1) + 1 + x] = state >>> 24;
    }
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}
async function fixture(count) {
  const id = randomUUID(),
    index = resolve(runDir, "fixture-index"),
    media = resolve(runDir, "fixture-media");
  const generation = resolve(index, "indexes/gen-1");
  await mkdir(generation, { recursive: true });
  await mkdir(resolve(media, "packs"), { recursive: true });
  await json(resolve(index, "CURRENT.json"), {
    library_id: id,
    index_version: 1,
    generation: "gen-1",
  });
  await json(resolve(media, "library.json"), {
    library_id: id,
    format_version: 1,
    image_format: "uncompressed-pax-tar",
  });
  const dbPath = resolve(generation, "catalog.sqlite"),
    db = new DatabaseSync(dbPath);
  db.exec(
    "CREATE TABLE state(key TEXT PRIMARY KEY,value INTEGER);INSERT INTO state VALUES('seq',1);CREATE TABLE objects(sha256 TEXT PRIMARY KEY,pack_path TEXT,offset INTEGER,length INTEGER,stored_ext TEXT) WITHOUT ROWID;BEGIN",
  );
  const insert = db.prepare("INSERT INTO objects VALUES(?,?,?,?, 'png')"),
    objects = [];
  for (let pack = 0; pack < 2; pack++) {
    let offset = 0;
    const buffers = [],
      packPath = `packs/${pack}.tar`;
    for (let i = pack; i < count; i += 2) {
      const bytes = png(i),
        id = digest(bytes);
      insert.run(id, packPath, offset, bytes.length);
      objects.push({ id, bytes: bytes.length, pack: packPath, offset });
      buffers.push(bytes);
      offset += bytes.length;
    }
    await writeFile(resolve(media, packPath), Buffer.concat(buffers));
  }
  db.exec("COMMIT");
  db.close();
  return {
    id,
    index,
    media,
    dbPath,
    objects: objects.sort((a, b) => a.id.localeCompare(b.id)),
  };
}
const base = (pid) => "/v1/projects/" + pid;
const url = (
  pid,
  source,
  asset,
  edge = 360,
  id = randomUUID(),
  priority = "interactive",
) =>
  base(pid) +
  "/sources/" +
  source +
  "/assets/" +
  asset +
  "/media?" +
  new URLSearchParams({ edge: String(edge), request_id: id, priority });
async function preview(path) {
  const response = await engine.response(path);
  assert.equal(response.ok, true, response.ok ? "" : await response.text());
  const bytes = Buffer.from(await response.arrayBuffer());
  assert.ok(bytes.length > 100);
  return {
    bytes,
    cache: response.headers.get("x-studio-cache"),
    freshness: response.headers.get("x-studio-freshness"),
    verifiedMs: response.headers.get("x-studio-verified-ms"),
  };
}
async function measure(name, run) {
  const started = performance.now();
  const result = await run();
  timings[name] = Math.round(performance.now() - started);
  return result;
}
process.on("exit", () => engine.child?.kill());
try {
  const lake = await fixture(96);
  await engine.start();
  const initialQueryLimits = (await engine.api("/v1/resources")).query_limits;
  assert.equal(initialQueryLimits.query_memory_bytes, String(12 * 2 ** 30));
  const configuredQuery = await engine.api("/v1/resources/query", "PUT", {
    memory_gib: 3,
  });
  assert.equal(configuredQuery.query_memory_bytes, String(3 * 2 ** 30));
  await engine.expectError(
    "/v1/resources/query",
    "PUT",
    { memory_gib: 0 },
    "INVALID_INPUT",
  );
  await engine.expectError(
    "/v1/resources/query",
    "PUT",
    { memory_gib: 65 },
    "INVALID_INPUT",
  );
  const adjustedResources = await engine.api("/v1/resources");
  assert.equal(
    adjustedResources.resources.find((r) => r.class === "native_query")
      .byte_budget,
    String(3 * 2 ** 30 + 256 * 2 ** 20),
  );
  checks.push(
    "query memory configuration validates bounds and updates the admission budget independently of cache quota",
  );
  const project = await engine.api("/v1/projects", "POST", {
    name: "读取与缓存夹具",
  });
  const isolated = await engine.api("/v1/projects", "POST", {
    name: "访问隔离夹具",
  });
  const attach = {
    name: "合成图片范围",
    kind: "danbooru",
    index_root: lake.index,
    media_root: lake.media,
  };
  await engine.api(base(project.id) + "/sources", "POST", attach);
  const input = lake.objects.slice(0, 12);
  const cold = await measure("cold_12_ms", () =>
    Promise.all(input.map((a) => preview(url(project.id, lake.id, a.id)))),
  );
  assert.ok(
    cold.every((r) => r.cache === "generated" && r.freshness === "verified"),
  );
  samples.cold = await engine.api("/v1/resources");
  assert.equal(
    Number(samples.cold.previews.source_bytes),
    input.reduce((s, a) => s + a.bytes, 0),
  );
  assert.ok(
    samples.cold.previews.max_batch > 1 &&
      samples.cold.previews.max_batch <= 16,
  );
  assert.ok(samples.cold.previews.pack_opens < 12);
  const warm = await measure("warm_12_ms", () =>
    Promise.all(input.map((a) => preview(url(project.id, lake.id, a.id)))),
  );
  assert.ok(
    warm.every(
      (r, i) => r.cache === "hit" && digest(r.bytes) === digest(cold[i].bytes),
    ),
  );
  samples.warm = await engine.api("/v1/resources");
  assert.equal(
    samples.warm.previews.source_bytes,
    samples.cold.previews.source_bytes,
  );
  checks.push(
    "cold requests use physical pack batches; warm hits have identical bytes and no additional source payload reads",
  );
  const sharedAsset = lake.objects[12];
  const beforeShared = samples.warm.previews;
  await engine.expectError(
    url(project.id, lake.id, sharedAsset.id) + "&max_source_bytes=1",
    "GET",
    undefined,
    "READ_BUDGET_EXCEEDED",
  );
  assert.equal(
    (await engine.api("/v1/resources")).previews.source_bytes,
    beforeShared.source_bytes,
  );
  assert.equal(
    (
      await preview(
        url(project.id, lake.id, input[0].id) + "&max_source_bytes=0",
      )
    ).cache,
    "hit",
  );
  const shared = await Promise.all(
    Array.from({ length: 12 }, () =>
      preview(url(project.id, lake.id, sharedAsset.id)),
    ),
  );
  const afterShared = (await engine.api("/v1/resources")).previews;
  assert.equal(afterShared.generated - beforeShared.generated, 1);
  assert.equal(
    Number(afterShared.source_bytes) - Number(beforeShared.source_bytes),
    sharedAsset.bytes,
  );
  assert.ok(afterShared.shared > beforeShared.shared);
  assert.equal(new Set(shared.map((r) => digest(r.bytes))).size, 1);
  checks.push(
    "twelve simultaneous HTTP consumers share exactly one source read and one generation",
  );

  await engine.expectError(
    url(isolated.id, lake.id, input[0].id),
    "GET",
    undefined,
    "NOT_FOUND",
  );
  await engine.api(base(isolated.id) + "/sources", "POST", attach);
  assert.equal(
    (await preview(url(isolated.id, lake.id, input[0].id))).cache,
    "hit",
  );
  const early = randomUUID();
  await engine.api(
    base(project.id) + "/read-requests/" + early + "/cancel",
    "POST",
  );
  const beforeEarly = (await engine.api("/v1/resources")).previews.source_bytes;
  await engine.expectError(
    url(project.id, lake.id, lake.objects[13].id, 360, early),
    "GET",
    undefined,
    "CANCELLED",
  );
  assert.equal(
    (await engine.api("/v1/resources")).previews.source_bytes,
    beforeEarly,
  );
  const cancelledBrowse = await fetch(
    engine.connection.endpoint + base(project.id) + "/assets?limit=4",
    {
      headers: {
        Authorization: "Bearer " + engine.connection.token,
        "x-studio-read-id": early,
      },
    },
  );
  assert.equal((await cancelledBrowse.json()).code, "CANCELLED");
  checks.push(
    "cache hits require project membership; cancellation arriving before media or browse requests prevents their execution",
  );

  const cacheDirectory = (await engine.api("/v1/resources")).cache.directory;
  within(runDir, cacheDirectory);
  await engine.stop();
  await engine.start();
  assert.equal(
    (await engine.api("/v1/resources")).query_limits.query_memory_bytes,
    String(3 * 2 ** 30),
  );
  checks.push("query memory configuration survives engine restart");
  await engine.api(base(project.id) + "/open", "POST");
  const restarted = await measure("restart_warm_ms", () =>
    preview(url(project.id, lake.id, input[0].id)),
  );
  assert.equal(restarted.cache, "hit");
  assert.equal((await engine.api("/v1/resources")).previews.source_bytes, "0");
  assert.equal(digest(restarted.bytes), digest(cold[0].bytes));
  const offline = within(runDir, resolve(runDir, "fixture-media-offline"));
  within(runDir, lake.media);
  await rename(lake.media, offline);
  try {
    const retained = await preview(url(project.id, lake.id, input[0].id));
    assert.equal(retained.freshness, "offline_cached");
    assert.equal(retained.verifiedMs, restarted.verifiedMs);
    await engine.expectError(
      url(project.id, lake.id, lake.objects[13].id),
      "GET",
      undefined,
      "SOURCE_UNAVAILABLE",
    );
  } finally {
    await rename(offline, lake.media);
  }
  checks.push(
    "engine restart uses persistent cache without source payload I/O; offline hits retain prior verification time and missing entries fail clearly",
  );

  assert.equal(
    (await preview(url(project.id, lake.id, input[0].id, 720))).cache,
    "generated",
  );
  const db = new DatabaseSync(lake.dbPath);
  db.exec("UPDATE state SET value=2 WHERE key='seq'");
  db.close();
  assert.equal(
    (await preview(url(project.id, lake.id, input[0].id))).cache,
    "hit",
  );
  const relink = within(runDir, resolve(runDir, "fixture-media-relinked"));
  await rename(lake.media, relink);
  await engine.api(
    base(project.id) + "/sources/" + lake.id + "/relink",
    "POST",
    { index_root: lake.index, media_root: relink },
  );
  lake.media = relink;
  assert.equal(
    (await preview(url(project.id, lake.id, input[0].id))).cache,
    "hit",
  );
  await json(resolve(lake.media, "library.json"), {
    library_id: randomUUID(),
    format_version: 1,
    image_format: "uncompressed-pax-tar",
  });
  await engine.expectError(
    url(project.id, lake.id, input[0].id),
    "GET",
    undefined,
    "SOURCE_ID_MISMATCH",
  );
  await json(resolve(lake.media, "library.json"), {
    library_id: lake.id,
    format_version: 1,
    image_format: "uncompressed-pax-tar",
  });
  checks.push(
    "render size invalidates output; append and verified relink preserve content-addressed hits; source identity mismatch cannot use cache",
  );

  const key = digest(
    Buffer.from(
      JSON.stringify([
        "preview-key-v1",
        "danbooru",
        lake.id,
        input[0].id,
        "sha256:" + input[0].id,
        360,
        "fit-no-crop-v1",
        "image-0.25-v1",
        "jpeg-q86-v1",
      ]),
    ),
  );
  const corrupted = within(
    runDir,
    resolve(cacheDirectory, "objects", key + ".jpg"),
  );
  await writeFile(corrupted, "corrupted preview");
  const repaired = await preview(url(project.id, lake.id, input[0].id));
  assert.equal(repaired.cache, "generated");
  assert.equal(digest(repaired.bytes), digest(cold[0].bytes));
  assert.equal((await engine.api("/v1/resources")).cache.corrupt, 1);
  await measure("bounded_96_ms", async () => {
    for (let i = 0; i < lake.objects.length; i += 12)
      await Promise.all(
        lake.objects
          .slice(i, i + 12)
          .map((a) => preview(url(project.id, lake.id, a.id))),
      );
  });
  samples.scale96 = await engine.api("/v1/resources");
  assert.ok(samples.scale96.previews.max_batch <= 16);
  assert.ok(
    samples.scale96.resources.every(
      (r) => BigInt(r.peak_reserved_bytes) <= BigInt(r.byte_budget),
    ),
  );
  checks.push(
    "corrupt material is regenerated and 96-image bounded waves stay within advertised resource reservations",
  );

  const keyInput = { source_id: lake.id, asset_id: input[0].id };
  const selection = await engine.api(base(project.id) + "/selection", "PATCH", {
    expected_revision: 0,
    add: [keyInput],
    remove: [],
    clear: false,
  });
  const job = await engine.api(base(project.id) + "/tools/jobs", "POST", {
    idempotency_key: randomUUID(),
    scope: {
      project_id: project.id,
      target: { kind: "selection", revision: selection.revision },
    },
    run: {
      operator_id: "core.manifest",
      operator_version: 1,
      parameters_version: 1,
      parameters: {},
    },
  });
  await engine.wait(base(project.id) + "/jobs", (r) =>
    r.items.some((j) => j.id === job.id && j.status === "succeeded"),
  );
  const artifact = await engine.api(base(project.id) + "/artifacts/" + job.id);
  const authoritative = await Promise.all(
    artifact.files.map(async (f) => ({
      path: within(runDir, resolve(project.directory, f.path)),
      hash: digest(await readFile(resolve(project.directory, f.path))),
    })),
  );
  await engine.api("/v1/resources/cache", "PUT", { quota_mib: 0 });
  await engine.wait("/v1/resources", (r) => r.cache.entries === 0);
  assert.equal(
    (await preview(url(project.id, lake.id, input[0].id))).cache,
    "generated",
  );
  assert.equal((await engine.api("/v1/resources")).cache.entries, 0);
  await engine.stop();
  await engine.start();
  await engine.api(base(project.id) + "/open", "POST");
  assert.equal((await engine.api("/v1/resources")).cache.quota_bytes, "0");
  await engine.api("/v1/resources/cache", "PUT", { quota_mib: 2 });
  await preview(url(project.id, lake.id, input[0].id));
  await engine.api("/v1/resources/cache/clear", "POST");
  assert.equal((await engine.api("/v1/resources")).cache.entries, 0);
  assert.deepEqual(
    await engine.api(base(project.id) + "/selection"),
    selection,
  );
  for (const file of authoritative)
    assert.equal(digest(await readFile(file.path)), file.hash);
  checks.push(
    "zero quota disables retention and survives restart; clearing cache preserves selection and every authoritative artifact byte",
  );

  await preview(url(project.id, lake.id, input[0].id));
  await engine.stop();
  await writeFile(
    within(runDir, resolve(cacheDirectory, "cache.sqlite")),
    "damaged index fixture",
  );
  await engine.start();
  await engine.api(base(project.id) + "/open", "POST");
  assert.equal((await engine.api("/v1/resources")).cache.index_rebuilt, true);
  assert.equal(
    (await preview(url(project.id, lake.id, input[0].id))).cache,
    "generated",
  );
  for (const file of authoritative)
    assert.equal(digest(await readFile(file.path)), file.hash);
  checks.push(
    "damaged cache index is isolated and rebuilt independently of project authority",
  );
  samples.final = await engine.api("/v1/resources");
  await json(resolve(runDir, "report.json"), {
    passed: true,
    checks,
    timings,
    samples,
    scope: {
      kind: "synthetic_png_indexed_pack_fixture",
      objects: 96,
      media_payload_bytes: lake.objects.reduce((s, a) => s + a.bytes, 0),
      physical_disk_cold: false,
    },
    projectId: project.id,
    runDir,
  });
  console.log(
    JSON.stringify(
      { passed: true, checks, timings, report: resolve(runDir, "report.json") },
      null,
      2,
    ),
  );
} finally {
  await engine.stop();
}
