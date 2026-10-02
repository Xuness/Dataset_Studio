import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";
import { setImmediate } from "node:timers";
import { engineProfile } from "./engine-profile.mjs";
const structuredClone = globalThis.structuredClone;
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const directory = resolve(root, ".local/test-runs/client-foundations");
await mkdir(directory, { recursive: true });
async function compiled(source, name) {
  const code = ts.transpileModule(
    await readFile(resolve(root, source), "utf8"),
    {
      compilerOptions: {
        target: ts.ScriptTarget.ES2022,
        module: ts.ModuleKind.ESNext,
      },
    },
  ).outputText;
  const path = resolve(directory, name + ".mjs");
  await writeFile(path, code);
  return import(pathToFileURL(path).href + "?" + Date.now());
}
const { DraftCoordinator } = await compiled(
  "packages/client/src/drafts.ts",
  "drafts",
);
const { ModuleRegistry } = await compiled(
  "packages/ui/src/modules.ts",
  "modules",
);
const checks = [];
const rows = new Map();
const calls = [];
let hold = null;
const key = (pid, module, instance) => JSON.stringify([pid, module, instance]);
const port = {
  async get(pid, module, instance) {
    return {
      draft: structuredClone(rows.get(key(pid, module, instance)) ?? null),
    };
  },
  async save(pid, module, instance, request) {
    calls.push({ pid, module, instance, request: structuredClone(request) });
    if (hold) await hold;
    const id = key(pid, module, instance);
    const old = rows.get(id);
    if ((old?.revision ?? 0) !== request.expected_revision)
      throw Object.assign(new Error("conflict"), { code: "REVISION_CONFLICT" });
    const row = {
      project_id: pid,
      module_id: module,
      instance_id: instance,
      revision: request.expected_revision + 1,
      schema_version: request.schema_version,
      updated_at: "fixture",
      value: structuredClone(request.value),
    };
    rows.set(id, row);
    return row;
  },
  async preference(name) {
    return { preference: rows.get(name) ?? null };
  },
  async savePreference(name, request) {
    const old = rows.get(name);
    assert.equal(old?.revision ?? 0, request.expected_revision);
    const row = {
      key: name,
      schema_version: request.schema_version,
      revision: request.expected_revision + 1,
      value: request.value,
    };
    rows.set(name, row);
    return row;
  },
};
const decode = (v) =>
  v && typeof v === "object" && typeof v.name === "string" ? v : null;
