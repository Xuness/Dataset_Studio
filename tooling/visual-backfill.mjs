import { randomUUID } from "node:crypto";
import { mkdir, open, readFile, rename, unlink } from "node:fs/promises";
import { resolve, dirname, relative, isAbsolute } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const args = process.argv.slice(2).filter((a) => a !== "--");
const option = (name) =>
  args.find((a) => a.startsWith(`--${name}=`))?.slice(name.length + 3);
if (args.includes("--help")) {
  console.log(`pnpm visual:backfill [--sources=来源ID,...] [--chunk=5000] [--cycles=0] [--batch=128]
[--models=all|模型名称,...] [--micro=8] [--data-dir=.local/dev]
pnpm visual:backfill --state=状态JSON --resume
默认使用三个 Booru 来源和三个当前模型。cycles=0 运行到发现水位追上各湖当前发布版本。
按有界任务接续发现水位，在任务间按索引状态重建；Ctrl+C 请求暂停当前任务。
状态文件与日志属于命令行控制器；正式特征和任务始终由引擎管理。`);
  process.exit(0);
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const directory = resolve(option("data-dir") ?? resolve(root, ".local/dev"));
let interrupted = false;
process.on("SIGINT", () => {
  interrupted = true;
});
process.on("SIGTERM", () => {
  interrupted = true;
});
async function api(path, body, method = "POST") {
  const connection = JSON.parse(
    await readFile(resolve(directory, "engine.json"), "utf8"),
  );
  const endpoint = new URL(connection.endpoint);
  if (endpoint.protocol !== "http:" || endpoint.hostname !== "127.0.0.1")
    throw new Error("引擎须为本机 HTTP 服务");
  const response = await fetch(`${connection.endpoint}/v1/visual/${path}`, {
    method: body === undefined ? "GET" : method,
    headers: {
      Authorization: `Bearer ${connection.token}`,
      "Content-Type": "application/json",
    },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(120000),
  });
  const value = await response.json();
  if (!response.ok)
    throw Object.assign(new Error(`${value.code}: ${value.message}`), {
      code: value.code,
    });
  return value;
}
const statePath = resolve(
  option("state") ??
    resolve(root, ".local/visual-backfills", randomUUID() + ".json"),
);
const localRelative = relative(resolve(root, ".local"), statePath);
if (localRelative.startsWith("..") || isAbsolute(localRelative))
  throw new Error("控制器状态须位于 .local 内");
await mkdir(dirname(statePath), { recursive: true });
const lockPath = statePath + ".lock";
let lock;
try {
  lock = await open(lockPath, "wx");
} catch (error) {
  if (error.code !== "EEXIST") throw error;
  const owner = JSON.parse(await readFile(lockPath, "utf8"));
  let live = false;
  try {
    process.kill(owner.pid, 0);
    live = true;
  } catch (e) {
    if (e.code !== "ESRCH") throw e;
  }
  if (live) throw new Error(`该控制器进程仍在运行：PID ${owner.pid}`);
  await unlink(lockPath);
  lock = await open(lockPath, "wx");
}
await lock.writeFile(JSON.stringify({ pid: process.pid }));
await lock.close();
let state;
let explicitResumeAvailable = args.includes("--resume");
async function save() {
  state.updated_at = new Date().toISOString();
  const temporary = statePath + ".tmp";
  const file = await open(temporary, "w");
  try {
    await file.writeFile(JSON.stringify(state, null, 2));
    await file.sync();
  } finally {
    await file.close();
  }
  await rename(temporary, statePath);
}
function log(value) {
  console.log(JSON.stringify({ at: new Date().toISOString(), ...value }));
}
async function submit(kind, request, repair = false) {
  state.pending = { kind, id: randomUUID(), request, repair };
  state.pending.request.request_id = state.pending.id;
  await save();
}
async function settle() {
  const pending = state.pending;
  const collection = pending.kind === "index" ? "indexes" : "runs";
  let allowResume = explicitResumeAvailable;
  explicitResumeAvailable = false;
  let observationsFailed = 0;
  for (;;) {
    let job;
    try {
      try {
        job = await api(`${collection}/${pending.id}`);
      } catch (error) {
        if (error.code !== "NOT_FOUND") throw error;
        job = await api(collection, pending.request);
      }
      observationsFailed = 0;
    } catch (error) {
      if (
        error.code &&
        error.code !== "VISUAL_BUSY" &&
        error.code !== "VISUAL_UNAVAILABLE"
      )
        throw error;
      if (++observationsFailed > 20) throw error;
      await sleep(3000);
      continue;
    }
    // --resume authorizes the state observed at controller startup, not a
    // later manual pause, nor a pause of every subsequent task in the series.
    const explicitlyResumeNow =
      allowResume && ["paused", "failed"].includes(job.status);
    allowResume = false;
    state.progress = {
      id: job.id,
      status: job.status,
      completed_objects: job.completed_objects,
      progress: job.progress,
    };
    await save();
    if (interrupted && ["queued", "planning", "running"].includes(job.status)) {
      try {
        job = await api(`${collection}/${job.id}/actions`, { action: "pause" });
      } catch (error) {
        // Completion may win the race against the pause request.
        const observed = await api(`${collection}/${job.id}`);
        if (
          !["completed", "completed_with_errors", "cancelled"].includes(
            observed.status,
          )
        )
          throw error;
        job = observed;
      }
    }
    if (job.status === "paused" || job.status === "failed") {
      const restart =
        job.status === "paused" && job.error?.startsWith("引擎重启");
      if (!interrupted && (explicitlyResumeNow || restart)) {
        await api(`${collection}/${job.id}/actions`, { action: "resume" });
      } else {
        state.status = job.status;
        state.error = job.error;
        await save();
        return false;
      }
    } else if (job.status === "cancelled") {
      state.status = "cancelled";
      await save();
      return false;
    } else if (["completed", "completed_with_errors"].includes(job.status)) {
      while ((await api("runtime")).busy) await sleep(100);
      if (pending.kind === "extract") {
        state.cycles_completed++;
        const discovered = job.sources.reduce((n, s) => n + s.discovered, 0);
        state.discovered_bindings += discovered;
        state.failed_features += job.failed_features;
        state.discovery_exhausted ||= discovered === 0;
        if (pending.repair) state.discovery_exhausted = true;
      }
      state.completed_tasks.push({
        kind: pending.kind,
        id: job.id,
        status: job.status,
      });
      state.pending = null;
      await save();
      log({
        kind: pending.kind,
        id: job.id,
        status: job.status,
        completed_objects: job.completed_objects,
      });
      return true;
    }
    await sleep(5000);
  }
}
try {
  if (option("state")) {
    state = JSON.parse(await readFile(statePath, "utf8"));
    if (!args.includes("--resume")) throw new Error("恢复控制器需要 --resume");
    if (state.data_dir !== directory)
      throw new Error("应用目录与已保存控制器不一致");
  } else {
    const runtime = await api("runtime");
    const sources = (await api("sources")).items;
    const models = (await api("models")).recipes;
    const names = (option("models") ?? "all").split(",");
    const sourceIds = option("sources")?.split(",") ?? sources.map((s) => s.id);
    const recipes = models.filter(
      (m) => names.includes("all") || names.includes(m.model),
    );
    if (
      !sourceIds.length ||
      sourceIds.some((id) => !sources.some((s) => s.id === id)) ||
      !recipes.length
    )
      throw new Error("来源或模型选择无效");
    state = {
      id: randomUUID(),
      data_dir: directory,
      materials_root: runtime.config?.materials_root,
      source_ids: sourceIds,
      recipe_ids: recipes.map((r) => r.id),
      chunk: Number(option("chunk") ?? 5000),
      max_cycles: Number(option("cycles") ?? 0),
      batch: Number(option("batch") ?? 128),
      micro_batch_sizes: Object.fromEntries(
        recipes.map((r) => [r.model, Number(option("micro") ?? 8)]),
      ),
      status: "running",
      cycles_completed: 0,
      discovered_bindings: 0,
      failed_features: 0,
      discovery_exhausted: false,
      repaired_inventory: false,
      pending: null,
      completed_tasks: [],
      created_at: new Date().toISOString(),
    };
    if (
      !Number.isInteger(state.chunk) ||
      state.chunk < 1 ||
      state.chunk > 100000 ||
      !Number.isInteger(state.max_cycles) ||
      state.max_cycles < 0 ||
      !Number.isInteger(state.batch) ||
      state.batch < 1 ||
      state.batch > 512 ||
      Object.values(state.micro_batch_sizes).some(
        (n) => !Number.isInteger(n) || n < 1 || n > 64,
      )
    )
      throw new Error(
        "chunk 须为 1-100000，cycles 须为非负整数，batch 须为 1-512，micro 须为 1-64",
      );
    await save();
  }
  if (
    !["completed", "completed_with_errors", "batch_limit_reached"].includes(
      state.status,
    )
  ) {
    state.status = "running";
    await save();
    log({ status: "running", state: statePath });
    for (;;) {
      if (
        (await api("runtime")).config?.materials_root !== state.materials_root
      )
        throw new Error("视觉材料目录已改变，请检查控制器配置");
      if (state.pending && !(await settle())) break;
      if (interrupted) {
        state.status = "paused";
        await save();
        break;
      }
      const indexes = (await api("index-status")).items;
      const rebuild = indexes.find(
        (i) => state.recipe_ids.includes(i.recipe_id) && i.needs_rebuild,
      );
      if (rebuild) {
        await submit("index", {
          recipe_id: rebuild.recipe_id,
          partitions: null,
          max_iterations: 30,
          sample_rate: 128,
        });
        continue;
      }
      if (
        state.discovery_exhausted ||
        (state.max_cycles && state.cycles_completed >= state.max_cycles)
      ) {
        const coverage = (await api("coverage")).items.filter(
          (c) =>
            state.source_ids.includes(c.source_id) &&
            state.recipe_ids.includes(c.recipe_id),
        );
        if (state.discovery_exhausted && coverage.some((c) => c.pending > 0)) {
          if (state.repaired_inventory)
            throw new Error("完整补算后仍有待算条目，请检查来源与覆盖记录");
          state.repaired_inventory = true;
          await submit(
            "extract",
            {
              source_ids: state.source_ids,
              recipe_ids: state.recipe_ids,
              max_objects_per_source: null,
              batch_size: state.batch,
              micro_batch_sizes: state.micro_batch_sizes,
              incremental: false,
            },
            true,
          );
          continue;
        }
        state.coverage = coverage;
        state.snapshot = await api("snapshots", {
          name: `Booru backfill ${new Date().toISOString().slice(0, 10)}`,
          recipe_ids: state.recipe_ids,
        });
        state.status = !state.discovery_exhausted
          ? "batch_limit_reached"
          : coverage.some((c) => c.failed)
            ? "completed_with_errors"
            : "completed";
        await save();
        log({
          status: state.status,
          state: statePath,
          snapshot: state.snapshot.id,
        });
        break;
      }
      await submit("extract", {
        source_ids: state.source_ids,
        recipe_ids: state.recipe_ids,
        max_objects_per_source: state.chunk,
        batch_size: state.batch,
        micro_batch_sizes: state.micro_batch_sizes,
        incremental: true,
      });
    }
  } else {
    log({ status: state.status, state: statePath });
  }
} catch (error) {
  if (state) {
    state.status = "failed";
    state.error = String(error);
    await save();
  }
  throw error;
} finally {
  await unlink(lockPath);
}
