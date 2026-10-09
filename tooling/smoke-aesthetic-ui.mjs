import { browserOptions } from "./platform.mjs";
// Reopen a successful isolated integration-aesthetic fixture; never use a real project.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, open, readFile, writeFile } from "node:fs/promises";
import { resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";
import { EngineFixture, sleep } from "./engine-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
if (!process.argv[2])
  throw new Error("Pass the successful integration-aesthetic run directory");
const fixture = resolve(process.argv[2]);
assert.ok(
  fixture.startsWith(
    resolve(root, ".local/test-runs/integration-aesthetic-"),
  ) && fixture.includes(sep),
);
assert.equal(
  JSON.parse(await readFile(resolve(fixture, "report.json"), "utf8")).passed,
  true,
);
const run = resolve(root, ".local/test-runs/smoke-aesthetic-ui-" + Date.now());
await mkdir(run, { recursive: true });
const engine = new EngineFixture(root, resolve(fixture, "state"), run);
const url = "http://127.0.0.1:1439";
const uiStageName = "界面创建的阶段 " + Date.now();
const checks = [],
  errors = [];
let browser, vite, page;
try {
  await engine.start();
  const log = await open(resolve(run, "vite.log"), "a");
  vite = spawn(
    process.execPath,
    [
      resolve(root, "apps/desktop/node_modules/vite/bin/vite.js"),
      "--port",
      "1439",
      "--strictPort",
    ],
    {
      cwd: resolve(root, "apps/desktop"),
      env: { ...process.env, STUDIO_DATA_DIR: resolve(fixture, "state") },
      windowsHide: true,
      stdio: ["ignore", log.fd, log.fd],
    },
  );
  await log.close();
  for (let n = 0; n < 100; n++) {
    if ((await fetch(url).catch(() => null))?.ok) break;
    if (vite.exitCode !== null) throw new Error("Vite stopped");
    await sleep(100);
  }
  browser = await chromium.launch({ ...browserOptions(), headless: true });
  const context = await browser.newContext({
    viewport: { width: 2560, height: 1440 },
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
  });
  page = await context.newPage();
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(url);
  await page
    .locator(".recent-row")
    .filter({ hasText: "评审链路隔离测试" })
    .click();
  await page.getByRole("button", { name: "美学排序", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "美学排序", exact: true }),
  ).toHaveCount(1);
  await page.locator('[data-aesthetic-view="evaluation"]').click();
  await page
    .locator(".aesthetic-stage-list button")
    .filter({ hasText: "4 批有效" })
    .first()
    .click();
  await expect(page.locator(".aesthetic-main")).toContainText("本阶段已完成");
  await expect(page.locator(".aesthetic-main .aesthetic-counters")).toHaveCount(
    0,
  );
  await page.getByRole("tab", { name: "阶段状态", exact: true }).click();
  await expect(page.locator(".evaluation-status-panel")).toContainText(
    "调用尝试",
  );
  await page.getByRole("tab", { name: "冻结标准", exact: true }).click();
  await expect(page.locator(".evaluation-config-panel pre")).toContainText(
    "system_prompt_id",
  );
  await page.getByRole("tab", { name: "阶段状态", exact: true }).click();
  await page.getByRole("button", { name: "批次与梯队", exact: true }).click();
  await page.locator(".aesthetic-batch summary").first().click();
  await expect(
    page.locator(".aesthetic-batch").first().locator("img"),
  ).toHaveCount(16);
  await page.waitForFunction(() =>
    [...globalThis.document.querySelectorAll(".aesthetic-batch img")].every(
      (image) => image.complete && image.naturalWidth > 0,
    ),
  );
  await page.screenshot({ path: resolve(run, "batch-evidence.png") });
  checks.push(
    "sidebar entry, paginated stage list, counters, 16 image previews and tied observation",
  );
  await page.getByRole("button", { name: "保护候选", exact: true }).click();
  await expect(page.locator(".aesthetic-images img")).toHaveCount(4);
  await expect(page.locator(".aesthetic-main")).toContainText("不会自动加分");
  await page.screenshot({ path: resolve(run, "protected-candidates.png") });
  checks.push(
    "protected candidates display independently from final selection",
  );
  const projects = (await engine.api("/v1/projects")).items;
  const pid = projects.find((p) => p.name === "评审链路隔离测试").id;
  const stage = (await engine.api(`/v1/projects/${pid}/aesthetic/stages`))
    .items[0];
  await page.getByRole("button", { name: "新建评审", exact: true }).click();
  await page.getByLabel("阶段名称", { exact: true }).fill(uiStageName);
  await page
    .getByLabel("候选工作集", { exact: true })
    .selectOption(stage.config.request.collection_id);
  await page
    .getByLabel("评审模型", { exact: true })
    .selectOption(stage.config.request.model_id);
  await page
    .getByLabel("审美标准（System Prompt）", { exact: true })
    .selectOption(stage.config.request.system_prompt_id);
  await page.getByLabel("采样方式", { exact: true }).selectOption("refine");
  await page.getByLabel("调用次数上限", { exact: true }).fill("3");
  await page
    .getByRole("checkbox", {
      name: "小预算试跑：允许预算低于最低曝光下界",
      exact: true,
    })
    .check();
  await page.getByRole("button", { name: "预检输入", exact: true }).click();
  await expect(page.getByLabel("输入预检结果")).toContainText(
    "基础容量预检通过",
  );
  await page
    .getByRole("button", { name: "创建并冻结候选", exact: true })
    .click();
  await expect(page.locator(".aesthetic-main")).toContainText(uiStageName);
  await expect(page.locator(".aesthetic-main")).toContainText("待开始", {
    timeout: 30000,
  });
  const created = (
    await engine.api(`/v1/projects/${pid}/aesthetic/stages`)
  ).items.find((s) => s.name === uiStageName);
  assert.equal(created.attempts, 0);
  assert.equal(created.config.request.max_calls, 3);
  assert.equal(created.config.request.budget_mode, "trial");
  assert.equal(created.config.request.sampling.mode, "refine");
  assert.equal(created.sampling.version, "neighbor_budget_v2");
  await page.getByRole("button", { name: "资料浏览", exact: true }).click();
  await page.getByRole("button", { name: "美学排序", exact: true }).click();
  await page.getByRole("button", { name: "新建评审", exact: true }).click();
  await expect(page.getByLabel("阶段名称", { exact: true })).toHaveValue(
    uiStageName,
  );
  checks.push(
    "form creates frozen stage with zero network attempts; draft survives module switch",
  );
  assert.deepEqual(errors, []);
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify({ passed: true, checks, errors, fixture }, null, 2),
  );
  console.log(
    JSON.stringify({
      passed: true,
      checks: checks.length,
      report: resolve(run, "report.json"),
    }),
  );
} catch (error) {
  await page?.screenshot({ path: resolve(run, "failure.png") }).catch(() => {});
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      { passed: false, checks, errors, error: String(error) },
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
