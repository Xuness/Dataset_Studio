// Open only a successful isolated execution fixture. No UI action may start paid evaluation.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { mkdir, open, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";
import { EngineFixture, sleep, within } from "./engine-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
assert.ok(
  process.argv[2],
  "Pass a successful integration-aesthetic-execution run",
);
const fixture = within(
  resolve(root, ".local/test-runs"),
  resolve(process.argv[2]),
);
assert.ok(
  fixture.startsWith(
    resolve(root, ".local/test-runs/integration-aesthetic-execution-"),
  ),
);
const report = JSON.parse(
  await readFile(resolve(fixture, "report.json"), "utf8"),
);
assert.equal(report.passed, true);
const run = resolve(
  root,
  ".local/test-runs/smoke-aesthetic-execution-ui-" + Date.now(),
);
await mkdir(run, { recursive: true });
const engine = new EngineFixture(root, resolve(fixture, "state"), run);
const reservation = createServer();
await new Promise((done) => reservation.listen(0, "127.0.0.1", done));
const port = reservation.address().port;
await new Promise((done) => reservation.close(done));
const url = `http://127.0.0.1:${port}`;
const checks = [],
  errors = [];
let browser, vite, page;
const stagePath = (id) =>
  `/v1/projects/${report.projectId}/aesthetic/stages/${id}`;