const identity = (projectId) => ({
  projectId,
  moduleId: "core.query",
  instanceId: "default",
  schemaVersion: 1,
});
const coordinator = new DraftCoordinator(port);
const one = coordinator.open(identity("first"), { name: "" }, decode);
const two = coordinator.open(identity("second"), { name: "" }, decode);
await coordinator.flush();
assert.equal(calls.length, 0);
one.set({ name: "older" });
let release;
hold = new Promise((done) => {
  release = done;
});
const writing = one.flush();
await new Promise((done) => setImmediate(done));
one.set({ name: "newer" });
two.set({ name: "other project" });
assert.equal(one.getSnapshot().value.name, "newer");
release();
hold = null;
await writing;
await coordinator.flush("second");
assert.deepEqual(
  calls
    .filter((c) => c.pid === "first")
    .map((c) => [c.request.expected_revision, c.request.value.name]),
  [
    [0, "older"],
    [1, "newer"],
  ],
);
assert.equal(
  rows.get(key("first", "core.query", "default")).value.name,
  "newer",
);
assert.equal(
  rows.get(key("second", "core.query", "default")).value.name,
  "other project",
);
checks.push(
  "serial CAS writes coalesce edits; late responses never replace newer values or another project",
);
const before = calls.length;
one.set({ name: "a" });
one.set({ name: "b" });
one.set({ name: "c" });
await one.flush();
assert.equal(calls.length, before + 1);
rows.get(key("first", "core.query", "default")).revision++;
one.set({ name: "local conflict" });
await assert.rejects(one.flush());
assert.equal(one.getSnapshot().status, "conflict");
assert.equal(one.getSnapshot().value.name, "local conflict");
await assert.rejects(coordinator.flush("first"));
await one.keepLocal();
assert.equal(
  rows.get(key("first", "core.query", "default")).value.name,
  "local conflict",
);
rows.get(key("first", "core.query", "default")).value = {
  name: "remote choice",
};
await one.reload();
assert.equal(one.getSnapshot().value.name, "remote choice");
checks.push(
  "bounded coalescing; conflicts retain local edits and block close until explicit resolution",
);
rows.set(key("future", "core.query", "default"), {
  schema_version: 9,
  revision: 7,
  value: { unknown: "retained" },
});
const future = coordinator.open(identity("future"), { name: "" }, decode);
await future.flush();
assert.equal(future.getSnapshot().status, "unsupported");
assert.throws(() => future.set({ name: "overwrite" }));
assert.equal(
  rows.get(key("future", "core.query", "default")).value.unknown,
  "retained",
);
assert.throws(() => one.set({ name: "x".repeat(70000) }));
const pref = coordinator.preference(
  "studio.layout",
  { name: "default" },
  decode,
);
await pref.flush();
pref.set({ name: "compact" });
await coordinator.flush();
assert.equal(rows.get("studio.layout").value.name, "compact");
const replacement = { ...port, save: async (...args) => port.save(...args) };
coordinator.rebind(replacement);
assert.equal(coordinator.open(identity("first"), { name: "" }, decode), one);
checks.push(
  "future formats and oversized edits are preserved/rejected; preferences share flush semantics; reconnection retains controllers",
);
const registry = new ModuleRegistry();
const module = {
  id: "core.fixture",
  version: 1,
  protocolVersion: 1,
  draftSchema: { version: 1 },
  contributions: [
    {
      kind: "entry",
      id: "open",
      label: "Fixture",
      icon: "images",
      command: "fixture.open",
    },
    {
      kind: "command",
      id: "fixture.open",
      execute: (context) => (context.opened = true),
    },
  ],
};
registry.register(module);
registry.validate();
const context = { opened: false };
registry.execute("fixture.open", context);
assert.equal(context.opened, true);
assert.throws(() => registry.register(module));
assert.throws(() =>
  new ModuleRegistry().register({ ...module, protocolVersion: 2 }),
);
assert.throws(() =>
  new ModuleRegistry().register({
    ...module,
    contributions: [{ kind: "unknown", id: "unknown" }],
  }),
);
const invalid = new ModuleRegistry();
invalid.register({ ...module, contributions: [module.contributions[0]] });
assert.throws(() => invalid.validate());
checks.push(
  "module IDs, contribution types, protocol versions and command links validated before assembly",
);
const { initialHistory, restoreHistory, nextHistory } = await compiled(
  "packages/ui/src/browserHistory.ts",
  "browser-history",
);
let browserHistory = nextHistory(
  nextHistory(initialHistory(), "page-two"),
  "page-three",
);
const restoredHistory = restoreHistory(
  {
    scopeKey: "source",
    cursor: "page-three",
    pageNumber: 3,
    pageSize: 48,
    anchor: null,
    version: "v1",
    history: JSON.parse(JSON.stringify(browserHistory)),
  },
  "source",
);
assert.deepEqual(restoredHistory, browserHistory);
assert.equal(restoredHistory.cursors[restoredHistory.index - 1], "page-two");
browserHistory = nextHistory(
  { ...restoredHistory, index: 1 },
  "new-page-three",
);
assert.equal(browserHistory.cursors.at(-1), "new-page-three");
assert.ok(!browserHistory.cursors.includes("page-three"));
for (let i = 0; i < 300; i++)
  browserHistory = nextHistory(
    browserHistory,
    "cursor-" + i + "x".repeat(1024),
  );
