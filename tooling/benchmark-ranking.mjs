import assert from "node:assert/strict";
import { copyFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { createHash } from "node:crypto";
import { performance } from "node:perf_hooks";
import { EngineFixture, sleep } from "./engine-fixture.mjs";

const { values } = parseArgs({
  options: {
    "index-root": { type: "string" },
    "media-root": { type: "string" },
    parameters: { type: "string" },
    "memory-gib": { type: "string", default: "16" },
    "timeout-seconds": { type: "string", default: "1800" },
    label: { type: "string", default: "full-lake" },
    "reuse-run": { type: "string" },
    "live-connection": { type: "string" },
    "project-id": { type: "string" },
    "source-id": { type: "string" },
  },
});
assert.ok(
  !(values["reuse-run"] && values["live-connection"]),
  "Choose one existing runtime",
);
for (const field of values["live-connection"]
  ? ["parameters", "project-id", "source-id"]
  : values["reuse-run"]
    ? ["parameters"]
    : ["index-root", "media-root", "parameters"])
  assert.ok(values[field], `--${field} is required`);
const memoryGiB = Number(values["memory-gib"]);
const timeoutSeconds = Number(values["timeout-seconds"]);
assert.ok(Number.isInteger(memoryGiB) && memoryGiB >= 1 && memoryGiB <= 64);
assert.ok(Number.isInteger(timeoutSeconds) && timeoutSeconds >= 1);
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local/test-runs",
  "ranking-benchmark-" + Date.now(),
);
await mkdir(run, { recursive: true });
const executable = resolve(run, "studio-engine.exe");
await copyFile(resolve(root, "target/release/studio-engine.exe"), executable);
const engineSha256 = createHash("sha256")
  .update(await readFile(executable))
  .digest("hex");
const request = JSON.parse(await readFile(resolve(values.parameters), "utf8"));
const previous = values["reuse-run"]
  ? JSON.parse(
      await readFile(resolve(values["reuse-run"], "report.json"), "utf8"),
    )
  : null;
if (previous)
  assert.equal(
    previous.succeeded,
    true,
    "Reuse requires a completed benchmark",
  );
const dataDirectory = values["live-connection"]
  ? dirname(resolve(values["live-connection"]))
  : (previous?.dataDirectory ??
    (previous ? resolve(values["reuse-run"], "state") : resolve(run, "state")));
const engine = new EngineFixture(root, dataDirectory, run, {
  executable,
  env: {
    RUST_LOG: "studio_engine=info,studio_sources=info,studio_storage=info",
  },
});
const report = {
  label: values.label,
  startedAt: new Date().toISOString(),
  scope:
    "Full source at a retained version; isolated project; complete ranking, validation, publication and first page",
  osCache: "Not cleared; no ranking input or member cache is pre-seeded",
  engineSha256,
  memoryGiB,
  dataDirectory,
  reuseRun: values["reuse-run"] ? resolve(values["reuse-run"]) : null,
  request,
  samples: [],
};
const save = () =>
  writeFile(resolve(run, "report.json"), JSON.stringify(report, null, 2));
