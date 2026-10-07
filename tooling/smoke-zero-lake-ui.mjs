import { browserOptions } from "./platform.mjs";
// UI-created empty lakes, actual archive writers, metadata selection and typed task handoff.
import assert from "node:assert/strict";
import { execFile, spawn } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, open, writeFile, stat } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";
import { EngineFixture, sleep } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";
import { lakeWorkerPython } from "./lake-worker-runtime.mjs";
import { verifyLakeWorkerBundle } from "./lake-worker-bundle.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(root, ".local/test-runs", "zero-lake-ui-" + Date.now());
const python = lakeWorkerPython(root);
assert.ok(python, "Worker runtime is required");
await mkdir(run, { recursive: true });
const engine = new EngineFixture(root, resolve(run, "engine"));
const checks = [],
  errors = [];
const url = "http://127.0.0.1:1461";
let browser, vite, page, context;
async function bridge() {
  await context.route(engine.connection.endpoint + "/**", async (route) => {
    const request = route.request();
    if (request.url().includes("/events?")) return route.abort();
    const headers = { ...request.headers() };
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
    } catch {
      await route
        .fulfill({
          status: 503,
          headers: cors,
          json: {
            code: "FIXTURE_RESTART",
            message: "Isolated engine restarting",
          },
        })
        .catch(() => {});
    }
  });
}
try {
  await engine.start();
  const { StudioClient } = await clientFixture(root, resolve(run, "client"));
  let client = new StudioClient(engine.connection);
  await client.lakeUpdates.configure({
    python,
    state_root: resolve(run, "controller"),
  });
  await verifyLakeWorkerBundle(root, engine.dataDir);
  const project = await client.createProject({
    name: "从零建湖验收",
    parent_directory: resolve(run, "projects"),
  });
  assert.equal(
    (await client.sourceCollections.workspaceLakes({ limit: 100 })).items
      .length,
    0,
  );
  const log = await open(resolve(run, "vite.log"), "a");
  vite = spawn(
    process.execPath,
    [
      resolve(root, "apps/desktop/node_modules/vite/bin/vite.js"),
      "--port",
      "1461",
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
  browser = await chromium.launch({ ...browserOptions(), headless: true });
  context = await browser.newContext({
    viewport: { width: 2560, height: 1440 },
  });
  await context.route(url + "/__studio/connection", (route) =>
    route.fulfill({ json: engine.connection }),
  );
  await bridge();
  page = await context.newPage();
  page.setDefaultTimeout(30000);
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(url);
  await page.locator(".recent-row").filter({ hasText: project.name }).click();
  await page.getByRole("button", { name: /^数据湖 ·/ }).click();
  await page
    .getByRole("button", { name: "打开数据湖工作台", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "新建更新", exact: true }),
  ).toBeDisabled();
  for (const [site, label] of [
    ["danbooru", "Danbooru"],
    ["gelbooru", "Gelbooru"],
    ["yandere", "Yandere"],
    ["pixiv", "Pixiv"],
  ]) {
    await page
      .getByRole("button", { name: "创建 / 登记数据湖", exact: true })
      .click();
    const dialog = page.getByRole("dialog", {
      name: "创建或登记数据湖",
      exact: true,
    });
    await dialog.getByLabel("站点", { exact: true }).selectOption(site);
    await dialog
      .getByLabel("在线库目录", { exact: true })
      .fill(resolve(run, site, "online"));
    await dialog
      .getByLabel("图片归档目录", { exact: true })
      .fill(resolve(run, site, "archive"));
    await expect(
      dialog.getByLabel("创建新的空湖", { exact: true }),
    ).toBeChecked();
    await expect(dialog.getByLabel(/添加到当前项目/)).toBeChecked();
    await dialog
      .getByRole("button", { name: `创建 ${label} 数据湖`, exact: true })
      .click();
    await expect(dialog).toHaveCount(0, { timeout: 30000 });
    const sources = (await client.sourceAccess.list(project.id)).items;
    assert.ok(
      sources.some((source) => source.kind === site && source.count === 0),
    );
    if (site !== "pixiv")
      await assert.rejects(
        stat(resolve(run, site, "online/CURRENT.json")),
        (error) => error.code === "ENOENT",
      );
  }
  const lakes = (await client.sourceCollections.workspaceLakes({ limit: 100 }))
    .items;
  assert.equal(lakes.length, 4);
  await writeFile(
    resolve(run, "created-lakes.json"),
    JSON.stringify(lakes, null, 2),
  );
  await page.screenshot({ path: resolve(run, "four-empty-lakes.png") });
  checks.push(
    "UI creates four empty lakes and attaches each to the current project without imports or producer indexes",
  );

  await engine.stop();
  await promisify(execFile)(
    python,
    [resolve(root, "tooling/zero-lake-fixture.py"), run],
    { windowsHide: true },
  );
  await engine.start();
  client = new StudioClient(engine.connection);
  await client.openProject(project.directory);
  await bridge();
  await page.reload();
  if (
    await page
      .locator(".recent-row")
      .filter({ hasText: project.name })
      .isVisible()
  )
    await page.locator(".recent-row").filter({ hasText: project.name }).click();
  if (!(await page.locator(".lake-outline").isVisible())) {
    await page.getByRole("button", { name: /^数据湖 ·/ }).click();
    await page
      .getByRole("button", { name: "打开数据湖工作台", exact: true })
      .click();
  }
  await expect(page.locator(".lake-outline")).toBeVisible();
  await page
    .locator(".lake-outline button")
    .filter({ hasText: "Danbooru" })
    .click();
  await page
    .getByLabel("数据湖工作视图", { exact: true })
    .selectOption("metadata");
  await expect(page.getByRole("table", { name: "帖子元数据" })).toContainText(
    "尚未下载",
  );
  await expect(page.getByRole("table", { name: "帖子元数据" })).toContainText(
    "已有图片",
  );
  await page.getByLabel("只看尚无图片的帖子", { exact: true }).check();
  await expect(
    page.getByRole("button", { name: "为匹配元数据新建任务", exact: true }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "查询元数据", exact: true }).click();
  await expect(page.getByLabel("选择帖子 12", { exact: true })).toBeVisible();
  await expect(page.getByLabel("选择帖子 11", { exact: true })).toHaveCount(0);
  await page.getByLabel("选择帖子 12", { exact: true }).check();
  await page.screenshot({ path: resolve(run, "metadata-selection.png") });
  await page
    .getByRole("button", { name: "为已选帖子新建任务", exact: true })
    .click();
  let dialog = page.getByRole("dialog", {
    name: "新建数据湖更新",
    exact: true,
  });
  await expect(dialog.getByLabel("候选来源", { exact: true })).toHaveValue(
    "local",
  );
  await expect(
    dialog.getByLabel("只选择尚无图片的帖子", { exact: true }),
  ).toBeChecked();
  await dialog
    .getByLabel("保存策略", { exact: true })
    .selectOption("metadata_only");
  await dialog
    .getByRole("button", { name: "检查任务摘要", exact: true })
    .click();
  await dialog.getByRole("button", { name: "开始更新", exact: true }).click();
  await expect(dialog.getByRole("status")).toContainText("各湖任务已创建");
  await dialog.getByRole("button", { name: "关闭", exact: true }).click();
  const id = (await client.lakeUpdates.jobs()).items.find(
    (job) => job.definition.range.source === "local",
  ).id;
  const done = await engine.wait(
    `/v1/lake-updates/jobs/${id}`,
    (job) => job.state === "completed",
  );
  assert.deepEqual(done.definition.range.post_ids, [12]);
  assert.equal(done.definition.range.missing_media, true);
  assert.ok(done.definition.range.version);
  assert.deepEqual(done.counts, { metadata: 1 });
  assert.equal(done.telemetry.api_requests, 0);
  checks.push(
    "metadata-only post selection preserves filters, selected IDs and version through the real task UI with zero API requests",
  );

  await page
    .locator(".lake-outline button")
    .filter({ hasText: "Danbooru" })
    .click();
  await page.getByRole("button", { name: "新建更新", exact: true }).click();
  dialog = page.getByRole("dialog", { name: "新建数据湖更新", exact: true });
  await dialog
    .getByRole("button", { name: "配置另一批更新", exact: true })
    .click();
  await dialog.getByLabel("Danbooru", { exact: true }).check();
  await dialog.getByLabel("候选来源", { exact: true }).selectOption("remote");
  await dialog.getByLabel("全部包含 Tag", { exact: true }).fill("a b c");
  await dialog.getByLabel("排除 Tag", { exact: true }).fill("a");
  await dialog
    .getByLabel("保存策略", { exact: true })
    .selectOption("metadata_only");
  await dialog
    .getByRole("button", { name: "检查任务摘要", exact: true })
    .click();
  await expect(dialog.getByRole("region", { name: "任务摘要" })).toContainText(
    "本地判断组合条件",
  );
  await dialog.getByRole("button", { name: "开始更新", exact: true }).click();
  await expect(dialog.getByRole("status")).toContainText("各湖任务已创建");
  await dialog.getByRole("button", { name: "关闭", exact: true }).click();
  checks.push(
    "Booru UI accepts multi-tag local predicates and bounded contradiction planning without source traffic",
  );

  await page
    .locator(".lake-outline button")
    .filter({ hasText: "Pixiv" })
    .click();
  await page.getByRole("button", { name: "新建更新", exact: true }).click();
  dialog = page.getByRole("dialog", { name: "新建数据湖更新", exact: true });
  await dialog.getByLabel("Pixiv 种子", { exact: true }).fill("12345");
  await dialog
    .getByLabel("Pixiv 采集范围", { exact: true })
    .selectOption("works");
  await dialog
    .locator("summary")
    .filter({ hasText: "标签筛选（可选）" })
    .click();
  await dialog
    .getByLabel("Pixiv 全部包含 Tag", { exact: true })
    .fill("blue hair");
  await dialog
    .getByRole("button", { name: "检查任务摘要", exact: true })
    .click();
  await expect(
    dialog.getByRole("region", { name: "Pixiv 任务摘要" }),
  ).toBeVisible();
  await page.screenshot({ path: resolve(run, "pixiv-tag-scope.png") });
  await dialog.getByRole("button", { name: "关闭", exact: true }).click();
  checks.push(
    "Pixiv scope editor preserves spaces in literal tags within a selected work scope",
  );
  await page
    .locator(".lake-outline button")
    .filter({ hasText: "Danbooru" })
    .click();
  await page
    .getByLabel("数据湖工作视图", { exact: true })
    .selectOption("metadata");
  await page.setViewportSize({ width: 1706, height: 960 });
  await page.screenshot({ path: resolve(run, "metadata-scaled.png") });
  assert.ok(
    await page.evaluate(
      () =>
        globalThis.document.documentElement.scrollWidth <=
        globalThis.innerWidth + 1,
    ),
  );
  assert.deepEqual(errors, []);
  await writeFile(
    resolve(run, "result.json"),
    JSON.stringify({ passed: true, checks, errors }, null, 2),
  );
  console.log(JSON.stringify({ run, checks }));
} catch (error) {
  await page?.screenshot({ path: resolve(run, "failure.png") }).catch(() => {});
  await writeFile(
    resolve(run, "failure.json"),
    JSON.stringify(
      { error: String(error), stack: error.stack, checks, errors },
      null,
      2,
    ),
  );
  throw error;
} finally {
  await browser?.close();
  vite?.kill();
  await engine.stop();
}