async function selectStage(id) {
  await page
    .getByLabel("美学工作视图", { exact: true })
    .selectOption("evaluation");
  await page.getByLabel("阶段归档范围").selectOption("active");
  const stage = await engine.api(stagePath(id));
  await page.getByLabel("搜索评审阶段").fill(stage.name);
  await page
    .locator(".aesthetic-stage-list button")
    .filter({ hasText: stage.name })
    .first()
    .click();
  await expect(
    page.getByRole("heading", { name: stage.name, exact: true }),
  ).toBeVisible();
  return stage;
}
try {
  await engine.start();
  const log = await open(resolve(run, "vite.log"), "a");
  vite = spawn(
    process.execPath,
    [
      resolve(root, "apps/desktop/node_modules/vite/bin/vite.js"),
      "--port",
      String(port),
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
  for (let i = 0; i < 120; i++) {
    if ((await fetch(url).catch(() => null))?.ok) break;
    if (vite.exitCode !== null) throw Error("Vite stopped");
    await sleep(100);
  }
  browser = await chromium.launch({ channel: "msedge", headless: true });
  const context = await browser.newContext({
    viewport: { width: 1600, height: 1080 },
  });
  await context.route(url + "/__studio/connection", (route) =>
    route.fulfill({ json: engine.connection }),
  );
  await context.route(engine.connection.endpoint + "/**", async (route) => {
    const request = route.request();
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
    if (request.url().endsWith("/control") && request.postData())
      assert.notEqual(
        JSON.parse(request.postData()).action,
        "start",
        "UI smoke must never dispatch model calls",
      );
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
    .filter({ hasText: "美学执行恢复隔离测试" })
    .click();
  if (!(await page.getByLabel("美学工作视图", { exact: true }).count()))
    await page.getByRole("button", { name: "美学排序", exact: true }).click();

  const isolated = await selectStage(report.cases.isolated);
  await page.getByRole("button", { name: "异常候选", exact: true }).click();
  await page.getByLabel("候选状态", { exact: true }).selectOption("blocked");
  await expect(page.locator(".evaluation-candidate")).toHaveCount(16);
  await expect(
    page.getByRole("button", { name: /定位关联批次/ }),
  ).toBeVisible();
  await page.screenshot({ path: resolve(run, "candidate-locks.png") });
  await page.getByRole("button", { name: /定位关联批次/ }).click();
  await expect(page.locator(".aesthetic-batch")).toHaveCount(1);
  await expect(page.locator(".aesthetic-batch")).toHaveAttribute("open", "");
  await expect(page.locator(".aesthetic-batch summary").first()).toContainText(
    "批次 1",
  );
  checks.push(
    "blocked transport candidates are visible and link directly to their expanded batch with a stage-local number",
  );

  await page
    .getByRole("button", { name: "批量重试异常批次", exact: true })
    .click();
  const recovery = page.getByRole("dialog", {
    name: "批量安排重试",
    exact: true,
  });
  await expect(
    recovery.getByRole("button", { name: "确认加入重试队列" }),
  ).toBeDisabled();
  await recovery.getByRole("checkbox").check();
  await recovery.getByRole("button", { name: "确认加入重试队列" }).click();
  await expect(recovery.getByText(/已处理 1 批/)).toBeVisible();
  await recovery.getByRole("button", { name: "完成", exact: true }).click();
  assert.equal(
    (await engine.api(stagePath(isolated.id))).attempts,
    isolated.attempts,
  );
  checks.push(
    "one acknowledged bulk action queues exceptions without starting or charging another model request",
  );

  await page.getByRole("button", { name: "查看排名快照", exact: true }).click();
  const fit = page.getByRole("dialog", { name: "生成排名快照", exact: true });
  await expect(fit).toBeVisible();
  await expect(fit.getByLabel("来源评审阶段", { exact: true })).toHaveValue(
    isolated.id,
  );
  await page.screenshot({ path: resolve(run, "snapshot-context.png") });
  await fit.getByRole("button", { name: "取消", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "从一份排名快照开始", exact: true }),
  ).toBeVisible();
  checks.push(
    "a stage without a snapshot opens a prefilled fit dialog without displaying an unrelated existing snapshot",
  );

  await selectStage(report.cases.valid);
  await page.getByRole("button", { name: "批次与梯队", exact: true }).click();
  await page.getByLabel("批次状态", { exact: true }).selectOption("");
  await page.locator(".aesthetic-batch summary").first().click();
  await expect(page.getByLabel("批次 1 图片顺序")).toHaveValue("tiers");
  await expect(
    page.locator(".aesthetic-batch .aesthetic-tier-tag"),
  ).toHaveCount(16);
  await expect(
    page.locator(".aesthetic-batch .aesthetic-images img"),
  ).toHaveCount(16);
  await page.waitForFunction(() =>
    [
      ...globalThis.document.querySelectorAll(
        ".aesthetic-batch .aesthetic-images img",
      ),
    ].every((img) => img.complete && img.naturalWidth > 0),
  );
  await page.screenshot({ path: resolve(run, "batch-tiers.png") });
  await page
    .getByRole("button", { name: "查看大图 img01", exact: true })
    .click();
  await expect(
    page.getByRole("dialog", { name: "评审图片大图", exact: true }),
  ).toBeVisible();
  await page.waitForFunction(() =>
    [
      ...globalThis.document.querySelectorAll(".aesthetic-evidence-viewer img"),
    ].some((img) => img.complete && img.naturalWidth > 0),
  );
  await page.screenshot({ path: resolve(run, "evidence-viewer.png") });
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("dialog", { name: "评审图片大图", exact: true }),
  ).toHaveCount(0);
  checks.push(
    "tier order and input order are available; evidence images reuse the zoomable keyboard-navigable viewer",
  );

  await page.getByRole("button", { name: "执行设置", exact: true }).click();
  const execution = page.getByRole("dialog", {
    name: "阶段执行设置",
    exact: true,
  });
  await expect(execution.getByLabel("响应方式", { exact: true })).toHaveValue(
    "stream",
  );
  await expect(
    execution.getByLabel("首包等待（秒）", { exact: true }),
  ).toHaveValue("2");
  await expect(
    execution.getByLabel("单次请求总时限（秒）", { exact: true }),
  ).toHaveValue("4");
  await page.screenshot({ path: resolve(run, "execution-settings.png") });
  await execution
    .getByRole("button", { name: "关闭阶段执行设置", exact: true })
    .click();
  await page.getByRole("button", { name: "复制配置", exact: true }).click();
  const copy = page.getByRole("dialog", { name: "新建评审阶段", exact: true });
  await expect(copy).toBeVisible();
  await expect(copy.getByLabel("阶段名称", { exact: true })).toHaveValue(
    /副本/,
  );
  await copy
    .getByRole("button", { name: "关闭新建评审阶段", exact: true })
    .click();
  checks.push(
    "execution settings show independent time budgets and stage cloning preserves editable configuration",
  );

  if (report.cases.noRaw) {
    const noRaw = await selectStage(report.cases.noRaw);
    await page.getByRole("button", { name: "批次与梯队", exact: true }).click();
    await page.locator(".aesthetic-batch summary").first().click();
    await expect(
      page.getByRole("button", { name: "本地重解析原始回执", exact: true }),
    ).toBeDisabled();
    await page.getByRole("button", { name: "结束阶段", exact: true }).click();
    const end = page.getByRole("dialog", { name: "结束评审阶段", exact: true });
    await expect(end).toBeVisible();
    await end.getByRole("button", { name: "返回", exact: true }).click();
    assert.equal((await engine.api(stagePath(noRaw.id))).state, noRaw.state);
    await page.getByRole("button", { name: "管理阶段", exact: true }).click();
    let manage = page.getByRole("dialog", {
      name: "管理评审阶段",
      exact: true,
    });
    await manage
      .getByLabel("阶段名称", { exact: true })
      .fill("归档恢复界面验证");
    await manage.getByRole("button", { name: "保存名称", exact: true }).click();
    await expect(manage).toHaveCount(0);
    await page.getByRole("button", { name: "管理阶段", exact: true }).click();
    manage = page.getByRole("dialog", { name: "管理评审阶段", exact: true });
    await manage.getByRole("button", { name: "归档阶段", exact: true }).click();
    await page.getByLabel("搜索评审阶段").fill("");
    await page.getByLabel("阶段归档范围").selectOption("archived");
    await page
      .locator(".aesthetic-stage-list button")
      .filter({ hasText: "归档恢复界面验证" })
      .click();
    await page.getByRole("button", { name: "管理阶段", exact: true }).click();
    manage = page.getByRole("dialog", { name: "管理评审阶段", exact: true });
    await manage.getByRole("button", { name: "恢复阶段", exact: true }).click();
    await expect(manage).toHaveCount(0);
    const restored = await engine.api(stagePath(noRaw.id));
    assert.equal(restored.archived, false);
    assert.equal(restored.config_hash, noRaw.config_hash);
    assert.equal(restored.attempts, noRaw.attempts);
    checks.push(
      "missing receipts disable reparsing; ending requires an explicit decision; rename/archive/restore retain frozen evidence and attempt counts",
    );
  }

  const selected = await selectStage(report.cases.valid);
  await page.getByRole("button", { name: "查看排名快照", exact: true }).click();
  await expect(page.getByLabel("排名 Rating", { exact: true })).toHaveValue(
    "g",
  );
  await page.getByRole("button", { name: "生成工作集", exact: true }).click();
  const derive = page.getByRole("dialog", {
    name: "从排名生成工作集",
    exact: true,
  });
  await derive.getByLabel("排名范围", { exact: true }).selectOption("count");
  await derive.getByLabel("前 N 名", { exact: true }).fill("3");
  await derive.getByRole("button", { name: "预览筛选", exact: true }).click();
  await expect(derive.getByText(/实际将生成 4 张图片/)).toBeVisible({
    timeout: 30000,
  });
  await expect(derive.locator(".aesthetic-preview-images figure")).toHaveCount(
    4,
  );
  await page.screenshot({ path: resolve(run, "derive-preview.png") });
  await derive.getByRole("button", { name: "取消", exact: true }).click();
  await selectStage(selected.id);
  await page.getByRole("button", { name: "新建评审", exact: true }).click();
  const create = page.getByRole("dialog", {
    name: "新建评审阶段",
    exact: true,
  });
  await create
    .getByLabel("候选工作集", { exact: true })
    .selectOption(isolated.config.request.collection_id);
  await create
    .getByLabel("评审模型", { exact: true })
    .selectOption(isolated.config.request.model_id);
  await create
    .getByLabel("审美标准（System Prompt）", { exact: true })
    .selectOption(isolated.config.request.system_prompt_id);
  await create.getByLabel("调用次数上限", { exact: true }).fill("1");
  await create.getByRole("button", { name: "预检输入", exact: true }).click();
  await expect(create.getByText("预检未通过", { exact: true })).toBeVisible();
  await expect(
    create.getByRole("button", { name: "创建并冻结候选", exact: true }),
  ).toBeDisabled();
  await create
    .getByRole("button", { name: "关闭新建评审阶段", exact: true })
    .click();
  checks.push(
    "Top N preview shows real selected images and exact counts; insufficient complete-run budget is blocked before stage creation",
  );
  assert.deepEqual(errors, []);
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify({ passed: true, checks, fixture, errors }, null, 2),
  );
  console.log(
    JSON.stringify({
      passed: true,
      checks: checks.length,
      report: resolve(run, "report.json"),
    }),
  );
} catch (error) {
  if (page)
    await page
      .screenshot({ path: resolve(run, "failure.png") })
      .catch(() => {});
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      { passed: false, checks, error: String(error), errors, fixture },
      null,
      2,
    ),
  );
  console.error(
    JSON.stringify({ passed: false, report: resolve(run, "report.json") }),
  );
  throw error;
} finally {
  await browser?.close();
  if (vite) {
    vite.kill();
    await new Promise((done) => {
      if (vite.exitCode !== null) done();
      else vite.once("exit", done);
    });
  }
  await engine.stop();
}