assert.ok(browserHistory.cursors.length <= 128);
assert.ok(JSON.stringify(browserHistory).length <= 48 * 1024);
assert.ok(browserHistory.firstPage > 1);
assert.deepEqual(
  restoreHistory(
    {
      scopeKey: "other",
      cursor: "old",
      pageNumber: 9,
      pageSize: 48,
      anchor: null,
      version: "v1",
    },
    "source",
  ),
  initialHistory(),
);
checks.push(
  "browser history survives view remounts, forks correctly and remains bounded in pages and bytes",
);
const { querySignature } = await compiled(
  "packages/ui/src/querySemantics.ts",
  "query-semantics",
);
const queryA = {
  source_ids: ["lake"],
  conditions: [
    {
      field: "observation.rating",
      operator: "in",
      value: { type: "text_list", value: ["s", "g"] },
    },
    {
      field: "observation.tag_string",
      operator: "has_no_tags",
      value: { type: "text_list", value: ["comic", "group"] },
    },
  ],
  observation_rule: "current_post",
  order: "identity",
  version: 2,
};
const queryB = {
  ...queryA,
  conditions: [
    {
      ...queryA.conditions[1],
      value: { type: "text_list", value: ["group", "comic"] },
    },
    {
      ...queryA.conditions[0],
      value: { type: "text_list", value: ["g", "s"] },
    },
  ],
};
assert.equal(querySignature(queryA), querySignature(queryB));
assert.notEqual(
  querySignature(queryA),
  querySignature({ ...queryB, observation_rule: "origin" }),
);
assert.notEqual(
  querySignature(queryA),
  querySignature({
    ...queryB,
    input_scope: {
      project_id: "project",
      target: { kind: "selection", revision: 3 },
    },
  }),
);
checks.push(
  "saved query identity follows condition/set semantics while preserving observation rule and scope differences",
);
const { jobPresentation, isJobActive } = await compiled(
  "packages/ui/src/jobPresentation.ts",
  "jobPresentation",
);
const progressJob = {
  id: "full-lake",
  project_id: "fixture",
  operator: "danbooru.metarecall",
  status: "preparing",
  input_members_frozen: true,
  total: 11493687,
  completed: 11493687,
  attempt: 0,
  created_at: "1788961094870",
  error: null,
  stage: { name: "metadata_snapshot", completed: 7629312, total: 11493687 },
};
assert.equal(jobPresentation(progressJob).percent.toFixed(1), "66.4");
assert.equal(jobPresentation(progressJob).phase, 1);
assert.equal(isJobActive(progressJob), true);
for (const stage of ["indexing", "publishing", "complete"]) {
  const p = jobPresentation({
    ...progressJob,
    status: "running",
    stage: { name: stage, completed: 1, total: 1 },
  });
  assert.equal(
    p.percent,
    null,
    "A complete worker counter must not report published completion",
  );
  assert.equal(p.succeeded, false);
  assert.equal(p.phase, 3);
}
const unknown = jobPresentation({
  ...progressJob,
  status: "waiting_input",
  total: 0,
  input_members_frozen: false,
});
assert.equal(unknown.percent, null);
assert.equal(unknown.phase, 0, "A queued retry ignores an old attempt's stage");
assert.equal(
  jobPresentation({ ...progressJob, status: "succeeded" }).percent,
  100,
);
assert.equal(isJobActive({ status: "failed" }), false);
const generic = jobPresentation({
  ...progressJob,
  operator: "core.manifest",
  status: "running",
  completed: 25,
  total: 100,
  stage: undefined,
});
assert.equal(generic.percent, 25);
assert.equal(generic.progressLabel, "处理进度");
checks.push(
  "multi-stage task progress uses the current stage, keeps finalization indeterminate, distinguishes unknown input, and retains item progress for ordinary tools",
);
assert.equal(engineProfile([], {}), "release");
assert.equal(engineProfile(["--engine-profile=debug"], {}), "debug");
assert.equal(
  engineProfile(["--release"], { STUDIO_ENGINE_PROFILE: "debug" }),
  "release",
);
assert.equal(engineProfile([], {}, "debug"), "debug");
assert.throws(() => engineProfile(["--engine-profile"], {}));
assert.throws(() => engineProfile([], { STUDIO_ENGINE_PROFILE: "other" }));
assert.equal(
  jobPresentation({
    ...progressJob,
    status: "running",
    stage: { name: "writing", rating: "q", completed: 50, total: 100 },
  }).title,
  "保存评分明细 · Q 分级",
);
assert.equal(
  jobPresentation({
    ...progressJob,
    status: "running",
    stage: { name: "output_checksum", completed: 1048576, total: 2097152 },
  }).count,
  "1.0 MiB / 2.0 MiB",
);
checks.push(
  "engine profile defaults to optimized execution with explicit overrides, and ranking stages preserve rating and byte units",
);
const { rankingLabel, comparisonDeltaLabel } = await compiled(
  "apps/desktop/src/features/aesthetic/analysisPresentation.ts",
  "aesthetic-presentation",
);
const rankingRow = {
  position: 999,
  rating: "g",
  component: 7,
  rank_min: 2,
  rank_max: 3,
  rating_rank_min: null,
  rating_rank_max: null,
};
assert.equal(rankingLabel(rankingRow), "分量 7 · 第 2–3 名");
assert.equal(
  rankingLabel({ ...rankingRow, rating_rank_min: 5, rating_rank_max: 6 }),
  "G · 第 5–6 名",
);
assert.equal(
  rankingLabel({ ...rankingRow, rank_min: null, rank_max: null }),
  "暂无可比名次",
);
assert.equal(
  rankingLabel({
    ...rankingRow,
    rating: "s",
    rating_rank_min: 1,
    rating_rank_max: 1,
  }),
  "S · 第 1 名",
);
checks.push(
  "aesthetic labels preserve ties and component boundaries without exposing pagination position as rank",
);
const comparable = {
  comparable: true,
  left_percentile: 0.8,
  right_percentile: 0.3,
  percentile_delta: 0.5,
};
assert.equal(comparisonDeltaLabel(comparable), "上移 50.00 个百分点");
assert.equal(
  comparisonDeltaLabel({
    ...comparable,
    left_percentile: 0.3,
    right_percentile: 0.8,
  }),
  "下移 50.00 个百分点",
);
assert.equal(
  comparisonDeltaLabel({
    ...comparable,
    comparable: false,
    reason: "候选范围不同",
  }),
  "候选范围不同",
);
assert.equal(
  comparisonDeltaLabel({ ...comparable, right_percentile: null }),
  "不可比较",
);
const { rankingBrowserInitial, decodeRankingBrowser } = await compiled(
  "apps/desktop/src/features/aesthetic/rankingBrowserState.ts",
  "ranking-browser-state",
);
const legacyBrowser = {
  ...rankingBrowserInitial,
  thumbnailSize: undefined,
  scrollTop: undefined,
};
assert.equal(decodeRankingBrowser(legacyBrowser).thumbnailSize, 208);
assert.equal(decodeRankingBrowser(legacyBrowser).scrollTop, 0);
assert.equal(
  decodeRankingBrowser({ ...legacyBrowser, past: Array(65).fill("") }),
  null,
);
checks.push(
  "comparison direction uses actual A/B percentiles, preserves unknowns, and old ranking sessions restore with bounded display defaults",
);
const { commonQueryFields } = await compiled(
  "packages/client/src/queryFields.ts",
  "query-fields",
);
const fieldContract = (id, field_type, operators, unit = null) => ({
  id,
  name: id,
  field_type,
  operators,
  unit,
  sortable: false,
});
const directoryContract = (fields, orders, rules, max_conditions = 12) => ({
  fields,
  orders,
  observation_rules: rules,
  max_conditions,
});
const combinedFields = commonQueryFields([
  directoryContract(
    [
      fieldContract("rating", "text", ["eq", "in"]),
      fieldContract("fav_count", "integer", ["gte"]),
      fieldContract("size", "integer", ["gte"], "byte"),
      fieldContract("ambiguous", "integer", ["eq"]),
    ],
    ["post_id_asc", "asset_key_asc"],
    ["current_post", "any_observation"],
  ),
  directoryContract(
    [
      fieldContract("rating", "text", ["eq"]),
      fieldContract("size", "integer", ["gte"], "pixel"),
      fieldContract("ambiguous", "text", ["eq"]),
    ],
    ["asset_key_asc"],
    ["current_post"],
    4,
  ),
]);
assert.deepEqual(
  combinedFields.fields.map((field) => [field.id, field.operators]),
  [["rating", ["eq"]]],
);
assert.deepEqual(combinedFields.orders, ["asset_key_asc"]);
assert.deepEqual(combinedFields.observation_rules, ["current_post"]);
assert.equal(combinedFields.max_conditions, 4);
assert.deepEqual(commonQueryFields([]).fields, []);
const literalTags = {
  ...fieldContract("tags", "tags", ["has_tag"]),
  basis: "work_tags.literal_tag",
};
const booruTags = { ...literalTags, basis: "observations.tag_string" };
for (const fields of [
  [literalTags, booruTags],
  [booruTags, literalTags],
]) {
  const mixed = commonQueryFields(
    fields.map((field) =>
      directoryContract([field], ["asset_key_asc"], ["current_post"], 12),
    ),
  );
  assert.equal(mixed.fields[0].basis, booruTags.basis);
}
checks.push(
  "multi-source field contracts preserve only compatible types, units, operators, orders and observation rules",
);
await writeFile(
  resolve(directory, "report.json"),
  JSON.stringify({ passed: true, checks }, null, 2),
);
console.log(
  JSON.stringify(
    { passed: true, checks, report: resolve(directory, "report.json") },
    null,
    2,
  ),
);
