// Optional Windows native smoke check; all state and media belong to its fixture.
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
  "smoke-settings-ui-" + Date.now(),
);
const state = resolve(run, "state");
const execute = promisify(execFile);
await mkdir(run, { recursive: true });
const engine = new EngineFixture(root, state);
const children = [];
const checks = [];
const errors = [];
let browser;
let page;
async function launch(command, args, name, cwd = root, extra = {}) {
  const log = await open(resolve(run, name + ".log"), "a");
  const child = spawn(command, args, {
    cwd,
    env: {
      ...process.env,
      STUDIO_DATA_DIR: state,
      STUDIO_DEVELOPMENT: "1",
      ...extra,
    },
    stdio: ["ignore", log.fd, log.fd],
    windowsHide: true,
  });
  children.push(child);
  await log.close();
  return child;
}
async function menu(name, item) {
  await page.getByRole("menuitem", { name, exact: true }).click();
  await page.getByRole("menuitem", { name: item, exact: true }).click();
}
async function screenshot(name) {
  await page.screenshot({ path: resolve(run, name + ".png") });
}
async function closeSettings() {
  await page
    .locator(".settings-footer")
    .getByRole("button", { name: "关闭", exact: true })
    .click();
  await expect(page.locator("dialog[open]")).toHaveCount(0);
}
try {
  assert.equal(
    process.platform,
    "win32",
    "This smoke check uses the Windows native host",
  );
  const active = await fetch("http://127.0.0.1:1420/__studio/workspace", {
    signal: AbortSignal.timeout(700),
  }).catch(() => null);
  assert.equal(
    active,
    null,
    "Close the development server before running this isolated smoke check",
  );
  await execute("python", [resolve(root, "tooling/ui-fixture.py"), run], {
    windowsHide: true,
  });
  const fixture = JSON.parse(
    await readFile(resolve(run, "fixture.json"), "utf8"),
  );
  await engine.start();
  const project = await engine.api("/v1/projects", "POST", {
    name: "设置窗口验收",
  });
  const source = await engine.api(
    "/v1/projects/" + project.id + "/sources",
    "POST",
    {
      name: "渐变参考资料",
      kind: "danbooru",
      index_root: fixture.lake,
      media_root: fixture.lake,
    },
  );
  await engine.api("/v1/projects/" + project.id + "/close", "POST");
  await launch(
    process.execPath,
    [resolve(root, "apps/desktop/node_modules/vite/bin/vite.js")],
    "vite",
    resolve(root, "apps/desktop"),
  );
  for (let i = 0; i < 100; i++) {
    const response = await fetch("http://127.0.0.1:1420", {
      signal: AbortSignal.timeout(700),
    }).catch(() => null);
    if (response?.ok) break;
    await sleep(100);
  }
  const native = await launch(
    resolve(root, "target/debug/studio-desktop.exe"),
    [],
    "native",
    root,
    {
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-port=9347",
      WEBVIEW2_USER_DATA_FOLDER: resolve(run, "webview"),
    },
  );
  for (let i = 0; i < 160; i++) {
    try {
      browser = await chromium.connectOverCDP("http://127.0.0.1:9347", {
        timeout: 700,
      });
      break;
    } catch {
      await sleep(100);
    }
  }
  assert.ok(browser, "Native WebView2 debugger must be available");
  page = browser.contexts()[0].pages()[0];
  page.on("pageerror", (error) => errors.push(error.message));
  await expect(
    page.locator(".recent-row").filter({ hasText: project.name }),
  ).toBeVisible({ timeout: 30000 });
  await menu("设置", "缓存与存储…");
  await expect(page.getByLabel("总缓存预算 GiB", { exact: true })).toHaveValue(
    "64",
  );
  await expect(page.getByLabel("临时缓存闲置小时")).toHaveValue("24");
  await expect(page.getByLabel("长期缓存过期方式")).toHaveValue("never");
  await page.getByLabel("总缓存预算 GiB", { exact: true }).fill("100");
  await page.getByRole("button", { name: "性能与任务", exact: true }).click();
  await page.getByLabel("查询工作内存 GiB").fill("16");
  await page.getByRole("button", { name: "缓存与存储", exact: true }).click();
  await expect(page.getByLabel("总缓存预算 GiB", { exact: true })).toHaveValue(
    "100",
  );
  await page.getByLabel("临时缓存保留方式").selectOption("session");
  await page.getByRole("button", { name: "保存缓存设置" }).click();
  await expect(
    page.getByRole("status").filter({ hasText: "缓存设置已保存" }),
  ).toBeVisible();
  assert.equal((await engine.api("/v1/settings")).cache.total_mib, 102400);
  assert.equal(
    (await engine.api("/v1/settings")).cache.temporary_session_only,
    true,
  );
  await page.locator(".settings-main").evaluate((element) => {
    element.scrollTop = 0;
  });
  await screenshot("settings-cache");
  const layout = await page.locator(".settings-dialog").evaluate((element) => {
    const bounds = element.getBoundingClientRect();
    return {
      x: bounds.x,
      y: bounds.y,
      right: bounds.right,
      bottom: bounds.bottom,
      viewport: [
        element.ownerDocument.defaultView.innerWidth,
        element.ownerDocument.defaultView.innerHeight,
      ],
      overflow: element.scrollWidth - element.clientWidth,
    };
  });
  assert.ok(
    layout.x >= 0 &&
      layout.y >= 0 &&
      layout.right <= layout.viewport[0] &&
      layout.bottom <= layout.viewport[1] &&
      layout.overflow <= 1,
    JSON.stringify(layout),
  );
  await page.getByRole("button", { name: "性能与任务", exact: true }).click();
  await expect(page.getByLabel("查询工作内存 GiB")).toHaveValue("16");
  await page.getByRole("button", { name: "保存性能设置" }).click();
  await expect(
    page.getByRole("status").filter({ hasText: "查询预算已保存" }),
  ).toBeVisible();
  await screenshot("settings-performance");
  await expect(page.locator("dialog[open]")).toHaveCount(1);
  await closeSettings();
  checks.push(
    "Native settings work before project open; 100 GiB and session mode persist; cache and memory drafts survive category navigation; one bounded dialog",
  );

  await page.locator(".recent-row").filter({ hasText: project.name }).click();
  await expect(page.getByRole("region", { name: "浏览筛选" })).toBeVisible({
    timeout: 30000,
  });
  await menu("设置", "缓存管理…");
  await page.getByRole("button", { name: "建立或更新全部分级" }).click();
  await engine.wait(
    "/v1/cache/rating-bases",
    (value) => value.items.length === 4,
    30000,
  );
  await expect(
    page.locator(".settings-table").first().locator("tbody tr"),
  ).toHaveCount(4);
  await screenshot("settings-bases");
  await closeSettings();
  const filters = page.getByRole("region", { name: "浏览筛选" });
  if (!(await filters.getByLabel("包含标签", { exact: true }).isVisible()))
    await page
      .getByRole("button", { name: "Rating / Tag 筛选", exact: true })
      .click();
  await filters.getByLabel("分级 G", { exact: true }).click();
  await filters.getByLabel("包含标签", { exact: true }).fill("solo");
  await filters.getByRole("button", { name: "应用筛选" }).click();
  const result = await engine.wait(
    "/v1/projects/" + project.id + "/query-results",
    (value) =>
      value.items.some(
        (r) => r.state === "ready" && r.cache.basis_ratings.includes("g"),
      ),
    30000,
  );
  const g = result.items.find(
    (r) => r.state === "ready" && r.cache.basis_ratings.includes("g"),
  );
  assert.equal(g.cache.tier, "temporary");
  assert.equal(g.cache.session_only, true);
  assert.equal(
    g.count,
    fixture.images.filter((row) =>
      row.current.some(
        (observation) =>
          observation.rating === "g" && observation.tags?.includes("solo"),
      ),
    ).length,
  );
  await expect(filters).toContainText("基础");
  await screenshot("filtered-candidates");
  await menu("项目", "关闭当前项目");
  await expect(
    page.locator(".recent-row").filter({ hasText: project.name }),
  ).toBeVisible();
  await page.locator(".recent-row").filter({ hasText: project.name }).click();
  await expect(page.getByRole("region", { name: "浏览筛选" })).toContainText(
    "筛选缓存已结束",
    { timeout: 20000 },
  );
  await screenshot("session-ended");
  await page
    .getByRole("region", { name: "浏览筛选" })
    .getByRole("button", { name: "应用筛选" })
    .click();
  const rebuilt = await engine.wait(
    "/v1/projects/" + project.id + "/query-results",
    (value) =>
      value.items.some(
        (r) =>
          r.id !== g.id &&
          r.state === "ready" &&
          r.cache.basis_ratings.includes("g"),
      ),
    30000,
  );
  const next = rebuilt.items.find(
    (r) =>
      r.id !== g.id &&
      r.state === "ready" &&
      r.cache.basis_ratings.includes("g"),
  );
  assert.notEqual(next.cache.mode, "reused");
  await menu("设置", "缓存管理…");
  await page
    .getByLabel("缓存类别 " + next.id, { exact: true })
    .selectOption("long_term");
  await expect(
    page.getByLabel("缓存类别 " + next.id, { exact: true }),
  ).toHaveValue("long_term");
  await page.getByLabel("固定缓存 " + next.id, { exact: true }).click();
  await expect(
    page.getByLabel("固定缓存 " + next.id, { exact: true }),
  ).toBeChecked();
  await page.locator(".settings-main").evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });
  await screenshot("settings-retention");
  await closeSettings();
  checks.push(
    "Native four-rating prebuild and real G plus Tag candidate query; closing/reopening expires session membership; applying again rebuilds; manager promotes and fixes the same result",
  );
  assert.deepEqual(errors, []);
  const exited = new Promise((done) => native.once("exit", done));
  await page.getByLabel("关闭窗口", { exact: true }).click();
  await Promise.race([exited, sleep(10000)]);
  assert.notEqual(native.exitCode, null, "Normal native close must exit");
  checks.push("Native close completes without page errors");
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      { passed: true, checks, layout, project, source, query: next, errors },
      null,
      2,
    ),
  );
  console.log(
    JSON.stringify({
      passed: true,
      checks: checks.length,
      report: resolve(run, "report.json"),
    }),
  );
} catch (error) {
  await screenshot("failure").catch(() => {});
  await writeFile(
    resolve(run, "failure.json"),
    JSON.stringify(
      { error: String(error), stack: error.stack, errors },
      null,
      2,
    ),
  );
  throw error;
} finally {
  await browser?.close().catch(() => {});
  for (const child of children.reverse())
    if (child.exitCode === null) child.kill();
  await engine.stop();
}
