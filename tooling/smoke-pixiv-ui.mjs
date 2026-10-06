// Real UI + SDK + owned engine, synthetic archive only. No remote Pixiv calls.
import assert from "node:assert/strict";
import { execFile, spawn } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, open, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";
import { EngineFixture, sleep } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";
import { lakeWorkerPython } from "./lake-worker-runtime.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(root, ".local/test-runs", "pixiv-ui-" + Date.now());
await mkdir(run, { recursive: true });
const engine = new EngineFixture(root, resolve(run, "engine"));
const checks = [],
  errors = [];
let browser, vite, page;
const url = "http://127.0.0.1:1457";
try {
  await promisify(execFile)(
    lakeWorkerPython(root),
    [resolve(root, "tooling/collections-fixture.py"), run],
    { windowsHide: true },
  );
  const refs = JSON.parse(
    await readFile(resolve(run, "collections.json"), "utf8"),
  );
  await promisify(execFile)(
    lakeWorkerPython(root),
    [resolve(root, "tooling/multibooru-fixture.py"), resolve(run, "booru")],
    { windowsHide: true },
  );
  const booru = JSON.parse(
    await readFile(resolve(run, "booru/multibooru.json"), "utf8"),
  );
  await engine.start();
  const { StudioClient } = await clientFixture(root, resolve(run, "client"));
  const client = new StudioClient(engine.connection);
  await client.lakeUpdates.configure({
    python: lakeWorkerPython(root),
    state_root: resolve(run, "control"),
  });
  const account = await client.sourceCollections.saveAccount({
    request_key: crypto.randomUUID(),
    expected_revision: null,
    account_id: crypto.randomUUID(),
    mode: "session",
    label: "UI 离线待登录",
    cookies: [
      {
        name: "PHPSESSID",
        value: "1000_offline_ui_fixture",
        domain: ".pixiv.net",
        path: "/",
        secure: true,
        http_only: true,
        expires_unix: null,
      },
    ],
  });
  const project = await client.createProject({
    name: "Pixiv 界面验收",
    parent_directory: resolve(run, "projects"),
  });
  await client.sourceAccess.attach(project.id, {
    kind: "auto",
    name: "Pixiv fixture",
    media_root: refs.lake.media_root,
    index_root: refs.lake.index_root,
  });
  for (const [kind, lake] of Object.entries(booru))
    await client.sourceAccess.attach(project.id, {
      kind,
      name: kind + " fixture",
      media_root: lake.lake,
      index_root: lake.lake,
    });
  // Reproduce a pre-fix project whose shared preference was overwritten by Pixiv.
  await client.drafts.save(project.id, "studio.session", "default", {
    expected_revision: 0,
    schema_version: 1,
    value: {
      order: "asset_key_asc",
      moduleId: "core.browser",
      panels: [],
      scope: { kind: "all" },
      focusKey: null,
      view: "grid",
      position: null,
    },
  });
  const log = await open(resolve(run, "vite.log"), "a");
  vite = spawn(
    process.execPath,
    [
      resolve(root, "apps/desktop/node_modules/vite/bin/vite.js"),
      "--port",
      "1457",
      "--strictPort",
    ],
    {
      cwd: resolve(root, "apps/desktop"),
      env: { ...process.env, STUDIO_DATA_DIR: engine.dataDir },
      windowsHide: true,
      stdio: ["ignore", log.fd, log.fd],
    },
  );
  await log.close();
  for (let n = 0; n < 100; n++) {
    if ((await fetch(url).catch(() => null))?.ok) break;
    if (vite.exitCode !== null) throw new Error("Vite exited");
    await sleep(100);
  }
  browser = await chromium.launch({ channel: "msedge", headless: true });
  const context = await browser.newContext({
    viewport: { width: 1920, height: 1200 },
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
    } catch (e) {
      if (!page?.isClosed()) throw e;
    }
  });
  page = await context.newPage();
  page.setDefaultTimeout(30000);
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(url);
  await page.locator(".recent-row").filter({ hasText: project.name }).click();
  await page
    .locator(".source-tree-row button.tree-row")
    .filter({ hasText: "Pixiv fixture" })
    .click();
  await expect(page.locator(".asset-grid article").first()).toBeVisible();
  await expect(page.getByLabel("浏览排序", { exact: true })).toHaveValue(
    "asset_key_asc",
  );
  const browseOrder = page.getByLabel("浏览排序", { exact: true });
  const openSource = async (name, order) => {
    await page
      .locator(".source-tree-row button.tree-row")
      .filter({ hasText: name })
      .click();
    await expect(browseOrder).toHaveValue(order);
    await expect(page.locator(".asset-grid article").first()).toBeVisible();
    await expect(async () =>
      assert.equal(
        (await client.drafts.get(project.id, "studio.session")).draft.value
          .scope.name,
        name,
      ),
    ).toPass();
  };
  const persistedOrder = async (field, value) => {
    await expect(async () =>
      assert.equal(
        (await client.drafts.get(project.id, "studio.session")).draft.value[
          field
        ],
        value,
      ),
    ).toPass();
  };
  await expect(browseOrder.locator('option[value="post_id_desc"]')).toHaveCount(
    0,
  );
  for (const kind of Object.keys(booru))
    await openSource(kind + " fixture", "post_id_desc");
  await persistedOrder("booruOrder", "post_id_desc");
  await browseOrder.selectOption("post_id_asc");
  await persistedOrder("booruOrder", "post_id_asc");
  await openSource("Pixiv fixture", "asset_key_asc");
  await browseOrder.selectOption("asset_key_desc");
  await persistedOrder("order", "asset_key_desc");
  await persistedOrder("booruOrder", "post_id_asc");
  await page.reload();
  await expect(browseOrder).toHaveValue("asset_key_desc");
  for (const kind of Object.keys(booru))
    await openSource(kind + " fixture", "post_id_asc");
  await page
    .getByRole("tabpanel", { name: "项目", exact: true })
    .getByRole("button", { name: "全部项目数据", exact: true })
    .click();
  await expect(browseOrder).toHaveValue("asset_key_desc");
  await expect(browseOrder.locator('option[value="post_id_desc"]')).toHaveCount(
    0,
  );
  await openSource("danbooru fixture", "post_id_asc");
  await browseOrder.selectOption("asset_key_asc");
  await persistedOrder("booruOrder", "asset_key_asc");
  await openSource("Pixiv fixture", "asset_key_desc");
  await page.reload();
  await expect(browseOrder).toHaveValue("asset_key_desc");
  await openSource("danbooru fixture", "asset_key_asc");
  await browseOrder.selectOption("post_id_desc");
  await persistedOrder("booruOrder", "post_id_desc");
  await openSource("Pixiv fixture", "asset_key_desc");
  await browseOrder.selectOption("asset_key_asc");
  await persistedOrder("order", "asset_key_asc");
  await persistedOrder("booruOrder", "post_id_desc");
  checks.push(
    "legacy shared identity order recovers Booru ID default; all three Booru lakes retain manual ID/identity choices across Pixiv, mixed browsing and reload",
  );
  await page
    .getByRole("button", { name: "Rating / Tag 筛选", exact: true })
    .click();
  await page
    .getByLabel("包含标签", { exact: true })
    .fill("missing_fixture_tag");
  const applyQuickFilter = page.getByRole("button", {
    name: "应用筛选",
    exact: true,
  });
  await expect(applyQuickFilter).toBeEnabled();
  const [quickFilterResponse] = await Promise.all([
    page.waitForResponse(
      (response) =>
        /\/query-(views|results)$/.test(response.url()) &&
        response.request().method() === "POST",
    ),
    applyQuickFilter.click(),
  ]);
  const quickFilter = await quickFilterResponse.json();
  assert.equal(quickFilter.spec.order, "asset_key_asc");
  assert.deepEqual(quickFilter.spec.conditions[0].value.value, [
    "missing_fixture_tag",
  ]);
  await engine.wait(
    `/v1/projects/${project.id}/query-results/${quickFilter.id}`,
    (result) => result.state === "ready",
  );
  assert.equal(
    (await client.queries.assets(project.id, quickFilter.id, { limit: 10 }))
      .page.items.length,
    0,
  );
  await expect(page.locator(".asset-grid article")).toHaveCount(0);
  await page.getByRole("button", { name: "清除筛选", exact: true }).click();
  await expect(browseOrder).toHaveValue("asset_key_asc");
  await expect(page.locator(".asset-grid article").first()).toBeVisible();
  await persistedOrder("booruOrder", "post_id_desc");
  checks.push(
    "Pixiv quick filters use the effective identity order without changing Booru preference",
  );
  await page.locator(".asset-grid .asset-thumb").first().click();
  await page.getByRole("tab", { name: "检查器", exact: true }).click();
  await page.getByRole("tab", { name: "属性", exact: true }).click();
  await expect(
    page.getByRole("region", { name: "Pixiv 作品资料", exact: true }),
  ).toContainText("紺屋 fixture");
  await page.getByRole("button", { name: "筛选同作者", exact: true }).click();
  await expect(page.getByLabel("条件字段 1", { exact: true })).toHaveValue(
    "author.id",
  );
  await expect(page.getByLabel("条件值 1", { exact: true })).toHaveValue(
    "10109777",
  );
  await page.getByLabel("条件字段 1", { exact: true }).selectOption("tags");
  await page.getByLabel("条件值 1", { exact: true }).fill("red hair");
  const queryResponse = page.waitForResponse(
    (r) =>
      /\/query-(views|results)$/.test(r.url()) &&
      r.request().method() === "POST",
  );
  await page.getByRole("button", { name: "执行查询", exact: true }).click();
  const query = await (await queryResponse).json();
  const result = await engine.wait(
    `/v1/projects/${project.id}/query-results/${query.id}`,
    (r) => !["queued", "running"].includes(r.state),
  );
  assert.equal(result.state, "ready");
  assert.equal(result.spec.conditions[0].value.value, "red hair");
  assert.equal(
    (await client.queries.assets(project.id, result.id, { limit: 10 })).page
      .items.length,
    1,
  );
  await expect(
    page.getByRole("button", { name: "执行查询", exact: true }),
  ).toBeEnabled();
  await expect(page.locator(".asset-grid article")).toHaveCount(1);
  await page.screenshot({ path: resolve(run, "pixiv-browser.png") });
  checks.push(
    "existing browser chooses supported sort; page/author metadata and exact author/literal-space tag filters work",
  );

  await page.getByRole("button", { name: /^数据湖 ·/ }).click();
  await page
    .getByRole("button", { name: "打开数据湖工作台", exact: true })
    .click();
  await expect(page.locator(".lake-outline")).toContainText("Pixiv");
  await expect(page.locator(".lake-table")).toContainText("公开范围完成");
  await page
    .getByRole("button", { name: "创建 / 登记数据湖", exact: true })
    .click();
  let dialog = page.getByRole("dialog", { name: "创建或登记数据湖" });
  await dialog.getByLabel("站点", { exact: true }).selectOption("pixiv");
  await dialog
    .getByLabel("在线库目录", { exact: true })
    .fill(resolve(run, "created-online"));
  await dialog
    .getByLabel("图片归档目录", { exact: true })
    .fill(resolve(run, "created-media"));
  await dialog
    .getByRole("button", { name: "创建 Pixiv 数据湖", exact: true })
    .click();
  await expect(dialog).toHaveCount(0);
  await expect(async () =>
    assert.equal((await client.sourceCollections.lakes()).items.length, 2),
  ).toPass();
  await page.getByRole("button", { name: "新建更新", exact: true }).click();
  dialog = page.getByRole("dialog", { name: "新建数据湖更新" });
  await expect(dialog.getByLabel("更新来源")).toHaveValue("collection");
  await expect(dialog.getByLabel("访问方式", { exact: true })).toHaveValue("");
  await dialog
    .getByLabel("Pixiv 种子")
    .fill("https://www.pixiv.net/users/10109777");
  await dialog
    .getByRole("button", { name: "检查任务摘要", exact: true })
    .click();
  await expect(
    dialog.getByRole("region", { name: "Pixiv 任务摘要" }),
  ).toContainText("10109777");
  await page.screenshot({ path: resolve(run, "pixiv-composer.png") });
  // Admission with an unverified synthetic account proves UI control without sending any requests to Pixiv.
  await dialog.getByLabel("访问方式", { exact: true }).selectOption(account.id);
  await dialog
    .getByRole("button", { name: "检查任务摘要", exact: true })
    .click();
  await dialog.getByRole("button", { name: "开始采集", exact: true }).click();
  await expect(dialog.getByRole("status")).toContainText("采集任务已创建");
  await dialog.getByRole("button", { name: "关闭", exact: true }).click();
  await expect(page.locator(".lake-details")).toContainText("等待登录凭据");
  checks.push(
    "lake creation and author collection use the existing update dialog; public mode needs no credentials; login state remains explicit",
  );
  await page.screenshot({ path: resolve(run, "pixiv-workspace.png") });
  await page.getByRole("button", { name: "新建复查", exact: true }).click();
  dialog = page.getByRole("dialog", { name: "新建数据湖更新" });
  await dialog.locator("summary").filter({ hasText: "本轮预算与周期" }).click();
  await dialog.getByLabel("按固定周期复查", { exact: true }).check();
  await dialog
    .getByRole("button", { name: "检查任务摘要", exact: true })
    .click();
  await dialog
    .getByRole("button", { name: "创建并启用计划", exact: true })
    .click();
  await expect(dialog.getByRole("status")).toContainText("周期计划已启用");
  await dialog.getByRole("button", { name: "关闭", exact: true }).click();
  await expect(page.getByLabel("数据湖工作视图")).toHaveValue("schedules");
  await expect(page.locator(".lake-table")).toContainText("已启用");
  await expect(page.locator(".lake-details")).toContainText("Pixiv 周期复查");
  await page.getByLabel("启用计划", { exact: true }).uncheck();
  await page.getByRole("button", { name: "保存计划", exact: true }).click();
  await expect(async () =>
    assert.equal(
      (await client.sourceCollections.schedules()).items[0].enabled,
      false,
    ),
  ).toPass();
  checks.push(
    "periodic snapshots share the schedule list and can be disabled with revision checks",
  );

  await page.getByRole("button", { name: "API 设置", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "设置", exact: true });
  const card = settings.locator(".collection-api-settings");
  await card
    .getByLabel("Pixiv PHPSESSID")
    .fill("12345_synthetic_ui_credential");
  await card
    .getByRole("button", { name: "保存 Pixiv 凭据", exact: true })
    .click();
  await expect(card.getByRole("status")).toContainText("凭据已加密保存");
  await expect(card.getByLabel("Pixiv PHPSESSID")).toHaveValue("");
  const publicAccounts = await client.sourceCollections.accounts();
  assert.ok(
    !JSON.stringify(publicAccounts).includes("synthetic_ui_credential"),
  );
  await card
    .getByRole("button", { name: "清除 Pixiv 凭据", exact: true })
    .click();
  await expect(card.getByRole("status")).toContainText("已清除该会话");
  await expect(
    settings
      .getByLabel("迁移数据湖")
      .locator("option")
      .filter({ hasText: "pixiv" }),
  ).toHaveCount(2);
  await page.screenshot({ path: resolve(run, "pixiv-settings.png") });
  await settings
    .getByRole("button", { name: "关闭", exact: true })
    .last()
    .click();
  checks.push(
    "Pixiv credentials stay in API settings, encrypted and never read back; Pixiv is included in relocation settings",
  );
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.screenshot({ path: resolve(run, "pixiv-workspace-1440.png") });
  assert.deepEqual(errors, []);
  assert.equal((await client.lakeUpdates.lakes()).items.length, 0);
  checks.push("no browser runtime errors or legacy site-enum leakage");
  await writeFile(
    resolve(run, "summary.json"),
    JSON.stringify({ ok: true, checks, run }, null, 2),
  );
  console.log(JSON.stringify({ ok: true, checks, run }));
} catch (e) {
  if (page && !page.isClosed())
    await page
      .screenshot({ path: resolve(run, "failure.png") })
      .catch(() => {});
  await writeFile(
    resolve(run, "summary.json"),
    JSON.stringify(
      { ok: false, checks, error: String(e.stack ?? e), errors, run },
      null,
      2,
    ),
  );
  console.error(e);
  process.exitCode = 1;
} finally {
  await browser?.close();
  vite?.kill();
  await engine.stop();
}
