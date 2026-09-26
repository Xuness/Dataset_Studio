import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, readFile, writeFile, open } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";
import { EngineFixture, sleep } from "./engine-fixture.mjs";

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
  await dialog
    .getByLabel("快速索引目录", { exact: true })
    .fill(lakes.gelbooru.lake);
  await dialog
    .getByLabel("图片湖目录", { exact: true })
    .fill(lakes.gelbooru.lake);
  await dialog.getByRole("button", { name: "检查数据湖", exact: true }).click();
  await expect(dialog.getByRole("status")).toContainText("已识别 Gelbooru");
  await page.screenshot({ path: resolve(run, "source-probe.png") });
  await dialog.getByRole("button", { name: "加入项目", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
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
  assert.deepEqual(errors, []);
  checks.push(
    "automatic source preflight and registration in a fresh isolated project",
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
  await browser?.close();
  if (vite && vite.exitCode === null) {
    const exited = new Promise((done) => vite.once("exit", done));
    vite.kill();
    await exited;
  }
  await engine.stop();
}
