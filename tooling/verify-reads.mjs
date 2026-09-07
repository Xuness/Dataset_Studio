import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { createHash, randomUUID } from "node:crypto";
import { performance } from "node:perf_hooks";
import { resolve } from "node:path";
import { fileURLToPath, URLSearchParams } from "node:url";
import { EngineFixture } from "./engine-fixture.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const options = { assets: [], edge: 360, budget: 2 * 1024 * 1024 };
for (let i = 2; i < process.argv.length; i += 2) {
  const flag = process.argv[i],
    value = process.argv[i + 1];
  if (value === undefined) throw new Error("Every flag needs a value");
  switch (flag) {
    case "--index-root":
      options.index = resolve(value);
      break;
    case "--media-root":
      options.media = resolve(value);
      break;
    case "--asset":
      options.assets.push(value);
      break;
    case "--max-source-bytes":
      options.budget = Number(value);
      break;
    case "--edge":
      options.edge = Number(value);
      break;
    default:
      throw new Error("Unknown option: " + flag);
  }
}
if (
  !options.index ||
  !options.media ||
  options.assets.length < 1 ||
  options.assets.length > 8 ||
  new Set(options.assets).size !== options.assets.length ||
  options.assets.some((id) => !/^[0-9a-f]{64}$/.test(id))
)
  throw new Error(
    "Use --index-root, --media-root and 1-8 distinct --asset SHA-256 identities",
  );
if (
  !Number.isSafeInteger(options.budget) ||
  options.budget < 1 ||
  options.budget > 64 * 1024 * 1024 ||
  !Number.isSafeInteger(options.edge) ||
  options.edge < 96 ||
  options.edge > 1600
)
  throw new Error("Budget must be 1..64 MiB in bytes; edge must be 96..1600");
const runDir = resolve(root, ".local", "read-verification-" + Date.now());
await mkdir(runDir, { recursive: true });
const engine = new EngineFixture(root, resolve(runDir, "state")),
  checks = [],
  timings = {},
  snapshots = {};
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
process.on("exit", () => engine.child?.kill());
try {
  await engine.start();
  const project = await engine.api("/v1/projects", "POST", {
      name: "指定读取范围验证",
    }),
    base = "/v1/projects/" + project.id;
  const source = await engine.api(base + "/sources", "POST", {
    kind: "danbooru",
    name: "只读验证来源",
    index_root: options.index,
    media_root: options.media,
  });
  const assetPath = (id) => base + "/sources/" + source.id + "/assets/" + id;
  const assets = [];
  for (const id of options.assets) assets.push(await engine.api(assetPath(id)));
  const expected = assets.reduce((sum, a) => sum + BigInt(a.bytes), 0n);
  if (expected > BigInt(options.budget))
    throw new Error(
      "BUDGET_REJECTED: selected payload bytes " +
        expected +
        " exceed " +
        options.budget,
    );
  snapshots.before = await engine.api("/v1/resources");
  async function preview(a) {
    const query = new URLSearchParams({
      edge: String(options.edge),
      request_id: randomUUID(),
      max_source_bytes: a.bytes,
    });
    const response = await engine.response(
      assetPath(a.key.asset_id) + "/media?" + query,
    );
    assert.equal(response.ok, true, response.ok ? "" : await response.text());
    const bytes = Buffer.from(await response.arrayBuffer());
    return {
      asset_id: a.key.asset_id,
      cache: response.headers.get("x-studio-cache"),
      freshness: response.headers.get("x-studio-freshness"),
      bytes: bytes.length,
      sha256: digest(bytes),
    };
  }
  let start = performance.now();
  const cold = await Promise.all(assets.map(preview));
  timings.cold_ms = Math.round(performance.now() - start);
  snapshots.cold = await engine.api("/v1/resources");
  assert.ok(cold.every((v) => v.cache === "generated"));
  assert.equal(BigInt(snapshots.cold.previews.source_bytes), expected);
  start = performance.now();
  const warm = await Promise.all(assets.map(preview));
  timings.warm_ms = Math.round(performance.now() - start);
  snapshots.warm = await engine.api("/v1/resources");
  assert.ok(
    warm.every((v, i) => v.cache === "hit" && v.sha256 === cold[i].sha256),
  );
  assert.equal(
    snapshots.warm.previews.source_bytes,
    snapshots.cold.previews.source_bytes,
  );
  checks.push(
    "explicit source identities are preflighted against a total budget; each physical read is capped at its admitted length",
    "warm hashes match and add no source payload reads",
  );
  await engine.stop();
  await engine.start();
  await engine.api(base + "/open", "POST");
  start = performance.now();
  const restarted = await Promise.all(assets.map(preview));
  timings.restart_warm_ms = Math.round(performance.now() - start);
  snapshots.restart = await engine.api("/v1/resources");
  assert.ok(
    restarted.every((v, i) => v.cache === "hit" && v.sha256 === cold[i].sha256),
  );
  assert.equal(snapshots.restart.previews.source_bytes, "0");
  checks.push(
    "persistent cache survives engine restart without source payload I/O",
  );
  const report = {
    passed: true,
    checks,
    timings,
    snapshots,
    assets,
    cold,
    warm,
    restarted,
    source_id: source.id,
    source_revision: source.revision,
    budget_bytes: options.budget,
    expected_payload_bytes: expected.toString(),
    scope: {
      count: assets.length,
      edge: options.edge,
      application_cache_cold: true,
      os_device_cache_flushed: false,
    },
    runDir,
  };
  await writeFile(
    resolve(runDir, "report.json"),
    JSON.stringify(report, null, 2),
  );
  console.log(
    JSON.stringify(
      {
        passed: true,
        count: assets.length,
        expected_payload_bytes: expected.toString(),
        timings,
        report: resolve(runDir, "report.json"),
      },
      null,
      2,
    ),
  );
} catch (error) {
  await writeFile(
    resolve(runDir, "failure.json"),
    JSON.stringify(
      { error: String(error), checks, timings, snapshots },
      null,
      2,
    ),
  );
  throw error;
} finally {
  await engine.stop();
}
