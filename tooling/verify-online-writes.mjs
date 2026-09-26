import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { performance } from "node:perf_hooks";
import { EngineFixture } from "./engine-fixture.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const config = JSON.parse(await readFile(resolve(process.argv[2]), "utf8"));
const run = resolve(
  root,
  ".local/test-runs",
  "formal-concurrency-" + Date.now(),
);
await mkdir(run, { recursive: true });
const engine = new EngineFixture(root, resolve(run, "state"));
const results = [];
try {
  await engine.start();
  const project = await engine.api("/v1/projects", "POST", {
    name: "在线读取并发验证",
  });
  const base = "/v1/projects/" + project.id;
  for (const lake of config) {
    const source = await engine.api(base + "/sources", "POST", {
      kind: "auto",
      name: lake.site,
      index_root: lake.index,
      media_root: lake.media,
    });
    const view = await engine.api(base + "/query-views", "POST", {
      spec: {
        version: 3,
        source_ids: [source.id],
        conditions: [],
        observation_rule: "current_post",
        order: "post_id_desc",
      },
    });
    const path = base + "/query-results/" + view.id + "/assets?limit=16";
    const expected = (await engine.api(path)).page.items.map((v) => v.key);
    const writer = spawn(
      process.env.PYTHON ?? "python",
      [resolve(root, "tooling/online-write-probe.py"), lake.index],
      { windowsHide: true, stdio: ["pipe", "pipe", "pipe"] },
    );
    let error = "";
    writer.stderr.on("data", (v) => {
      error += v;
    });
    const exited = once(writer, "exit");
    try {
      const ready = await Promise.race([
        once(writer.stdout, "data"),
        exited.then(() => {
          throw new Error(error);
        }),
      ]);
      assert.equal(String(ready[0]).trim(), "ready");
      const samples = [];
      for (let n = 0; n < 10; n++) {
        const start = performance.now();
        const page = await engine.api(path);
        samples.push(Math.round((performance.now() - start) * 1000) / 1000);
        assert.deepEqual(
          page.page.items.map((v) => v.key),
          expected,
        );
        assert.equal(writer.exitCode, null);
      }
      assert.ok(
        Math.max(...samples) < 1000,
        "published views waited for the uncommitted writer",
      );
      results.push({
        site: lake.site,
        requests: 10,
        writer_held: true,
        rows_per_request: 16,
        latency_ms: samples,
        source_data_changes: 0,
      });
    } finally {
      writer.stdin.end("\n");
      await exited;
    }
    await engine.api(base + "/query-results/" + view.id + "/release", "POST");
  }
  await writeFile(
    resolve(run, "result.json"),
    JSON.stringify({ run, results }, null, 2),
  );
  console.log(JSON.stringify({ run, results }));
} finally {
  await engine.stop();
}
