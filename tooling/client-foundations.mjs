import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";
import { setImmediate } from "node:timers";
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
