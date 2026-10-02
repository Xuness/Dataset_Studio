// Explicit live acceptance against an existing Studio engine. Only the named
// Pixiv lake/jobs are written; the engine and other source jobs remain owned by Studio.
import assert from "node:assert/strict";
import { mkdir, readFile, writeFile, rename } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { clientFixture } from "./client-fixture.mjs";
import { sleep } from "./engine-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
function option(name) {
  const i = process.argv.indexOf(name);
  return i < 0 ? undefined : process.argv[i + 1];
}
const output = option("--output"),
  author = option("--author"),
  mediaRoot = option("--media-root"),
  indexRoot = option("--index-root"),
  connectionPath = option("--connection"),
  projectId = option("--project");
assert.ok(
  output && mediaRoot && indexRoot && connectionPath && projectId,
  "Supply output, roots, connection file and project",
);
assert.match(author ?? "", /^[1-9][0-9]{0,19}$/);
const run = resolve(output);
const includeUnknown = process.argv.includes("--include-unknown");
const firstStage = includeUnknown ? "full_public" : "initial";
const secondStage = includeUnknown ? "incremental_full_public" : "incremental";
await mkdir(run, { recursive: true });
const progressFile = resolve(run, "progress.json");
const report = await readFile(progressFile, "utf8")
  .then(JSON.parse)
  .catch(() => ({
    started_at: new Date().toISOString(),
    author_id: author,
    media_root: resolve(mediaRoot),
    index_root: resolve(indexRoot),
    project_id: projectId,
    account_id: crypto.randomUUID(),
    keys: Object.fromEntries(
      ["lake", "account", "initial", "incremental"].map((k) => [
        k,
        crypto.randomUUID(),
      ]),
    ),
    checks: [],
    ok: false,
  }));
assert.equal(report.author_id, author);
assert.equal(report.media_root, resolve(mediaRoot));
assert.equal(report.index_root, resolve(indexRoot));
assert.equal(report.project_id, projectId);
for (const key of [firstStage, secondStage])
  report.keys[key] ??= crypto.randomUUID();
