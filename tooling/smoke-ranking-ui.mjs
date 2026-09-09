// Real React UI against an isolated fixture engine. No user browser profile is used.
import assert from "node:assert/strict";
import { spawn, execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, readFile, writeFile, open } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";
import { EngineFixture, sleep } from "./engine-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local",
  "test-runs",
  "smoke-ranking-ui-" + Date.now(),
);
const state = resolve(run, "state");
const execute = promisify(execFile);
await mkdir(run, { recursive: true });
const fixtureRoot = process.argv[2]
  ? resolve(process.argv[2])
  : resolve(run, "fixture");
if (!process.argv[2])
  await execute(
    "python",
    [resolve(root, "tooling/ranking-fixture.py"), fixtureRoot, "1024"],
    { windowsHide: true },
  );
const fixture = JSON.parse(
  await readFile(resolve(fixtureRoot, "fixture.json"), "utf8"),
);
const engine = new EngineFixture(root, state);
const checks = [],
  errors = [];
let browser, page, vite;
const url = "http://127.0.0.1:1428";
try {
  await engine.start();
  const project = await engine.api("/v1/projects", "POST", {
    name: "元数据排名界面验收",
  });
  const base = "/v1/projects/" + project.id;
  const source = await engine.api(base + "/sources", "POST", {
    name: "排名参考资料",
    kind: "danbooru",
    index_root: fixture.lake,
    media_root: fixture.lake,
  });
  await engine.api(base + "/close", "POST");
  const log = await open(resolve(run, "vite.log"), "a");
  vite = spawn(
    process.execPath,
    [
      resolve(root, "apps/desktop/node_modules/vite/bin/vite.js"),
      "--port",
      "1428",
    ],
    {
      cwd: resolve(root, "apps/desktop"),
      env: { ...process.env, STUDIO_DATA_DIR: state },
      windowsHide: true,
      stdio: ["ignore", log.fd, log.fd],
    },
  );
  await log.close();
  for (let i = 0; i < 100; i++) {
    if (
      (await fetch(url, { signal: AbortSignal.timeout(500) }).catch(() => null))
        ?.ok
    )
      break;
    if (vite.exitCode !== null) throw new Error("Fixture Vite exited");
    await sleep(100);
  }
  browser = await chromium.launch({ channel: "msedge", headless: true });
  const context = await browser.newContext({
    viewport: { width: 1540, height: 1000 },
  });
  // Route only this test context's connection; the product's origin allowlist stays intact.
  await context.route(url + "/__studio/connection", (route) =>
    route.fulfill({ json: engine.connection }),
  );
  await context.route(engine.connection.endpoint + "/**", async (route) => {
    const request = route.request();
    const path = request.url().slice(engine.connection.endpoint.length);
    const headers = { ...request.headers() };
    delete headers.host;
    delete headers.origin;
    const cors = {
      "access-control-allow-origin": url,
      "access-control-allow-methods": "GET,POST,PUT,PATCH,DELETE,OPTIONS",
      "access-control-allow-headers":
        request.headers()["access-control-request-headers"] ??
        "authorization,content-type,x-studio-session,x-studio-read-id",
    };
    if (request.method() === "OPTIONS")
      return route.fulfill({ status: 204, headers: cors });
    const response = await fetch(engine.connection.endpoint + path, {
      method: request.method(),
      headers,
      ...(request.postData() ? { body: request.postData() } : {}),
    });
    await route.fulfill({
      status: response.status,
      headers: { ...Object.fromEntries(response.headers), ...cors },
      body: Buffer.from(await response.arrayBuffer()),
    });
  });
  page = await context.newPage();
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(url);
  await page.locator(".recent-row").filter({ hasText: project.name }).click();
  await page.getByRole("button", { name: "计算工具", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: /Danbooru 元数据排名/ }),
  ).toBeVisible();
  await page.getByLabel("输入范围", { exact: true }).selectOption(source.id);
  await page
    .getByRole("radio", { name: "按名额生成候选集", exact: true })
    .check();
  await expect(page.getByLabel("主通道（%）")).toHaveValue("28");
  await expect(
    page.getByRole("button", { name: "生成候选集", exact: true }),
  ).toBeEnabled();
  await page.screenshot({ path: resolve(run, "01-config-wide.png") });
  await page.getByRole("button", { name: "生成候选集", exact: true }).click();
  await expect(page.getByText("本次排名已完成", { exact: true })).toBeVisible({
    timeout: 90000,
  });
  await expect(page.locator(".tasks-panel")).toHaveCount(0);
  await page.getByRole("button", { name: "结果榜单", exact: true }).click();
  await expect(
    page.getByRole("table", { name: "元数据排名榜单" }).locator("tbody tr"),
  ).toHaveCount(48);
  await page.getByLabel("分级", { exact: true }).selectOption("g");
  await expect(page.locator(".ranking-result-description")).toContainText(
    "匹配",
  );
  await page.getByRole("button", { name: "下一批", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "上一批", exact: true }),
  ).toBeEnabled();
  await page.getByRole("button", { name: "首批", exact: true }).click();
  await expect(
    page.getByRole("table", { name: "元数据排名榜单" }).locator("tbody tr"),
  ).toHaveCount(48);
  await page.screenshot({ path: resolve(run, "02-results-wide.png") });
  checks.push(
    "actual UI config submits a ranking job and shows bounded result pages with rating filters",
  );
  await page
    .getByRole("button", { name: "评分依据", exact: true })
    .first()
    .click();
  await expect(
    page.getByRole("complementary", { name: "评分依据" }),
  ).toBeVisible();
  await expect(
    page.locator(".ranking-detail-image img, img.ranking-detail-image"),
  ).toBeVisible({ timeout: 30000 });
  await page.screenshot({ path: resolve(run, "03-evidence-wide.png") });
  await page.getByRole("button", { name: "关闭评分依据" }).click();
  await page.getByRole("button", { name: "统计诊断", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "输入资格", exact: true }),
  ).toBeVisible();
  await page.screenshot({ path: resolve(run, "04-diagnostics-wide.png") });
  checks.push(
    "score evidence loads a real fixture preview and diagnostics show the frozen population",
  );
  await page.getByRole("button", { name: "结果榜单", exact: true }).click();
  await page.getByLabel("每个分级前 N 名").fill("75");
  await expect(page.locator(".ranking-result-description")).toContainText(
    "匹配 75 项",
  );
  await page
    .locator(".ranking-view")
    .getByRole("button", { name: "存为工作集", exact: true })
    .click();
  await page.getByLabel("工作集名称", { exact: true }).fill("界面导出的前75名");
  await page.getByRole("button", { name: "保存工作集", exact: true }).click();
  await expect(page.locator(".ranking-saved")).toContainText("75 项");
  const collections = (await engine.api(base + "/collections")).items;
  assert.equal(
    collections.find((c) => c.name === "界面导出的前75名").count,
    75,
  );
  checks.push(
    "save workset includes all 75 matches across unloaded pages and creates an ordinary project collection",
  );
  await page.setViewportSize({ width: 1000, height: 760 });
  await page.screenshot({ path: resolve(run, "05-results-narrow.png") });
  assert.ok(
    await page
      .locator("html")
      .evaluate((element) => element.scrollWidth <= element.clientWidth),
    "Narrow viewport must not overflow the document",
  );
  await page.getByRole("button", { name: "参数配置", exact: true }).click();
  await page.screenshot({ path: resolve(run, "06-config-narrow.png") });
  await page.getByRole("button", { name: "基础工具", exact: true }).click();
  await page
    .getByRole("button", { name: "Danbooru 元数据排名", exact: true })
    .click();
  await expect(
    page.getByRole("radio", { name: "按名额生成候选集", exact: true }),
  ).toBeChecked();
  await page.getByRole("button", { name: "结果榜单", exact: true }).click();
  await sleep(1100);
  await page.reload();
  await expect(page.getByRole("table", { name: "元数据排名榜单" })).toBeVisible(
    { timeout: 30000 },
  );
  await expect(page.getByLabel("每个分级前 N 名")).toHaveValue("75");
  checks.push(
    "module switching and page reconnection restore ranking draft, result selection and filters; both layouts stay within the viewport",
  );
  await page.setViewportSize({ width: 2560, height: 1440 });
  await expect(page.locator(".ranking-table tbody tr")).toHaveCount(48);
  await expect(page.locator(".ranking-loading")).toHaveCount(0);
  await page.getByTitle("属性面板", { exact: true }).click();
  await expect(page.locator(".ranking-inspector")).toHaveCount(0);
  await page.locator(".ranking-table tbody tr").nth(1).click();
  await expect(
    page.getByRole("complementary", { name: "评分依据" }),
  ).toBeVisible();
  const grip = page.getByRole("separator", {
    name: "排名属性面板宽度",
    exact: true,
  });
  await grip.press("ArrowLeft");
  await expect(grip).toHaveAttribute("aria-valuenow", "336");
  await grip.press("Home");
  await expect(grip).toHaveAttribute("aria-valuenow", "320");
  await expect(
    page.locator(".ranking-detail-image img, img.ranking-detail-image"),
  ).toBeVisible({ timeout: 30000 });
  await page.screenshot({ path: resolve(run, "07-results-2560.png") });
  await page.getByRole("button", { name: "参数配置", exact: true }).click();
  await page.getByText("用途条件", { exact: true }).click();
  await expect(
    page.getByRole("checkbox", { name: "启用成品最短边门槛" }),
  ).toBeHidden();
  await page.getByText("用途条件", { exact: true }).click();
  await expect(
    page.getByRole("checkbox", { name: "启用成品最短边门槛" }),
  ).toBeVisible();
  await page.screenshot({ path: resolve(run, "08-config-2560.png") });
  checks.push(
    "Photoshop-style dock owns the property area, obeys the shared visibility setting, resizes by keyboard, and keeps configuration groups collapsible at 2560x1440",
  );
  assert.deepEqual(errors, []);
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify({ passed: true, checks, errors, project, run }, null, 2),
  );
  console.log(JSON.stringify({ passed: true, checks: checks.length, run }));
} catch (error) {
  await page?.screenshot({ path: resolve(run, "failure.png") }).catch(() => {});
  await writeFile(
    resolve(run, "failure.json"),
    JSON.stringify(
      {
        error: String(error),
        errors,
        body: await page
          ?.locator("body")
          .innerText()
          .catch(() => ""),
        run,
      },
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
