import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import {
  mkdir,
  readFile,
  readdir,
  rename,
  symlink,
  writeFile,
} from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { EngineFixture, within } from "./engine-fixture.mjs";
import { pythonCommand } from "./platform.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local/test-runs",
  "integration-exports-" + Date.now(),
);
await mkdir(run, { recursive: true });
const engine = new EngineFixture(root, resolve(run, "state"));
const checks = [];
let movedMetadata;
process.on("exit", () => engine.child?.kill());
try {
  const fixtures = [];
  for (const name of ["a", "b"]) {
    await promisify(execFile)(
      pythonCommand(),
      [resolve(root, "tooling/ui-fixture.py"), resolve(run, name)],
      { windowsHide: true },
    );
    fixtures.push(
      JSON.parse(await readFile(resolve(run, name, "fixture.json"), "utf8")),
    );
  }
  await engine.start();
  const projects = [];
  for (const [index, fixture] of fixtures.entries()) {
    const project = await engine.api("/v1/projects", "POST", {
      name: "Export fixture " + index,
      parent_directory: resolve(run, "projects"),
    });
    const source = await engine.api(
      `/v1/projects/${project.id}/sources`,
      "POST",
      {
        kind: "danbooru",
        name: "Fixture " + index,
        index_root: fixture.lake,
        media_root: fixture.lake,
      },
    );
    projects.push({ project, source });
  }
  const { project, source } = projects[0];
  const base = `/v1/projects/${project.id}`;
  let revision = 0;
  const select = async (images) => {
    const value = await engine.api(base + "/selection", "PATCH", {
      expected_revision: revision,
      add: images.map((image) => ({
        source_id: source.id,
        asset_id: image.sha,
      })),
      remove: [],
      clear: true,
    });
    revision = value.revision;
  };
  const done = async (id) => {
    const list = await engine.wait(
      base + "/jobs",
      (data) =>
        data.items.some(
          (job) =>
            job.id === id &&
            ["succeeded", "failed", "cancelled"].includes(job.status),
        ),
      60000,
    );
    return list.items.find((job) => job.id === id);
  };
  const submit = async (destination, parameters = {}) => {
    const job = await engine.api(base + "/tools/jobs", "POST", {
      idempotency_key: crypto.randomUUID(),
      scope: {
        project_id: project.id,
        target: { kind: "selection", revision },
      },
      run: {
        operator_id: "core.export_files",
        operator_version: 1,
        parameters_version: 1,
        parameters: { destination, metadata: "tags", ...parameters },
      },
    });
    return done(job.id);
  };
  const rows = async (folder, name = "manifest.jsonl") =>
    (await readFile(resolve(folder, name), "utf8"))
      .trim()
      .split("\n")
      .map((line) => JSON.parse(line));
  const folder = async (name) => {
    const path = resolve(run, name);
    await mkdir(path, { recursive: true });
    return path;
  };
  await select([fixtures[0].images[0]]);
  await engine.api(`/v1/projects/${projects[1].project.id}/close`, "POST");
  const savePath = `${base}/sources/${source.id}/assets/${fixtures[0].images[0].sha}/original/save`;
  for (const lake of fixtures.map((f) => f.lake)) {
    await engine.expectError(
      savePath,
      "POST",
      { path: resolve(lake, "forbidden.png") },
      "INVALID_INPUT",
    );
    await assert.rejects(submit(lake), /INVALID_INPUT/);
  }
  const alias = resolve(run, "foreign-lake-alias");
  await symlink(
    fixtures[1].lake,
    alias,
    process.platform === "win32" ? "junction" : "dir",
  );
  await engine.expectError(
    savePath,
    "POST",
    { path: resolve(alias, "forbidden.png") },
    "INVALID_INPUT",
  );
  await assert.rejects(submit(alias), /INVALID_INPUT/);
  checks.push(
    "save and export reject all registered lake roots, closed-project sources and directory aliases",
  );

  const tags = await folder("tags");
  assert.equal(
    (await submit(tags, { tag_style: "comma" })).status,
    "succeeded",
  );
  const first = (await rows(tags))[0];
  const firstTags = await readFile(resolve(tags, first.metadata_file), "utf8");
  assert.equal(firstTags, "common, blue, solo, portrait");
  assert.equal(
    (await submit(tags, { tag_style: "space" })).status,
    "succeeded",
  );
  const changed = (await rows(tags))[0];
  assert.match(changed.file, /_2\.png$/);
  assert.equal(
    await readFile(resolve(tags, changed.metadata_file), "utf8"),
    "common blue solo portrait",
  );
  assert.equal(
    await readFile(resolve(tags, first.metadata_file), "utf8"),
    firstTags,
  );
  const names = await readdir(tags);
  assert.equal(
    (await submit(tags, { tag_style: "space" })).status,
    "succeeded",
  );
  assert.equal((await rows(tags))[0].status, "existing");
  assert.deepEqual(await readdir(tags), names);

  const unrelated = await folder("unrelated");
  await writeFile(
    resolve(unrelated, first.metadata_file),
    "unrelated existing text",
  );
  assert.equal((await submit(unrelated)).status, "succeeded");
  const paired = (await rows(unrelated))[0];
  assert.match(paired.file, /_2\.png$/);
  assert.equal(
    await readFile(resolve(unrelated, first.metadata_file), "utf8"),
    "unrelated existing text",
  );
  assert.equal(
    await readFile(resolve(unrelated, paired.metadata_file), "utf8"),
    firstTags,
  );

  const full = await folder("full");
  const oldJson = first.file.replace(/\.[^.]+$/, ".json");
  await writeFile(resolve(full, oldJson), "unrelated json");
  assert.equal((await submit(full, { metadata: "full" })).status, "succeeded");
  const fullRow = (await rows(full))[0];
  assert.match(fullRow.file, /_2\.png$/);
  assert.equal(
    JSON.parse(await readFile(resolve(full, fullRow.metadata_file), "utf8"))
      .asset.key.asset_id,
    fixtures[0].images[0].sha,
  );
  assert.equal((await submit(full, { metadata: "full" })).status, "succeeded");
  assert.equal((await rows(full))[0].status, "existing");
  checks.push(
    "changed tag styles and unrelated txt/json select a new matching pair; unchanged retries reuse it without touching old files",
  );

  const emptyTags = await folder("no-tags");
  const noTags = fixtures[0].images.find((image) => image.number === 12);
  await select([noTags]);
  await writeFile(resolve(emptyTags, `000001_${noTags.sha}.txt`), "stale tags");
  assert.equal((await submit(emptyTags)).status, "succeeded");
  const noTagsRow = (await rows(emptyTags))[0];
  assert.match(noTagsRow.file, /_2\.png$/);
  assert.equal(noTagsRow.metadata_file, undefined);
  assert.equal(noTagsRow.metadata_error, undefined);
  checks.push(
    "images without tags cannot acquire an unrelated existing tag file",
  );

  await select(fixtures[0].images.slice(0, 20));
  const metadata = within(
    run,
    resolve(fixtures[0].lake, "indexes/gen-ui/analysis.duckdb"),
  );
  const backup = within(run, metadata + ".export-test-backup");
  await rename(metadata, backup);
  movedMetadata = { metadata, backup };
  const incomplete = [];
  for (const manifest of [true, false]) {
    const destination = await folder("metadata-unavailable-" + manifest);
    const job = await submit(destination, { manifest });
    assert.equal(job.status, "failed");
    assert.match(job.error, /EXPORT_PARTIAL/);
    assert.equal(job.completed, 20);
    assert.equal(
      (await readdir(destination)).filter((name) => name.endsWith(".png"))
        .length,
      20,
    );
    const errors = await rows(destination, "export-errors.jsonl");
    assert.equal(errors.length, 20);
    assert.ok(errors.every((row) => row.metadata_error && !row.error));
    assert.equal(
      (await readdir(destination)).includes("manifest.jsonl"),
      manifest,
    );
    incomplete.push({ destination, job, manifest });
  }
  await rename(backup, metadata);
  movedMetadata = undefined;
  for (const { destination, job, manifest } of incomplete) {
    await engine.api(base + `/jobs/${job.id}/retry`, "POST");
    assert.equal((await done(job.id)).status, "succeeded");
    const files = await readdir(destination);
    assert.equal(files.filter((name) => name.endsWith(".png")).length, 20);
    assert.equal(files.filter((name) => name.endsWith(".txt")).length, 18);
    assert.ok(!files.includes("export-errors.jsonl"));
    assert.ok(!files.some((name) => name.startsWith(".studio-export-")));
    if (manifest)
      assert.ok(
        (await rows(destination)).every(
          (row) => row.status === "existing" && !row.metadata_error,
        ),
      );
  }
  checks.push(
    "metadata outages preserve all 20 originals, report partial completion and errors with manifest on/off; same-job retry repairs metadata without duplicate images",
  );
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify({ passed: true, checks }, null, 2),
  );
  console.log(
    JSON.stringify(
      { passed: true, checks, report: resolve(run, "report.json") },
      null,
      2,
    ),
  );
} finally {
  await engine.stop();
  if (movedMetadata) await rename(movedMetadata.backup, movedMetadata.metadata);
}
