// Opt-in, bounded real-source check. It reads index metadata and never opens pack payloads.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, readFile, writeFile, open } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { EngineFixture } from "./engine-fixture.mjs";

const arg = (name) => process.argv[process.argv.indexOf(name) + 1];
for (const name of ["--index-root", "--media-root", "--asset-id", "--post-id"])
  if (!process.argv.includes(name)) throw new Error("Required: " + name);
const asset = arg("--asset-id");
const post = arg("--post-id");
assert.match(asset, /^[0-9a-f]{64}$/);
assert.match(post, /^[1-9][0-9]{0,18}$/);
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const reportDir = resolve(root, ".local", "scope-verification-" + Date.now());
await mkdir(reportDir, { recursive: true });
const engine = new EngineFixture(root, resolve(reportDir, "state"));
const binary = resolve(root, "target/debug/studio-engine.exe");
const json = (path, value) => writeFile(path, JSON.stringify(value, null, 2));
const checks = [];
let peakWorkingSet = 0;
let peakPrivate = 0;
async function sampleMemory() {
  const child = spawn(
    "pwsh.exe",
    [
      "-NoProfile",
      "-Command",
      "$p=Get-Process -Id " +
        engine.child.pid +
        "; [pscustomobject]@{working=$p.WorkingSet64;private=$p.PrivateMemorySize64}|ConvertTo-Json -Compress",
    ],
    { windowsHide: true, stdio: ["ignore", "pipe", "pipe"] },
  );
  let text = "";
  child.stdout.on("data", (chunk) => (text += chunk));
  const code = await new Promise((done) => child.once("exit", done));
  if (code === 0) {
    const sample = JSON.parse(text);
    peakWorkingSet = Math.max(peakWorkingSet, sample.working);
    peakPrivate = Math.max(peakPrivate, sample.private);
  }
}
process.on("exit", () => engine.child?.kill());
try {
  await engine.start();
  const project = await engine.api("/v1/projects", "POST", {
    name: "真实来源范围检查",
  });
  const base = "/v1/projects/" + project.id;
  const source = await engine.api(base + "/sources", "POST", {
    kind: "danbooru",
    name: "Danbooru",
    index_root: arg("--index-root"),
    media_root: arg("--media-root"),
  });
  const sourceFile = resolve(reportDir, "source.json");
  await json(sourceFile, {
    id: source.id,
    name: source.name,
    kind: "danbooru",
    index_root: arg("--index-root"),
    media_root: arg("--media-root"),
  });
  const identity = {
    field: "asset.id",
    operator: "eq",
    value: { type: "text", value: asset },
  };
  const specs = [
    {
      name: "存储身份与格式",
      conditions: [
        identity,
        {
          field: "stored.extension",
          operator: "eq",
          value: { type: "text", value: "webp" },
        },
      ],
      observation_rule: "current_post",
    },
    {
      name: "关联观察的来源尺寸",
      conditions: [
        identity,
        {
          field: "post.id",
          operator: "eq",
          value: { type: "integer", value: post },
        },
        {
          field: "source.width",
          operator: "gte",
          value: { type: "integer", value: "1000" },
        },
        {
          field: "source.height",
          operator: "gte",
          value: { type: "integer", value: "1000" },
        },
      ],
      observation_rule: "any_observation",
    },
  ];
  for (let index = 0; index < specs.length; index++) {
    const example = specs[index];
    const spec = {
      version: 1,
      source_ids: [source.id],
      conditions: example.conditions,
      observation_rule: example.observation_rule,
      order: "asset_key_asc",
    };
    const specFile = resolve(reportDir, "spec-" + index + ".json");
    const planFile = resolve(reportDir, "plan-" + index + ".json");
    await json(specFile, spec);
    const log = await open(
      resolve(reportDir, "explain-" + index + ".log"),
      "w",
    );
    const explain = spawn(
      binary,
      [
        "explain-query",
        "--source",
        sourceFile,
        "--spec",
        specFile,
        "--output",
        planFile,
      ],
      { cwd: root, windowsHide: true, stdio: ["ignore", log.fd, log.fd] },
    );
    await log.close();
    assert.equal(
      await new Promise((done) => explain.once("exit", done)),
      0,
      "Explain failed; inspect its log",
    );
    const plan = JSON.parse(await readFile(planFile, "utf8"));
    assert.ok(plan.storage_plan.some((line) => /PRIMARY KEY|INDEX/.test(line)));
    const started = Date.now();
    const definition = await engine.api(base + "/queries", "POST", {
      name: example.name,
      spec,
    });
    const result = await engine.api(
      base + "/queries/" + definition.id + "/results",
      "POST",
      { expected_revision: definition.revision },
    );
    const finished = await engine.wait(
      base + "/query-results/" + result.id,
      (r) => ["ready", "failed"].includes(r.state),
      60000,
    );
    assert.equal(finished.state, "ready", JSON.stringify(finished));
    assert.equal(finished.count, 1);
    const buildMs = Date.now() - started;
    const pageStarted = Date.now();
    const page = await engine.api(
      base + "/query-results/" + result.id + "/assets?limit=2",
    );
    const pageMs = Date.now() - pageStarted;
    assert.equal(page.page.items.length, 1);
    assert.equal(page.page.items[0].key.asset_id, asset);
    assert.equal(page.page.next_cursor, null);
    assert.equal((await engine.api(base + "/selection")).count, 0);
    await sampleMemory();
    checks.push({
      name: example.name,
      result_id: result.id,
      count: finished.count,
      processed: finished.processed,
      source_versions: finished.source_versions,
      build_ms: buildMs,
      page_ms: pageMs,
      plan: planFile,
    });
  }
  const report = {
    passed: true,
    checks,
    max_sampled_working_set_bytes: peakWorkingSet,
    max_sampled_private_bytes: peakPrivate,
    boundaries:
      "Two identity-bounded queries, warm host caches possible; no pack payload reads, no cold-cache or full-lake pressure qualification. Memory is sampled after requests, not a continuous peak monitor.",
  };
  await json(resolve(reportDir, "report.json"), report);
  console.log(
    JSON.stringify(
      { ...report, report: resolve(reportDir, "report.json") },
      null,
      2,
    ),
  );
} finally {
  await engine.stop();
}
