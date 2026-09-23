// Reopen an isolated offline-analysis fixture; never use a real project.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, open, readFile, writeFile } from "node:fs/promises";
import { resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";
import { EngineFixture, sleep } from "./engine-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
if (!process.argv[2])
  throw new Error(
    "Pass the successful integration-aesthetic-analysis run directory",
  );
const fixture = resolve(process.argv[2]);
assert.ok(
  fixture.startsWith(
    resolve(root, ".local/test-runs/integration-aesthetic-analysis-"),
  ) && fixture.includes(sep),
);
assert.equal(
  JSON.parse(await readFile(resolve(fixture, "report.json"), "utf8")).passed,
  true,
);
const run = resolve(root, ".local/test-runs/smoke-workbench-ui-" + Date.now());
await mkdir(run, { recursive: true });
const engine = new EngineFixture(root, resolve(fixture, "state"), run);
const url = "http://127.0.0.1:1447";

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
      "1447",
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
  browser = await chromium.launch({ channel: "msedge", headless: true });
  const context = await browser.newContext({
    viewport: { width: 1600, height: 1100 },
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
  const dispatched = [];
  page.on("request", (request) => {
    if (
      request.method() === "POST" &&
      /aesthetic\/stages\/[^/]+\/control$/.test(request.url())
    )
      dispatched.push(request.postData());
  });
  const projects = (await engine.api("/v1/projects")).items;
  const pid = projects.find(
    (project) => project.name === "离线美学后端验收",
  ).id;
  const rootPath = `/v1/projects/${pid}/aesthetic`;
  await engine.api(`/v1/projects/${pid}/open`, "POST");
  const stagesBefore = (await engine.api(rootPath + "/stages")).items;
  const beforeAttempts = stagesBefore.reduce(
    (sum, stage) => sum + stage.attempts,
    0,
  );
  const stage = stagesBefore.find((stage) => stage.accepted > 0);
  await page.goto(url);
  await page
    .locator(".recent-row")
    .filter({ hasText: "离线美学后端验收" })
    .click();
  await page.getByRole("button", { name: "美学排序", exact: true }).click();
  await expect(page.getByLabel("美学工作视图", { exact: true })).toHaveValue(
    "ranking",
  );
  await expect(page.locator(".ranking-image-tile").first()).toBeVisible();
  await page.getByLabel("排名每页图片数").selectOption("12");
  await expect(page.locator(".ranking-image-tile")).toHaveCount(12);
  await page.waitForFunction(() => {
    const images = [
      ...globalThis.document.querySelectorAll(".ranking-image-tile img"),
    ];
    return (
      images.length === 12 &&
      images.every((image) => image.complete && image.naturalWidth > 0)
    );
  });
  await page.screenshot({ path: resolve(run, "ranking-grid.png") });
  const target = page.locator(".ranking-image-tile").nth(2);
  const label = await target.getAttribute("aria-label");
  await target.click();
  await page.getByRole("button", { name: "资料浏览", exact: true }).click();
  await page.getByRole("button", { name: "美学排序", exact: true }).click();
  await expect(
    page.getByRole("button", { name: label, exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  checks.push(
    "ranking-first entry, bounded image loading and selection survives editor switch",
  );

  await page.getByRole("button", { name: "排名下一页", exact: true }).click();
  await expect(page.locator(".ranking-pagebar")).toContainText("第 2 页");
  await expect(page.locator(".status-bar")).toContainText("项目已保存");
  await page.reload();
  await expect(page.locator(".ranking-pagebar")).toContainText("第 2 页");
  await page.getByRole("button", { name: "排名上一页", exact: true }).click();
  await expect(page.locator(".ranking-pagebar")).toContainText("第 1 页");
  checks.push(
    "ranking cursor, page size, active editor and view restored after reload",
  );

  await page
    .getByRole("button", { name: "详情面板更多操作", exact: true })
    .click();
  await page.getByRole("menuitem", { name: "停靠到左侧", exact: true }).click();
  await expect(
    page.locator(".aesthetic-workspace .wb-left [role=tab]"),
  ).toHaveCount(2);
  await expect(page.locator(".status-bar")).toContainText("项目已保存");
  await page.reload();
  await expect(
    page.locator(".aesthetic-workspace .wb-left [role=tab]"),
  ).toHaveCount(2);
  await page.getByRole("button", { name: "隐藏详情面板", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "显示详情", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "显示详情", exact: true }).click();
  await page.getByRole("button", { name: "恢复默认布局", exact: true }).click();
  await expect(page.locator(".aesthetic-workspace .wb-right")).toBeVisible();
  const grip = page.getByRole("separator", { name: "详情面板宽度" });
  await grip.focus();
  const oldSize = Number(await grip.getAttribute("aria-valuenow"));
  await grip.press("ArrowLeft");
  await expect(grip).toHaveAttribute("aria-valuenow", String(oldSize + 16));
  await grip.press("Home");
  checks.push(
    "panel docking, grouped tabs, persistence, hidden-panel recovery and keyboard resize",
  );

  await page.getByRole("button", { name: "查看大图", exact: true }).click();
  await expect(page.locator(".ranking-full-image img")).toBeVisible();
  await page
    .locator(".ranking-full-image")
    .getByRole("button", { name: "返回排名网格" })
    .click();
  await page
    .locator(".ranking-inspector summary")
    .filter({ hasText: "评审配置" })
    .click();
  await page
    .getByRole("button", { name: "查看冻结 System Prompt", exact: true })
    .click();
  await expect(
    page.getByRole("dialog", { name: "冻结的 System Prompt", exact: true }),
  ).toContainText("合成评审 fixture");
  await page.getByRole("button", { name: "关闭冻结的 System Prompt" }).click();
  checks.push(
    "full image and immutable system prompt inspection use production media and stage APIs",
  );

  const fitName = "UI 离线排名 " + Date.now();
  await page
    .locator(".wb-toolbar")
    .getByRole("button", { name: "生成排名快照", exact: true })
    .click();
  await page.getByLabel("快照名称", { exact: true }).fill(fitName);
  await page.getByLabel("来源评审阶段", { exact: true }).selectOption(stage.id);
  await page.getByRole("button", { name: "开始离线计算", exact: true }).click();
  await expect(page.locator(".ranking-task-state")).toContainText("已完成", {
    timeout: 60000,
  });
  await expect(page.locator(".ranking-task-state")).toContainText(fitName);
  const createdFit = (
    await engine.api(rootPath + "/analysis/jobs?limit=32")
  ).items.find((job) => job.request.name === fitName);
  assert.equal(createdFit.state, "completed");
  await expect(page.locator(".ranking-image-tile").first()).toBeVisible();
  checks.push("UI submits real offline fit and opens its published snapshot");

  await page.getByRole("button", { name: "复核保护状态", exact: true }).click();
  await page.getByLabel("复核人", { exact: true }).fill("UI fixture");
  await page
    .getByLabel("复核理由", { exact: true })
    .fill("合成图片保护复核测试");
  await page.getByRole("button", { name: "保存复核决定", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  const reviews = await engine.api(
    rootPath + `/analysis/snapshots/${createdFit.id}/reviews`,
  );
  assert.ok(
    reviews.items.some((review) => review.request.reviewer === "UI fixture"),
  );
  await page
    .getByLabel("美学工作视图", { exact: true })
    .selectOption("protected");
  await expect(page.locator(".ranking-image-tile").first()).toBeVisible();
  await expect(page.locator(".ranking-pagebar")).toContainText("复核水位");
  await page.screenshot({ path: resolve(run, "protection-review.png") });
  await page
    .getByLabel("美学工作视图", { exact: true })
    .selectOption("ranking");
  checks.push(
    "append-only protection review and effective protection browse with fixed watermark",
  );

  const worksetName = "UI 派生工作集 " + Date.now();
  await page.getByRole("button", { name: "生成工作集", exact: true }).click();
  await page.getByLabel("工作集名称", { exact: true }).fill(worksetName);
  await page.getByRole("button", { name: "预览筛选", exact: true }).click();
  await expect(page.getByRole("dialog")).toContainText("复核水位");
  await page
    .getByRole("button", { name: "生成完整工作集", exact: true })
    .click();
  await expect(page.locator(".ranking-task-state")).toContainText("已完成", {
    timeout: 60000,
  });
  await expect(page.locator(".ranking-task-state")).toContainText(worksetName);
  const derived = (
    await engine.api(rootPath + "/analysis/jobs?limit=32")
  ).items.find((job) => job.request.name === worksetName);
  assert.equal(derived.result.kind, "derive");
  assert.ok(derived.result.count > 0);
  await page.getByRole("button", { name: "打开工作集", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "资料浏览", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(".asset-card").first()).toBeVisible();
  await page.waitForFunction(
    () =>
      globalThis.document.querySelector(".asset-card img")?.naturalWidth > 0,
  );
  await page.getByRole("button", { name: "隐藏项目面板", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "显示项目", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "恢复默认布局", exact: true }).click();
  await expect(page.locator(".project-panel")).toBeVisible();
  await page.screenshot({ path: resolve(run, "browser-workbench.png") });
  checks.push(
    "previewed filter derives complete workset and hands it to shared browsing workspace",
  );

  await page.getByRole("button", { name: "美学排序", exact: true }).click();
  await page.getByRole("button", { name: "新建评审", exact: true }).click();
  await page.getByLabel("阶段名称", { exact: true }).fill("UI 未提交草稿");
  await page
    .getByRole("button", { name: "关闭新建评审阶段", exact: true })
    .click();
  await page.getByRole("button", { name: "资料浏览", exact: true }).click();
  await page.getByRole("button", { name: "美学排序", exact: true }).click();
  await page.getByRole("button", { name: "新建评审", exact: true }).click();
  await expect(page.getByLabel("阶段名称", { exact: true })).toHaveValue(
    "UI 未提交草稿",
  );
  await page.screenshot({ path: resolve(run, "create-stage.png") });
  await page
    .getByRole("button", { name: "关闭新建评审阶段", exact: true })
    .click();
  await page
    .getByLabel("美学工作视图", { exact: true })
    .selectOption("ranking");
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.screenshot({ path: resolve(run, "ranking-1280.png") });
  assert.ok(
    await page.evaluate(
      () =>
        globalThis.document.documentElement.scrollWidth <=
        globalThis.innerWidth,
    ),
  );
  const afterAttempts = (await engine.api(rootPath + "/stages")).items.reduce(
    (sum, stage) => sum + stage.attempts,
    0,
  );
  assert.equal(afterAttempts, beforeAttempts);
  assert.deepEqual(dispatched, []);
  checks.push(
    "creation draft survives dialog and module closure; 1280 layout fits; no paid dispatch",
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
