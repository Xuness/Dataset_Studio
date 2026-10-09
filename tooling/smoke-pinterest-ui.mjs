// Real UI/SDK/engine, synthetic archive and an exhausted-budget job; no source requests.
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
import { browserOptions } from "./platform.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(root, ".local/test-runs", `pinterest-ui-${Date.now()}`);
await mkdir(run, { recursive: true });
const engine = new EngineFixture(root, resolve(run, "engine"));
const python = lakeWorkerPython(root),
  url = "http://127.0.0.1:1463";
const errors = [],
  checks = [],
  submissions = [];
let browser,
  vite,
  page,
  dropSubmission = true;
try {
  await promisify(execFile)(
    python,
    [resolve(root, "tooling/pinterest-fixture.py"), run],
    { windowsHide: true },
  );
  const refs = JSON.parse(
    await readFile(resolve(run, "pinterest.json"), "utf8"),
  );
  await engine.start();
  const { StudioClient } = await clientFixture(root, resolve(run, "client"));
  const client = new StudioClient(engine.connection);
  await client.lakeUpdates.configure({
    python,
    state_root: resolve(run, "controller"),
  });
  const project = await client.createProject({
    name: "Pinterest 界面验证",
    parent_directory: resolve(run, "projects"),
  });
  await client.sourceAccess.attach(project.id, {
    kind: "auto",
    name: "Pinterest 样本",
    media_root: refs.lake.media_root,
    index_root: refs.lake.index_root,
  });
  const log = await open(resolve(run, "vite.log"), "a");
  vite = spawn(
    process.execPath,
    [
      resolve(root, "apps/desktop/node_modules/vite/bin/vite.js"),
      "--port",
      "1463",
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
  const context = await browser.newContext({
    viewport: { width: 2560, height: 1440 },
    deviceScaleFactor: 1,
  });
  await context.route(url + "/__studio/connection", (route) =>
    route.fulfill({ json: engine.connection }),
  );
  await context.route(engine.connection.endpoint + "/**", async (route) => {
    const request = route.request(),
      headers = { ...request.headers() };
    if (request.url().includes("/events?")) return route.abort();
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
    const create =
      request.method() === "POST" &&
      request.url().endsWith("/v1/pinterest-collections/jobs");
    try {
      if (create) {
        const body = request.postDataJSON();
        // The browser must only coalesce this pre-exhausted job, never start a new network task.
        assert.deepEqual(body.definition, refs.budget_job.definition);
        submissions.push(body);
      }
      const response = await fetch(request.url(), {
        method: request.method(),
        headers,
        ...(request.postData() ? { body: request.postData() } : {}),
      });
      const body = Buffer.from(await response.arrayBuffer());
      if (create && dropSubmission) {
        dropSubmission = false;
        return route.abort("failed");
      }
      await route.fulfill({
        status: response.status,
        headers: { ...Object.fromEntries(response.headers), ...cors },
        body,
      });
    } catch (e) {
      if (create) errors.push(String(e));
      await route
        .fulfill({
          status: 503,
          headers: cors,
          json: { code: "FIXTURE_ERROR", message: "Isolated UI bridge failed" },
        })
        .catch(() => {});
    }
  });
  page = await context.newPage();
  page.setDefaultTimeout(30000);
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(url);
  await page.locator(".recent-row").filter({ hasText: project.name }).click();
  await page
    .locator(".source-tree-row button.tree-row")
    .filter({ hasText: "Pinterest 样本" })
    .click();
  await expect(page.locator(".asset-grid article")).toHaveCount(1);
  await expect(page.locator(".asset-grid article")).toContainText(
    "Pinterest #",
  );
  await expect(page.getByLabel("浏览排序", { exact: true })).toHaveValue(
    "asset_key_asc",
  );
  await expect(
    page.getByLabel("浏览排序").locator('option[value="post_id_desc"]'),
  ).toHaveCount(0);
  await page.locator(".asset-grid .asset-thumb").first().click();
  await page.getByRole("tab", { name: "检查器", exact: true }).click();
  await page.getByRole("tab", { name: "属性", exact: true }).click();
  await expect(
    page.getByRole("region", { name: "Pinterest Pin 资料" }),
  ).toBeVisible();
  const observation = page.getByLabel("历史观察", { exact: true });
  await expect(
    observation.locator("option").filter({ hasText: "Pin 详情" }),
  ).toHaveCount(1);
  await observation.selectOption({
    label: await observation
      .locator("option")
      .filter({ hasText: "Pin 详情" })
      .textContent(),
  });
  await expect(page.getByRole("region", { name: "元数据检查" })).toContainText(
    "保存账号样本",
  );
  await expect(page.getByRole("region", { name: "元数据检查" })).toContainText(
    "来源明确返回空值",
  );
  await page
    .locator("summary")
    .filter({ hasText: "分类标签与站点字段" })
    .click();
  await expect(
    page.getByText("保存账号样本", { exact: false }).first(),
  ).toBeVisible();
  await expect(page.locator(".property-preview img")).toBeVisible();
  await page.screenshot({ path: resolve(run, "pinterest-browser.png") });
  checks.push(
    "2560x1440 native source browser, supported sort, Pin/board/account metadata and missing-versus-null display",
  );

  await page.getByRole("button", { name: /^数据湖 ·/ }).click();
  await expect(page.getByRole("region", { name: "后台活动" })).toContainText(
    "Pinterest",
  );
  await page
    .getByRole("button", { name: "打开数据湖工作台", exact: true })
    .click();
  await page
    .getByRole("button", { name: "创建 / 登记数据湖", exact: true })
    .click();
  let dialog = page.getByRole("dialog", {
    name: "创建或登记数据湖",
    exact: true,
  });
  await dialog.getByLabel("站点", { exact: true }).selectOption("pinterest");
  await dialog
    .getByLabel("在线库目录", { exact: true })
    .fill(resolve(run, "empty-index"));
  await dialog
    .getByLabel("图片归档目录", { exact: true })
    .fill(resolve(run, "empty-media"));
  await dialog
    .getByRole("button", { name: "创建 Pinterest 数据湖", exact: true })
    .click();
  await expect(dialog).toHaveCount(0);
  assert.equal(
    (await client.sourceAccess.list(project.id)).items.filter(
      (s) => s.kind === "pinterest",
    ).length,
    2,
  );
  await page.getByRole("button", { name: "新建更新", exact: true }).click();
  dialog = page.getByRole("dialog", { name: "新建数据湖更新", exact: true });
  await expect(dialog.getByLabel("更新来源")).toHaveValue("pinterest");
  await dialog
    .getByLabel("Pinterest 数据湖", { exact: true })
    .selectOption(refs.lake.library_id);
  await dialog.getByLabel("Pin ID 或链接", { exact: true }).fill("3001\n3002");
  await dialog.getByLabel("最多详情请求", { exact: true }).fill("1");
  await dialog
    .getByRole("button", { name: "检查任务摘要", exact: true })
    .click();
  await expect(
    dialog.getByRole("region", { name: "Pinterest 任务摘要" }),
  ).toContainText("2 个指定 Pin");
  await page.screenshot({ path: resolve(run, "pinterest-composer.png") });
  await dialog.getByRole("button", { name: "开始采集", exact: true }).click();
  await expect(
    dialog.getByRole("button", { name: "重试同一次提交", exact: true }),
  ).toBeEnabled();
  await dialog
    .getByRole("button", { name: "重试同一次提交", exact: true })
    .click();
  await expect(dialog.getByRole("status")).toContainText("采集任务已创建");
  assert.equal(submissions.length, 2);
  assert.equal(submissions[0].request_key, submissions[1].request_key);
  assert.equal((await client.pinterestCollections.jobs()).items.length, 2);
  await dialog.getByRole("button", { name: "关闭", exact: true }).click();
  await expect(page.locator(".lake-details")).toContainText("本轮预算用完");
  await expect(page.locator(".lake-details")).toContainText("Pin 与媒体明细");
  await expect(page.locator(".lake-details .collection-task")).toHaveCount(3);
  await page.screenshot({ path: resolve(run, "pinterest-job.png") });
  await page
    .locator(".lake-details")
    .getByRole("button", { name: "暂停", exact: true })
    .click();
  await expect(page.locator(".lake-details")).toContainText("已暂停");
  await page
    .locator(".lake-details")
    .getByRole("button", { name: "取消后续工作", exact: true })
    .click();
  await expect(page.locator(".lake-details")).toContainText("已取消");
  assert.equal(
    (await client.pinterestCollections.job(refs.budget_job.id)).api_requests,
    1,
  );
  checks.push(
    "UI creates/attaches an empty lake, previews specified Pins, reuses the submission key after lost response, and pauses/cancels via formal APIs",
  );
  assert.deepEqual(errors, []);
  await writeFile(
    resolve(run, "summary.json"),
    JSON.stringify({ ok: true, run, checks, errors }, null, 2),
  );
  console.log(JSON.stringify({ ok: true, run, checks }));
} catch (e) {
  await page?.screenshot({ path: resolve(run, "failure.png") }).catch(() => {});
  await writeFile(
    resolve(run, "summary.json"),
    JSON.stringify(
      { ok: false, run, checks, errors, error: String(e.stack ?? e) },
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
