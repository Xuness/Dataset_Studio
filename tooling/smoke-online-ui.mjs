import { browserOptions } from "./platform.mjs";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, readFile, writeFile, open } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";
import { EngineFixture, sleep } from "./engine-fixture.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const fixture = resolve(process.argv[2] ?? "");
assert.ok(fixture.startsWith(resolve(root, ".local/test-runs/online-")));
const lakes = JSON.parse(
  await readFile(resolve(fixture, "multibooru.json"), "utf8"),
);
const run = resolve(root, ".local/test-runs/smoke-online-ui-" + Date.now());
await mkdir(run, { recursive: true });
const engine = new EngineFixture(root, resolve(run, "state"));
const url = "http://127.0.0.1:1459";
let browser, vite, page;
const errors = [],
  requests = [],
  checks = [];
try {
  await engine.start();
  const project = await engine.api("/v1/projects", "POST", {
    name: "Online UI",
  });
  for (const [name, lake] of Object.entries(lakes))
    await engine.api("/v1/projects/" + project.id + "/sources", "POST", {
      kind: "auto",
      name,
      index_root: lake.lake,
      media_root: lake.lake,
    });
  const log = await open(resolve(run, "vite.log"), "a");
  vite = spawn(
    process.execPath,
    [
      resolve(root, "apps/desktop/node_modules/vite/bin/vite.js"),
      "--port",
      "1459",
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
  browser = await chromium.launch({ ...browserOptions(), headless: true });
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
  page.on("request", (r) => {
    if (r.method() === "POST" && /\/query-(views|results)$/.test(r.url()))
      requests.push({ path: r.url(), body: r.postDataJSON() });
  });
  await page.goto(url);
  await page.locator(".recent-row").filter({ hasText: project.name }).click();
  await page
    .locator(".source-tree-row button.tree-row")
    .filter({ hasText: "gelbooru" })
    .click();
  await expect(page.locator(".asset-grid article").first()).toBeVisible({
    timeout: 30000,
  });
  await page.getByRole("button", { name: "定位与显示", exact: true }).click();
  await page
    .getByLabel("浏览排序", { exact: true })
    .selectOption("post_id_asc");
  await page
    .getByRole("button", { name: "查看 Gelbooru #10001", exact: true })
    .click();
  await page.getByRole("tab", { name: "检查器", exact: true }).click();
  await page.getByRole("tab", { name: "属性", exact: true }).click();
  const inspector = page.getByRole("region", {
    name: "元数据检查",
    exact: true,
  });
  await expect(inspector).toContainText("32 × 40", { timeout: 30000 });
  const tag = "censored\n";
  await inspector
    .getByRole("button", { name: JSON.stringify(tag), exact: true })
    .click();
  await expect(page.getByLabel("条件值 1", { exact: true })).toHaveValue(
    JSON.stringify(tag),
  );
  const response = page.waitForResponse(
    (r) => r.request().method() === "POST" && r.url().endsWith("/query-views"),
  );
  await page.getByRole("button", { name: "执行查询", exact: true }).click();
  const view = await (await response).json();
  assert.equal(view.state, "ready");
  assert.equal(view.count, null);
  await expect(page.locator(".asset-grid article")).toHaveCount(1, {
    timeout: 30000,
  });
  await expect(page.locator(".query-result-row.current")).toContainText(
    "未计算总数",
  );
  assert.equal(
    requests.filter((r) => r.path.endsWith("/query-results")).length,
    0,
  );
  checks.push(
    "ordinary tag query creates a ready paged view without materializing the whole result",
  );
  await page.screenshot({ path: resolve(run, "paged-view.png") });
  await page.getByTitle("将当前浏览范围保存为工作集", { exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("工作集名称", { exact: true }).fill("固定浏览视图");
  await dialog.getByRole("button", { name: "保存", exact: true }).click();
  await expect(dialog).toHaveCount(0, { timeout: 30000 });
  const collections = await engine.api(
    "/v1/projects/" + project.id + "/collections",
  );
  assert.ok(
    collections.items.some((c) => c.name === "固定浏览视图" && c.count === 1),
  );
  checks.push("workset dialog captures and saves the visible view");
  await page.screenshot({ path: resolve(run, "captured-workset.png") });
  assert.deepEqual(errors, []);
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify({ status: "passed", checks }, null, 2),
  );
  console.log(JSON.stringify({ run, checks }));
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
