// Explicit live validation only. Ordinary integration tests never contact Pixiv.
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { EngineFixture, sleep } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";
import { lakeWorkerPython } from "./lake-worker-runtime.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
function option(name) {
  const at = process.argv.indexOf(name);
  return at >= 0 ? process.argv[at + 1] : undefined;
}
const authorId = option("--author");
const output = option("--output");
assert.match(
  authorId ?? "",
  /^[1-9][0-9]{0,19}$/,
  "Supply an explicit author ID",
);
assert.ok(output, "Supply an isolated output directory");
const run = resolve(output);
await mkdir(run, { recursive: true });
const engine = new EngineFixture(root, resolve(run, "engine"));
const report = {
  author_id: authorId,
  mode: "anonymous",
  run,
  checks: [],
  ok: false,
};
try {
  await engine.start();
  const { StudioClient } = await clientFixture(root, resolve(run, "client"));
  const client = new StudioClient(engine.connection);
  await client.lakeUpdates.configure({
    python: lakeWorkerPython(root),
    state_root: resolve(run, "control"),
  });
  const lake = await client.sourceCollections.createLake({
    request_key: crypto.randomUUID(),
    site: "pixiv",
    media_root: resolve(run, "archive"),
    index_root: resolve(run, "online"),
  });
  report.lake = lake;
  const account = await client.sourceCollections.saveAccount({
    request_key: crypto.randomUUID(),
    expected_revision: null,
    account_id: crypto.randomUUID(),
    label: "Public validation",
    mode: "anonymous",
  });
  const shared = await client.lakeUpdates.pipeline();
  await client.lakeUpdates.savePipeline({
    expected_revision: shared.revision,
    value: {
      ...shared.value,
      max_download_mib: 32,
      reserve_mib: 128,
      max_image_pixels: 100_000_000,
    },
  });
  const definition = (kind, ids, metadataOnly, apiRequests) => ({
    version: 1,
    collector: "pixiv_web_v1",
    library_id: lake.library_id,
    account_id: account.id,
    seeds: { kind, ids },
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
        profile: metadataOnly ? "metadata_only" : "original",
        existing: "match_profile",
        allow_sample: false,
      },
      retain_original: !metadataOnly,
      ugoira: metadataOnly ? "metadata_only" : "archive_with_poster",
      reuse: { mode: "revalidate", max_age_hours: 0 },
    },
    run_budget: {
      api_requests: apiRequests,
      admitted_authors: 1,
      download_bytes: 128 * 1024 ** 2,
      wall_seconds: 300,
    },
  });
  async function settled(id) {
    let value;
    const deadline = Date.now() + 360_000;
    do {
      value = await client.sourceCollections.job(id);
      if (
        [
          "waiting_budget",
          "waiting_credentials",
          "needs_review",
          "completed",
          "completed_with_gaps",
          "paused",
          "cancelled",
        ].includes(value.state) &&
        !value.execution_active
      )
        return value;
      await sleep(1000);
    } while (Date.now() < deadline);
    await client.sourceCollections.action(id, {
      request_key: crypto.randomUUID(),
      expected_revision: value.revision,
      action: "pause",
    });
    throw new Error("Live validation reached its bounded wall-clock limit");
  }
  const directoryJob = await client.sourceCollections.create(
    definition("authors", [authorId], true, 2),
  );
  report.directory_job = await settled(directoryJob.job.id);
  await writeFile(
    resolve(run, "progress.json"),
    JSON.stringify(report, null, 2),
  );
  assert.equal(
    report.directory_job.progress.authors.scanned,
    1,
    JSON.stringify(report.directory_job),
  );
  const project = await client.createProject({
    name: "Pixiv public validation",
  });
  report.project = project;
  const source = await client.sourceAccess.attach(project.id, {
    kind: "auto",
    name: "Pixiv public",
    media_root: lake.media_root,
    index_root: lake.index_root,
  });
  const author = await client.sourceAccess.author(
    project.id,
    source.id,
    authorId,
  );
  report.author_name = author.observation.display_name;
  const directory = await client.sourceAccess.authorWorks(
    project.id,
    source.id,
    authorId,
    { limit: 200 },
  );
  report.directory_first_page = directory.items.length;
  report.directory_has_more = directory.next_cursor !== null;
  assert.ok(
    directory.items.length > 0,
    "The author had no publicly enumerated works",
  );
  const selected = directory.items
    .map((item) => item.work_id)
    .sort((a, b) => Number(BigInt(b) - BigInt(a)))
    .slice(0, 2);
  report.selected_work_ids = selected;
  const mediaJob = await client.sourceCollections.create(
    definition("works", selected, false, 12),
  );
  report.media_job = await settled(mediaJob.job.id);
  const rows = [];
  for (const id of selected) {
    const work = await client.sourceAccess.work(project.id, source.id, id);
    const media = await client.sourceAccess.workMedia(
      project.id,
      source.id,
      id,
      { limit: 200 },
    );
    rows.push({
      work_id: id,
      title: work.observation?.title,
      manifest_state: media.manifest_state,
      media_count: media.items.length,
      bindings: media.items.flatMap((item) => item.bindings),
    });
  }
  report.works = rows;
  const binding = rows
    .flatMap((row) => row.bindings)
    .find((item) => item.browsable_image);
  assert.ok(
    binding,
    "No public image was successfully acquired within the test budget",
  );
  const base = `/v1/projects/${project.id}/sources/${source.id}/assets/${binding.object_sha256}`;
  const metadata = await engine.api(base + "/metadata");
  const origins = await engine.api(
    base +
      `/records/${binding.record_id}/observations?version=${encodeURIComponent(metadata.version.token)}`,
  );
  const origin = origins.items.find((item) => item.relation === "asset_origin");
  const raw = await engine.api(
    base +
      `/records/${binding.record_id}/observations/${origin.observation_id}/raw?version=${encodeURIComponent(metadata.version.token)}`,
  );
  assert.equal(raw.status, "available");
  assert.equal(JSON.parse(raw.json).error, false);
  const image = await engine.response(base + "/media?edge=96");
  assert.equal(image.ok, true);
  await image.arrayBuffer();
  report.checks.push(
    "public author profile and directory",
    "bounded original-image collection",
    "Studio native work/page/metadata/raw and image-preview reads",
  );
  report.ok = true;
  report.visibility_verified = false;
} catch (error) {
  report.error = { name: error.name, message: error.message };
  process.exitCode = 1;
} finally {
  await engine.stop();
  await writeFile(
    resolve(run, "summary.json"),
    JSON.stringify(report, null, 2),
  );
  console.log(
    JSON.stringify({
      ok: report.ok,
      author_id: authorId,
      author_name: report.author_name,
      works: report.selected_work_ids,
      report: resolve(run, "summary.json"),
      error: report.error,
    }),
  );
}
