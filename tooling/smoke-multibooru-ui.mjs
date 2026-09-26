import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, readFile, writeFile, open, rename } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";
import { EngineFixture, sleep, within } from "./engine-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const fixture = resolve(process.argv[2] ?? "");
assert.ok(fixture.startsWith(resolve(root, ".local/test-runs/multibooru-")));
const lakes = JSON.parse(
  await readFile(resolve(fixture, "multibooru.json"), "utf8"),
);
const run = resolve(root, ".local/test-runs/smoke-multibooru-ui-" + Date.now());
await mkdir(run, { recursive: true });
const engine = new EngineFixture(root, resolve(run, "state"));
const url = "http://127.0.0.1:1458";
let browser, vite, page;
let offlineFile;
const errors = [],
  requests = [],
  checks = [];
try {
  await engine.start();
  const project = await engine.api("/v1/projects", "POST", {
    name: "Multibooru UI",
    parent_directory: resolve(run, "projects"),
  });
  const log = await open(resolve(run, "vite.log"), "a");
  vite = spawn(
    process.execPath,
    [
      resolve(root, "apps/desktop/node_modules/vite/bin/vite.js"),
      "--port",
      "1458",
      "--strictPort",
    ],
    {
      cwd: resolve(root, "apps/desktop"),
      env: { ...process.env, STUDIO_DATA_DIR: resolve(run, "state") },
      windowsHide: true,
      stdio: ["ignore", log.fd, log.fd],
    },
  );
  await log.close();
  for (let i = 0; i < 100; i++) {
    if ((await fetch(url).catch(() => null))?.ok) break;
    if (vite.exitCode !== null) throw new Error("Vite stopped");
    await sleep(100);
  }
  browser = await chromium.launch({ channel: "msedge", headless: true });
  const context = await browser.newContext({
    viewport: { width: 1920, height: 1080 },
  });
  await context.route(url + "/__studio/connection", (route) =>
    route.fulfill({ json: engine.connection }),
  );
  await context.route(engine.connection.endpoint + "/**", async (route) => {
    const request = route.request(),
      headers = { ...request.headers() };
    delete headers.host;
    delete headers.origin;
    const cors = {
      "access-control-allow-origin": url,
      "access-control-allow-methods": "GET,POST,PUT,PATCH,OPTIONS",
      "access-control-allow-headers":
        request.headers()["access-control-request-headers"] ??
        "authorization,content-type,x-studio-session",
    };
    if (request.method() === "OPTIONS")
      return route.fulfill({ status: 204, headers: cors });
    if (request.url().includes("/events?")) return route.abort();
    try {
      const response = await fetch(request.url(), {
        method: request.method(),
        headers,
        ...(request.postData() ? { body: request.postData() } : {}),
      });
      await route.fulfill({
        status: response.status,
        headers: { ...Object.fromEntries(response.headers), ...cors },
        body: Buffer.from(await response.arrayBuffer()),
      });
    } catch (error) {
      if (!page?.isClosed()) throw error;
    }
  });
  page = await context.newPage();
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("request", (request) => {
    if (request.method() === "POST" && request.url().endsWith("/query-results"))
      requests.push(request.postDataJSON());
  });
  await page.goto(url);
  await page.locator(".recent-row").filter({ hasText: project.name }).click();
  await page
    .getByRole("button", { name: "添加第一个数据湖", exact: true })
    .click();
  const dialog = page.getByRole("dialog");
  const added = [];
  async function attachLake(kind, name) {
    await dialog
      .getByLabel("数据源类型", { exact: true })
      .selectOption(kind === "danbooru" ? kind : "auto");
    await dialog
      .getByLabel("快速索引目录", { exact: true })
      .fill(lakes[kind].lake);
    await dialog
      .getByLabel("图片湖目录", { exact: true })
      .fill(lakes[kind].lake);
    await dialog
      .getByRole("button", { name: "检查数据湖", exact: true })
      .click();
    await expect(dialog.getByRole("status")).toContainText("已识别 " + name);
    await page.screenshot({ path: resolve(run, kind + "-source-probe.png") });
    await dialog.getByRole("button", { name: "加入项目", exact: true }).click();
    await expect(dialog).toHaveCount(0);
    added.push(lakes[kind].library_id);
    const sources = await engine.api(`/v1/projects/${project.id}/sources`);
    assert.deepEqual(
      sources.items.map((source) => source.id).sort(),
      [...added].sort(),
    );
    checks.push(
      name + " attached through UI without replacing previous sources",
    );
  }
  await attachLake("danbooru", "Danbooru");
  const addSource = page.getByRole("button", {
    name: "添加数据湖",
    exact: true,
  });
  await expect(addSource).toBeVisible();
  await page.screenshot({ path: resolve(run, "add-after-danbooru.png") });
  await addSource.click();
  await attachLake("yandere", "Yandere");
  await expect(addSource).toBeVisible();

  // The project menu remains usable when the source tree is hidden.
  async function toggleProjectPanel() {
    await page.getByRole("menuitem", { name: "窗口", exact: true }).click();
    await page.getByRole("menuitem", { name: "项目面板", exact: true }).click();
  }
  await toggleProjectPanel();
  await expect(addSource).toBeHidden();
  await page.getByRole("menuitem", { name: "项目", exact: true }).click();
  await page.screenshot({ path: resolve(run, "add-from-project-menu.png") });
  await page
    .getByRole("menuitem", { name: "添加数据湖…", exact: true })
    .click();
  await attachLake("gelbooru", "Gelbooru");
  await toggleProjectPanel();
  await expect(page.locator(".project-foot")).toContainText("3 个数据湖");
  await page.reload();
  await expect(page.locator(".project-foot")).toContainText("3 个数据湖");
  await expect(addSource).toBeVisible();
  for (const name of ["Danbooru", "Yandere", "Gelbooru"]) {
    const source = page
      .locator(".source-tree-row button.tree-row")
      .filter({ hasText: name });
    await expect(source).toHaveCount(1);
    await source.click();
    await expect(source).toHaveClass(/active/);
    await expect(page.locator(".asset-grid article").first()).toBeVisible({
      timeout: 30000,
    });
  }
  await page.screenshot({ path: resolve(run, "three-sources.png") });
  checks.push(
    "persistent add button after first source and project menu with hidden source panel",
    "three sources survive page reload and remain independently browsable",
  );
  await page
    .locator("button.tree-row")
    .filter({ hasText: "Gelbooru" })
    .first()
    .click();
  await page.getByRole("button", { name: "定位与显示", exact: true }).click();
  await page
    .getByLabel("浏览排序", { exact: true })
    .selectOption("post_id_asc");
  const card = page.getByRole("button", {
    name: "查看 Gelbooru #10001",
    exact: true,
  });
  await expect(card).toBeVisible({ timeout: 30000 });
  await card.click();
  await page.getByRole("tab", { name: "检查器", exact: true }).click();
  await page.getByRole("tab", { name: "属性", exact: true }).click();
  const inspector = page.getByRole("region", {
    name: "元数据检查",
    exact: true,
  });
  await expect(inspector).toContainText("32 × 40", { timeout: 30000 });
  const tag = "censored\n";
  await expect(
    inspector.getByRole("button", { name: JSON.stringify(tag), exact: true }),
  ).toBeVisible();
  await page.screenshot({ path: resolve(run, "gelbooru-inspector.png") });
  await inspector
    .getByRole("button", { name: JSON.stringify(tag), exact: true })
    .click();
  const input = page.getByLabel("条件值 1", { exact: true });
  await expect(input).toHaveValue(JSON.stringify(tag));
  const submitted = page.waitForResponse(
    (r) =>
      r.request().method() === "POST" && r.url().endsWith("/query-results"),
  );
  await page.getByRole("button", { name: "执行查询", exact: true }).click();
  const created = await (await submitted).json();
  const completed = await engine.wait(
    `/v1/projects/${project.id}/query-results/${created.id}`,
    (r) => !["queued", "running"].includes(r.state),
  );
  assert.equal(completed.state, "ready");
  assert.equal(completed.count, 1);
  await expect(page.locator(".query-result-row.current")).toContainText(
    "1 张",
    { timeout: 30000 },
  );
  await expect(page.locator(".asset-grid article")).toHaveCount(1);
  await expect.poll(() => requests.length).toBe(1);
  assert.equal(requests[0].spec.conditions[0].value.value, tag);
  await expect(
    page.getByRole("button", { name: "查看 Gelbooru #10001", exact: true }),
  ).toBeVisible({ timeout: 30000 });
  await page.screenshot({ path: resolve(run, "exact-tag-query.png") });
  const sourceChoices = page.getByRole("group", {
    name: "查询数据湖",
    exact: true,
  });
  await sourceChoices.getByLabel("Yandere", { exact: true }).check();
  await expect(
    page.getByLabel("条件字段 1").locator('option[value="fav_count"]'),
  ).toHaveCount(0);
  await page.getByLabel("查询名称", { exact: true }).fill("Yandere + Gelbooru");
  const saving = page.waitForResponse(
    (r) => r.request().method() === "POST" && r.url().endsWith("/queries"),
  );
  await page.getByRole("button", { name: "保存条件", exact: true }).click();
  const saved = await (await saving).json();
  assert.deepEqual(
    saved.spec.source_ids,
    [lakes.yandere.library_id, lakes.gelbooru.library_id].sort(),
  );
  async function runEditor() {
    const response = page.waitForResponse(
      (r) =>
        r.request().method() === "POST" &&
        /\/(query-results|queries\/[^/]+\/results)$/.test(r.url()),
    );
    await page.getByRole("button", { name: "执行查询", exact: true }).click();
    const submitted = await (await response).json();
    const result = await engine.wait(
      `/v1/projects/${project.id}/query-results/${submitted.id}`,
      (r) => !["queued", "running"].includes(r.state),
    );
    assert.equal(result.state, "ready", JSON.stringify(result));
    assert.equal(result.count, 2);
    await expect(page.locator(".query-result-row.current")).toContainText(
      "2 张",
      { timeout: 30000 },
    );
    await expect(page.locator(".asset-grid article")).toHaveCount(2);
    return result;
  }
  await runEditor();
  assert.equal((await runEditor()).cache.mode, "reused");
  await page.screenshot({ path: resolve(run, "mixed-query-cache.png") });
  await sourceChoices.getByLabel("Danbooru", { exact: true }).check();
  await sourceChoices.getByLabel("Yandere", { exact: true }).uncheck();
  await sourceChoices.getByLabel("Gelbooru", { exact: true }).uncheck();
  await page.getByLabel("条件字段 1").selectOption("fav_count");
  await sourceChoices.getByLabel("Yandere", { exact: true }).check();
  await expect(
    page.getByRole("button", { name: "执行查询", exact: true }),
  ).toBeDisabled();
  await expect(page.getByLabel("条件字段 1")).toHaveValue("fav_count");
  await expect(page.locator(".condition-error").first()).toContainText(
    "不可用",
  );
  await page.getByLabel("已保存查询", { exact: true }).selectOption("");
  await page.getByLabel("已保存查询", { exact: true }).selectOption(saved.id);
  const draftPath = `/v1/projects/${project.id}/drafts/core.query/default`;
  await engine.wait(
    draftPath,
    (r) =>
      r.draft?.value.sourceIds?.length === 2 &&
      r.draft.value.conditions[0]?.field === "tags",
  );
  await page.reload();
  await expect(
    sourceChoices.getByLabel("Yandere", { exact: true }),
  ).toBeChecked();
  await expect(
    sourceChoices.getByLabel("Gelbooru", { exact: true }),
  ).toBeChecked();
  await expect(
    sourceChoices.getByLabel("Danbooru", { exact: true }),
  ).not.toBeChecked();
  await expect(page.getByLabel("查询名称", { exact: true })).toBeEnabled();
  checks.push(
    "multi-lake queries can be edited, saved, restored and reused; incompatible existing conditions are preserved and blocked",
  );

  const workset = await engine.api(
    `/v1/projects/${project.id}/collections`,
    "POST",
    {
      name: "Gelbooru fixed sample",
      scope: {
        project_id: project.id,
        target: { kind: "query_result", result_id: completed.id },
      },
    },
  );
  const online = within(fixture, resolve(lakes.yandere.lake, "library.json"));
  const offline = online + ".offline-test";
  await rename(online, offline);
  offlineFile = { online, offline };
  await page.reload();
  const listedSources = await engine.api(`/v1/projects/${project.id}/sources`);
  assert.equal(
    listedSources.items.find((s) => s.id === lakes.yandere.library_id)
      .available,
    false,
  );
  await page
    .locator(".workset-tree button.tree-row")
    .filter({ hasText: workset.name })
    .click();
  await page.getByRole("tab", { name: "筛选", exact: true }).click();
  const filters = page.getByRole("region", { name: "浏览筛选", exact: true });
  await filters
    .getByLabel("包含标签", { exact: true })
    .fill(JSON.stringify(tag));
  const filtering = page.waitForResponse(
    (r) =>
      r.request().method() === "POST" && r.url().endsWith("/query-results"),
  );
  await filters.getByRole("button", { name: "应用筛选", exact: true }).click();
  const filtered = await (await filtering).json();
  assert.deepEqual(filtered.spec.source_ids, [lakes.gelbooru.library_id]);
  assert.equal(filtered.spec.input_scope.target.collection_id, workset.id);
  const fixedResult = await engine.wait(
    `/v1/projects/${project.id}/query-results/${filtered.id}`,
    (r) => !["queued", "running"].includes(r.state),
  );
  assert.equal(fixedResult.state, "ready");
  assert.equal(fixedResult.count, 1);
  await expect(filters).toContainText("1 张匹配", { timeout: 30000 });
  await expect(
    page.locator('[title="筛选 · Gelbooru fixed sample"]'),
  ).toBeVisible();
  await expect(page.locator(".asset-grid article")).toHaveCount(1);
  await page.screenshot({
    path: resolve(run, "scoped-filter-unrelated-offline.png"),
  });
  await rename(offline, online);
  offlineFile = undefined;
  checks.push(
    "workset quick filtering uses only its actual lake and works with an unrelated lake offline",
  );

  const oldDraft = await engine.api(draftPath);
  await engine.api(draftPath, "PUT", {
    schema_version: 1,
    expected_revision: oldDraft.draft.revision,
    value: {
      definition: null,
      name: "Legacy source draft",
      sourceId: lakes.gelbooru.library_id,
      conditions: completed.spec.conditions,
      rule: "current_post",
      order: "post_id_desc",
      inputScope: null,
    },
  });
  await page.reload();
  await page.getByRole("tab", { name: "项目查询", exact: true }).click();
  await expect(
    sourceChoices.getByLabel("Gelbooru", { exact: true }),
  ).toBeChecked();
  await expect(
    sourceChoices.getByLabel("Yandere", { exact: true }),
  ).not.toBeChecked();
  await expect(page.getByLabel("条件值 1", { exact: true })).toHaveValue(
    JSON.stringify(tag),
  );
  checks.push(
    "legacy single-source query drafts migrate without losing source or literal tag conditions",
  );
  const singleDraft = await engine.api(draftPath);
  await engine.api(draftPath, "PUT", {
    schema_version: 1,
    expected_revision: singleDraft.draft.revision,
    value: {
      ...singleDraft.draft.value,
      definition: saved,
      sourceId: saved.spec.source_ids[0],
      conditions: saved.spec.conditions,
    },
  });
  await page.reload();
  await page.getByRole("tab", { name: "项目查询", exact: true }).click();
  await expect(
    sourceChoices.getByLabel("Yandere", { exact: true }),
  ).toBeChecked();
  await expect(
    sourceChoices.getByLabel("Gelbooru", { exact: true }),
  ).toBeChecked();
  checks.push(
    "legacy read-only multi-source definitions keep all source IDs when upgraded to editable drafts",
  );
  await page.getByRole("button", { name: "全部项目数据", exact: true }).click();
  await page.getByRole("tab", { name: "筛选", exact: true }).click();
  await filters
    .getByLabel("包含标签", { exact: true })
    .fill(JSON.stringify(tag));
  const allFiltering = page.waitForResponse(
    (r) =>
      r.request().method() === "POST" && r.url().endsWith("/query-results"),
  );
  await filters.getByRole("button", { name: "应用筛选", exact: true }).click();
  const allResult = await (await allFiltering).json();
  await engine.wait(
    `/v1/projects/${project.id}/query-results/${allResult.id}`,
    (r) => r.state === "ready",
  );
  await expect(page.locator(".asset-grid article")).toHaveCount(2);
  await page.getByRole("menuitem", { name: "项目", exact: true }).click();
  await page
    .getByRole("menuitem", { name: "添加数据湖…", exact: true })
    .click();
  await dialog.getByLabel("数据源类型", { exact: true }).selectOption("demo");
  await dialog.getByRole("button", { name: "检查数据湖", exact: true }).click();
  await expect(dialog.getByRole("status")).toContainText("内置参考资料");
  await dialog.getByRole("button", { name: "加入项目", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(filters).toContainText("项目的数据湖范围已变化");
  await expect(page.locator(".asset-grid article")).toHaveCount(2);
  await expect(
    filters.getByRole("button", { name: "应用筛选", exact: true }),
  ).toBeDisabled();
  await page.screenshot({
    path: resolve(run, "source-change-requires-refilter.png"),
  });
  checks.push(
    "adding a lake warns that all-project filter membership is frozen and blocks unsupported cross-source tag conditions",
  );
  assert.deepEqual(errors, []);
  checks.push(
    "site-aware card and metadata labels",
    "stored dimensions and lossless tag display",
    "metadata tag action populates structured query without whitespace changes",
    "query request and rendered result",
  );
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify({ status: "passed", checks }, null, 2),
  );
  console.log(
    JSON.stringify({
      status: "passed",
      report: resolve(run, "report.json"),
      checks: checks.length,
    }),
  );
} catch (error) {
  if (page) {
    await page
      .screenshot({ path: resolve(run, "failure.png") })
      .catch(() => {});
    await writeFile(
      resolve(run, "failure-dom.txt"),
      await page
        .locator("body")
        .innerText()
        .catch(() => ""),
    );
  }
  throw error;
} finally {
  if (offlineFile) await rename(offlineFile.offline, offlineFile.online);
  await browser?.close();
  if (vite && vite.exitCode === null) {
    const exited = new Promise((done) => vite.once("exit", done));
    vite.kill();
    await exited;
  }
  await engine.stop();
}
