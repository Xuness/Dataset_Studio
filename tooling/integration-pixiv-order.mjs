import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { EngineFixture } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";
import { lakeWorkerPython } from "./lake-worker-runtime.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(root, ".local/test-runs", "pixiv-order-" + Date.now());
await mkdir(run, { recursive: true });
const fixture = async (...args) => {
  await promisify(execFile)(
    lakeWorkerPython(root),
    [resolve(root, "tooling/pixiv-order-fixture.py"), run, ...args],
    { windowsHide: true },
  );
  return JSON.parse(await readFile(resolve(run, "pixiv-order.json"), "utf8"));
};
const engine = new EngineFixture(root, resolve(run, "engine"));
const checks = [];
try {
  const before = await fixture();
  await engine.start();
  const { StudioClient } = await clientFixture(root, resolve(run, "client"));
  const client = new StudioClient(engine.connection);
  const project = await client.createProject({
    name: "Pixiv order",
    parent_directory: resolve(run, "projects"),
  });
  const source = await client.sourceAccess.attach(project.id, {
    kind: "pixiv",
    name: "Pixiv order",
    index_root: before.index_root,
    media_root: before.media_root,
  });
  const fields = await client.queries.fields(project.id, source.id);
  assert.ok(source.descriptor.capabilities.post_order);
  assert.ok(
    fields.orders.includes("post_id_asc") &&
      fields.orders.includes("post_id_desc"),
  );
  const spec = (order, filtered = false) => ({
    version: 3,
    source_ids: [source.id],
    conditions: filtered
      ? [
          {
            field: "tags",
            operator: "has_tag",
            value: { type: "text", value: "order_fixture" },
          },
        ]
      : [],
    observation_rule: "current_post",
    order,
  });
  const expected = (refs, order, filtered) => {
    const descending = order === "post_id_desc";
    const rows = Object.entries(refs.positions)
      .sort(([ak, [ap, ao]], [bk, [bp, bo]]) => {
        const a = BigInt(ap),
          b = BigInt(bp);
        return (
          (a === b ? 0 : (a < b ? -1 : 1) * (descending ? -1 : 1)) ||
          ao - bo ||
          ak.localeCompare(bk) * (descending ? -1 : 1)
        );
      })
      .map(([key]) => key);
    return filtered ? rows : [...rows, refs.unlinked];
  };
  const allPages = async (result, order, limit = 7) => {
    const keys = [];
    let cursor;
    for (let n = 0; n < 100; n++) {
      const { page } = await client.queries.assets(project.id, result.id, {
        order,
        limit,
        ...(cursor ? { cursor } : {}),
      });
      assert.ok(!page.preparing);
      keys.push(...page.items.map((item) => item.key.asset_id));
      cursor = page.next_cursor;
      if (!cursor) return keys;
    }
    throw new Error("Paging did not terminate");
  };
  const saved = [];
  for (const filtered of [false, true]) {
    const view = await client.queries.browse(
      project.id,
      spec("post_id_asc", filtered),
    );
    for (const order of ["post_id_asc", "post_id_desc"])
      assert.deepEqual(
        await allPages(view, order),
        expected(before, order, filtered),
      );
    const capture = await client.queries.capture(project.id, {
      project_id: project.id,
      target: { kind: "query_result", result_id: view.id },
    });
    const fixed = await engine.wait(
      `/v1/projects/${project.id}/query-results/${capture.id}`,
      (r) => !["queued", "running"].includes(r.state),
    );
    assert.equal(fixed.state, "ready");
    for (const order of ["post_id_asc", "post_id_desc"])
      assert.deepEqual(
        await allPages(fixed, order),
        expected(before, order, filtered),
      );
    saved.push({ view, fixed, filtered });
  }
  checks.push(
    "numeric IDs through 20 digits, natural page order in both directions, duplicate objects, unlinked-last, Tag filtering, direct and captured result pagination",
  );
  const workset = await client.createCollection(
    project.id,
    "Ordered Pixiv workset",
    {
      project_id: project.id,
      target: { kind: "query_result", result_id: saved[0].fixed.id },
    },
  );
  const selection = await client.selection(project.id);
  await engine.api(`/v1/projects/${project.id}/selection`, "PATCH", {
    expected_revision: selection.revision,
    add: expected(before, "post_id_asc", false).map((asset_id) => ({
      source_id: source.id,
      asset_id,
    })),
    remove: [],
    clear: false,
  });
  for (const scope of [{ collectionId: workset.id }, { selection: true }])
    for (const order of ["post_id_asc", "post_id_desc"]) {
      let cursor;
      const keys = [];
      do {
        const page = await client.assets(project.id, {
          ...scope,
          order,
          limit: 7,
          cursor,
        });
        keys.push(...page.items.map((item) => item.key.asset_id));
        cursor = page.next_cursor ?? undefined;
      } while (cursor);
      assert.deepEqual(keys, expected(before, order, false));
    }
  checks.push(
    "workset and selection pages use the same numeric ID and natural page order",
  );

  const otherRoot = resolve(run, "other");
  await promisify(execFile)(
    lakeWorkerPython(root),
    [resolve(root, "tooling/pixiv-order-fixture.py"), otherRoot],
    { windowsHide: true },
  );
  const otherRefs = JSON.parse(
    await readFile(resolve(otherRoot, "pixiv-order.json"), "utf8"),
  );
  const other = await client.sourceAccess.attach(project.id, {
    kind: "pixiv",
    name: "Other Pixiv",
    media_root: otherRefs.media_root,
    index_root: otherRefs.index_root,
  });
  const mixed = await client.queries.browse(project.id, {
    ...spec("post_id_asc"),
    source_ids: [source.id, other.id],
  });
  for (const order of ["post_id_asc", "post_id_desc"]) {
    const direction = order.endsWith("desc") ? -1 : 1;
    const lex = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
    const wanted = [source, other].flatMap((s) =>
      expected(before, order, false).map((sha) => ({
        source: s.id,
        sha,
        position: before.positions[sha],
      })),
    );
    wanted.sort((a, b) => {
      if (!a.position && b.position) return 1;
      if (a.position && !b.position) return -1;
      const id =
        a.position && b.position
          ? lex(BigInt(a.position[0]), BigInt(b.position[0]))
          : 0;
      return (
        direction * (id || lex(a.source, b.source)) ||
        (a.position?.[1] ?? 0) - (b.position?.[1] ?? 0) ||
        direction * lex(a.sha, b.sha)
      );
    });
    const actual = [];
    let cursor;
    do {
      const { page } = await client.queries.assets(project.id, mixed.id, {
        order,
        limit: 7,
        cursor,
      });
      actual.push(
        ...page.items.map(
          (item) => item.key.source_id + ":" + item.key.asset_id,
        ),
      );
      cursor = page.next_cursor ?? undefined;
    } while (cursor);
    assert.deepEqual(
      actual,
      wanted.map((item) => item.source + ":" + item.sha),
    );
  }
  checks.push(
    "multiple Pixiv lakes merge without losing within-work page order or 20-digit numeric ID precision",
  );
  const after = await fixture("append");
  for (const { view, fixed, filtered } of saved)
    for (const result of [view, fixed])
      assert.deepEqual(
        await allPages(result, "post_id_desc"),
        expected(before, "post_id_desc", filtered),
      );
  const latest = await client.queries.browse(project.id, spec("post_id_asc"));
  assert.deepEqual(
    await allPages(latest, "post_id_asc"),
    expected(after, "post_id_asc", false),
  );
  checks.push(
    "retained views and captured results preserve old work/page associations after publication, while fresh views use the new minimum work ID",
  );
  await engine.stop();
  await engine.start();
  const reopened = new StudioClient(engine.connection);
  await reopened.openProject(project.directory);
  const page = await reopened.queries.assets(project.id, saved[0].fixed.id, {
    order: "post_id_desc",
    limit: 7,
  });
  assert.deepEqual(
    page.page.items.map((item) => item.key.asset_id),
    expected(before, "post_id_desc", false).slice(0, 7),
  );
  checks.push("captured Pixiv order survives engine restart");
  console.log(JSON.stringify({ ok: true, checks, run }));
  await writeFile(
    resolve(run, "summary.json"),
    JSON.stringify({ ok: true, checks, run }, null, 2),
  );
} finally {
  await engine.stop();
}