process.on("exit", () => engine.child?.kill());
let base, job;
try {
  if (values["live-connection"]) {
    engine.connection = JSON.parse(
      await readFile(resolve(values["live-connection"]), "utf8"),
    );
    assert.ok(
      ["127.0.0.1", "localhost", "[::1]"].includes(
        new URL(engine.connection.endpoint).hostname,
      ),
    );
    const health = await engine.api("/v1/health");
    assert.equal(health.instance_id, engine.connection.instance_id);
    const build = JSON.parse(
      await readFile(resolve(dataDirectory, "development-build.json"), "utf8"),
    );
    assert.equal(build.instance_id, health.instance_id);
    assert.equal(
      build.fingerprint,
      engineSha256,
      "Live engine must match the measured release executable",
    );
    report.memoryGiB =
      Number(
        (await engine.api("/v1/resources")).query_limits.query_memory_bytes,
      ) /
      2 ** 30;
    report.scope =
      "Existing project and retained full source; complete ranking, validation, publication and first page";
    report.osCache =
      "Not cleared; this project's verified frozen inputs may be reused";
  } else {
    await engine.start();
    await engine.api("/v1/resources/query", "PUT", { memory_gib: memoryGiB });
  }
  const project = values["live-connection"]
    ? await engine.api("/v1/projects/" + values["project-id"] + "/open", "POST")
    : (previous?.project ??
      (await engine.api("/v1/projects", "POST", {
        name: "Ranking benchmark " + values.label,
      })));
  base = "/v1/projects/" + project.id;
  if (previous) await engine.api(base + "/open", "POST");
  const source =
    previous || values["live-connection"]
      ? (await engine.api(base + "/sources")).items.find(
          (s) => s.id === (previous?.source.id ?? values["source-id"]),
        )
      : await engine.api(base + "/sources", "POST", {
          name: "Danbooru benchmark",
          kind: "danbooru",
          index_root: resolve(values["index-root"]),
          media_root: resolve(values["media-root"]),
        });
  assert.ok(source, "The requested source must belong to the project");
  if (previous) {
    assert.equal(
      source?.revision,
      previous.source.revision,
      "Reuse benchmark requires the same source version",
    );
    report.osCache =
      "Not cleared; verified frozen metadata from the previous run is available for reuse";
  }
  report.project = project;
  report.source = source;
  const start = performance.now();
  job = await engine.api(base + "/tools/jobs", "POST", {
    idempotency_key: crypto.randomUUID(),
    scope: {
      project_id: project.id,
      target: {
        kind: "source",
        source_id: source.id,
        revision: source.revision,
      },
    },
    run: request,
  });
  report.jobId = job.id;
  let lastSample = -Infinity;
  let lastStage = "";
  console.log(JSON.stringify({ run, jobId: job.id, engineSha256 }));
  await save();
  while (true) {
    const jobs = await engine.api(base + "/jobs");
    job = jobs.items.find((v) => v.id === job.id);
    const seconds = (performance.now() - start) / 1000;
    const stage = `${job.stage?.name}/${job.stage?.rating ?? ""}`;
    if (
      seconds - lastSample >= 30 ||
      stage !== lastStage ||
      ["succeeded", "failed", "cancelled"].includes(job.status)
    ) {
      const sample = { seconds, status: job.status, stage: job.stage };
      report.samples.push(sample);
      report.job = job;
      report.elapsedSeconds = seconds;
      await save();
      console.log(
        JSON.stringify({
          seconds: +seconds.toFixed(2),
          status: job.status,
          stage: job.stage?.name,
          completed: job.stage?.completed,
          total: job.stage?.total,
        }),
      );
      lastSample = seconds;
      lastStage = stage;
    }
    if (["succeeded", "failed", "cancelled"].includes(job.status)) break;
    if (seconds > timeoutSeconds) {
      await engine.api(base + "/jobs/" + job.id + "/cancel", "POST");
      throw new Error("Benchmark exceeded its time budget");
    }
    await sleep(1000);
  }
  assert.equal(job.status, "succeeded", job.error ?? JSON.stringify(job));
  report.reusedSnapshot =
    job.stage?.telemetry?.phases.some((p) => p.name === "snapshot_reuse") ??
    false;
  if (previous)
    assert.equal(
      report.reusedSnapshot,
      true,
      "Expected verified snapshot reuse",
    );
  const artifact = await engine.api(base + "/jobs/" + job.id + "/ranking");
  const summary = await engine.api(
    base + "/artifacts/" + artifact.id + "/ranking",
  );
  const firstPage = await engine.api(
    base + "/artifacts/" + artifact.id + "/ranking/rows",
    "POST",
    {
      filter: {
        order: request.parameters.v2 ? "fused" : "main",
        eligibility: "eligible",
      },
      limit: 48,
    },
  );
  assert.equal(artifact.count, job.total);
  assert.equal(summary.input_count, job.total);
  assert.ok(firstPage.items.length > 0);
  report.elapsedSeconds = (performance.now() - start) / 1000;
  report.artifact = artifact;
  report.summary = summary;
  report.firstPage = firstPage;
  report.succeeded = true;
  report.within300Seconds = report.elapsedSeconds <= 300;
  await save();
  console.log(
    JSON.stringify({
      report: resolve(run, "report.json"),
      seconds: report.elapsedSeconds,
      count: job.total,
      within300Seconds: report.within300Seconds,
    }),
  );
} catch (error) {
  report.succeeded = false;
  report.error = String(error.stack ?? error);
  await save();
  console.error(report.error);
  process.exitCode = 1;
} finally {
  await engine.stop();
}