async function save() {
  const temporary = progressFile + ".tmp";
  await writeFile(temporary, JSON.stringify(report, null, 2));
  await rename(temporary, progressFile);
}
await save();
const { StudioClient } = await clientFixture(root, resolve(run, "client"));
let currentConnection, client;
async function current() {
  const connection = JSON.parse(
    await readFile(resolve(connectionPath), "utf8"),
  );
  assert.ok(
    /^http:\/\/127\.0\.0\.1:\d+$/.test(connection.endpoint),
    "Use the owned loopback engine",
  );
  if (connection.instance_id !== currentConnection?.instance_id) {
    currentConnection = connection;
    client = new StudioClient(connection);
  }
  return client;
}
async function call(run) {
  return run(await current());
}
async function action(id, value) {
  for (let i = 0; ; i++) {
    const job = await call((c) => c.sourceCollections.job(id));
    try {
      return await call((c) =>
        c.sourceCollections.action(id, {
          action: value,
          expected_revision: job.revision,
          request_key: crypto.randomUUID(),
        }),
      );
    } catch (e) {
      if (e.code !== "REVISION_CONFLICT" || i === 4) throw e;
    }
  }
}
const terminal = new Set([
  "completed",
  "completed_with_gaps",
  "cancelled",
  "waiting_credentials",
  "needs_review",
  "waiting_budget",
  "paused",
]);
async function settle(id, stage, exercisePause = false) {
  const until = Date.now() + 3 * 3600_000;
  let wasPaused = report.pause_resume_verified ?? false,
    lastSaved = 0,
    lastGood = Date.now();
  while (Date.now() < until) {
    let job;
    try {
      job = await call((c) => c.sourceCollections.job(id));
      lastGood = Date.now();
    } catch (e) {
      if (Date.now() - lastGood > 120_000) throw e;
      await sleep(3000);
      continue;
    }
    if (Date.now() - lastSaved > 15_000 || terminal.has(job.state)) {
      report[stage] = job;
      await save();
      lastSaved = Date.now();
    }
    if (
      exercisePause &&
      !wasPaused &&
      job.progress.works.details >= 4 &&
      !terminal.has(job.state)
    ) {
      await action(id, "pause");
      for (let n = 0; n < 120; n++) {
        const paused = await call((c) => c.sourceCollections.job(id));
        if (paused.state === "paused" && !paused.execution_active) {
          report.pause_checkpoint = paused.progress;
          break;
        }
        await sleep(1000);
      }
      assert.ok(report.pause_checkpoint, "Paused execution must actually exit");
      await action(id, "resume");
      wasPaused = true;
      report.pause_resume_verified = true;
      await save();
    } else if (terminal.has(job.state) && !job.execution_active) return job;
    await sleep(2500);
  }
  await action(id, "pause");
  throw new Error(
    "Live acceptance reached its bounded duration; job paused with progress preserved",
  );
}
try {
  delete report.error;
  assert.ok(
    (await call((c) => c.sourceCollections.capabilities())).periodic_snapshots,
  );
  if (!report.previous_lakes)
    report.previous_lakes = (
      await call((c) => c.sourceCollections.workspaceLakes({ limit: 200 }))
    ).items;
  if (!report.lake) {
    report.lake = await call((c) =>
      c.sourceCollections.createLake({
        request_key: report.keys.lake,
        site: "pixiv",
        media_root: report.media_root,
        index_root: report.index_root,
      }),
    );
    await save();
  }
  if (!report.account) {
    report.account = await call((c) =>
      c.sourceCollections.saveAccount({
        request_key: report.keys.account,
        expected_revision: null,
        account_id: report.account_id,
        label: "Pixiv 公开访问",
        mode: "anonymous",
      }),
    );
    await save();
  }
  if (!report.definition) {
    report.definition = {
      version: 1,
      collector: "pixiv_web_v1",
      library_id: report.lake.library_id,
      account_id: report.account.id,
      seeds: { kind: "authors", ids: [author] },
      scope: {
        work_types: ["illustration", "manga", "ugoira"],
        ratings: ["all_ages"],
        include_ai: true,
        include_unknown_markers: false,
      },
      discovery: {
        entrypoints: [],
        max_depth: 0,
        recommendation_seeds_per_author: 0,
      },
      media: {
        image_policy: {
          profile: "original",
          existing: "match_profile",
          allow_sample: false,
        },
        retain_original: true,
        ugoira: "archive_with_poster",
        reuse: { mode: "historical_if_same_locator", max_age_hours: 168 },
      },
      refresh: { mode: "missing_or_stale", max_age_hours: 168 },
      run_budget: {
        api_requests: 5000,
        admitted_authors: 1,
        download_bytes: 64 * 1024 ** 3,
        wall_seconds: 7200,
      },
    };
    await save();
  }
  if (!report.source) {
    await call((c) => c.openRecentProject(projectId));
    const sources = (await call((c) => c.sources(projectId))).items;
    report.source =
      sources.find(
        (s) => s.kind === "pixiv" && s.media_root === report.media_root,
      ) ??
      (await call((c) =>
        c.sourceAccess.attach(projectId, {
          kind: "auto",
          name: "Pixiv",
          media_root: report.media_root,
          index_root: report.index_root,
        }),
      ));
    await save();
  }
  const definition = includeUnknown
    ? {
        ...report.definition,
        scope: { ...report.definition.scope, include_unknown_markers: true },
      }
    : report.definition;
  if (includeUnknown) report.full_public_definition = definition;
  const first = await call((c) =>
    c.sourceCollections.create(definition, report.keys[firstStage]),
  );
  report[firstStage] = await settle(first.job.id, firstStage, !includeUnknown);
  const initialResult = report[firstStage];
  assert.ok(
    ["completed", "completed_with_gaps"].includes(initialResult.state),
    JSON.stringify({
      state: initialResult.state,
      reason: initialResult.wait_reason,
    }),
  );
  assert.equal(initialResult.progress.publication.pending_batches, 0);
  report.author = await call((c) =>
    c.sourceAccess.author(projectId, report.source.id, author),
  );
  let cursor,
    version,
    count = 0;
  do {
    const page = await call((c) =>
      c.sourceAccess.authorWorks(projectId, report.source.id, author, {
        limit: 200,
        ...(cursor ? { cursor, version } : {}),
      }),
    );
    count += page.items.length;
    cursor = page.next_cursor;
    version = page.version;
  } while (cursor);
  report.directory_count = count;
  assert.equal(initialResult.progress.works.planned, count);
  report.checks.push(
    "complete current public directory processed with distinct scope/visibility status",
    "pause waits for execution exit and resumes durable work",
  );
  const second = await call((c) =>
    c.sourceCollections.create(definition, report.keys[secondStage]),
  );
  report[secondStage] = await settle(second.job.id, secondStage);
  const incrementalResult = report[secondStage];
  assert.ok(
    ["completed", "completed_with_gaps"].includes(incrementalResult.state),
  );
  assert.equal(incrementalResult.progress.publication.pending_batches, 0);
  assert.ok(incrementalResult.progress.works.retained > 0);
  if (includeUnknown)
    assert.equal(incrementalResult.progress.works.excluded, 0);
  report.checks.push(
    "second snapshot reuses recent complete work and detects directory differences",
  );
  report.ok =
    initialResult.progress.task_gaps === 0 &&
    incrementalResult.progress.task_gaps === 0;
  report.visibility_verified = false;
  report.finished_at = new Date().toISOString();
} catch (e) {
  report.error = { name: e.name, code: e.code, message: e.message };
  process.exitCode = 1;
} finally {
  await save();
  await writeFile(
    resolve(run, "summary.json"),
    JSON.stringify(report, null, 2),
  );
  console.log(
    JSON.stringify({
      ok: report.ok,
      stage: report[secondStage]?.state ?? report[firstStage]?.state,
      report: resolve(run, "summary.json"),
      error: report.error,
    }),
  );
}
