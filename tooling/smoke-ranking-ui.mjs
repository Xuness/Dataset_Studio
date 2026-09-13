// Real React UI against an isolated fixture engine. No user browser profile is used.
import assert from "node:assert/strict";
import { spawn, execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, readFile, writeFile, open } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath, URL } from "node:url";
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
let holdSubmission = false,
  releaseSubmission;
let jobOverride = null,
  jobsOffline = false;
let holdAction = false,
  releaseAction;
const actionRequests = { submit: 0, cancel: 0, retry: 0 };
let browseReplay = null;
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
    if (browseReplay) {
      const replay = browseReplay;
      const parsed = new URL(request.url());
      if (
        parsed.pathname === base + "/assets" &&
        parsed.searchParams.get("collection_id") === replay.collectionId
      ) {
        const cursor = parsed.searchParams.get("cursor");
        replay.cursors.push(cursor);
        if (replay.released || cursor === "ui-ready")
          return route.fulfill({ headers: cors, json: replay.ready });
        return route.fulfill({
          headers: cors,
          json: {
            ...replay.ready,
            items: [],
            next_cursor:
              replay.mode === "scan" && !cursor ? "ui-scan" : "ui-ready",
            preparing: "正在准备范围排序",
            result_id: replay.mode === "scan" ? null : replay.result.id,
            scan: { scanned: cursor ? 16384 : 8192, total: 20000 },
          },
        });
      }
      if (parsed.pathname === base + "/query-results/" + replay.result.id) {
        replay.polls++;
        return route.fulfill({
          headers: cors,
          json: {
            ...replay.result,
            state:
              replay.mode === "failure"
                ? "failed"
                : replay.polls < 3
                  ? "running"
                  : "ready",
            error:
              replay.mode === "failure" ? "SOURCE_TIMEOUT：范围排序超时" : null,
            processed: 16384,
          },
        });
      }
      if (
        parsed.pathname ===
        base + "/query-results/" + replay.result.id + "/release"
      ) {
        replay.released = true;
        return route.fulfill({
          headers: cors,
          json: { ...replay.result, state: "released" },
        });
      }
    }
    if (path === base + "/tools/jobs" && request.method() === "POST") {
      actionRequests.submit++;
      if (holdSubmission)
        await new Promise((done) => {
          releaseSubmission = done;
        });
    }
    // Deterministic presentation states, confined to this browser's fixture traffic.
    // The actual engine job and published artifact remain unchanged.
    const isJobHistory =
      new URL(request.url()).pathname === base + "/job-history";
    if (
      (path === base + "/jobs" || isJobHistory) &&
      request.method() === "GET"
    ) {
      if (jobsOffline)
        return route.fulfill({
          status: 503,
          headers: cors,
          json: {
            code: "TEST_OFFLINE",
            message: "任务状态接口暂时不可用（界面验收）",
          },
        });
      if (jobOverride && isJobHistory) {
        const result = await engine.api(path);
        assert.ok(result.items.some((item) => item.job.id === jobOverride.id));
        return route.fulfill({
          headers: cors,
          json: {
            ...result,
            items: result.items.map((item) =>
              item.job.id === jobOverride.id
                ? { ...item, job: jobOverride }
                : item,
            ),
          },
        });
      }
      if (jobOverride)
        return route.fulfill({ headers: cors, json: { items: [jobOverride] } });
    }
    for (const action of ["cancel", "retry"]) {
      if (
        jobOverride &&
        path === base + "/jobs/" + jobOverride.id + "/" + action &&
        request.method() === "POST"
      ) {
        actionRequests[action]++;
        if (holdAction)
          await new Promise((done) => {
            releaseAction = done;
          });
        jobOverride = {
          ...jobOverride,
          status: action === "cancel" ? "cancelled" : "queued",
          error: null,
          ...(action === "retry" ? { stage: undefined, completed: 0 } : {}),
        };
        return route.fulfill({ headers: cors, json: jobOverride });
      }
    }
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
  holdSubmission = true;
  await page.getByRole("button", { name: "生成候选集", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "正在提交…", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByText("正在提交排名任务…", { exact: true }),
  ).toBeVisible();
  await expect.poll(() => typeof releaseSubmission).toBe("function");
  assert.equal(actionRequests.submit, 1);
  await page.screenshot({ path: resolve(run, "09-submitting.png") });
  holdSubmission = false;
  releaseSubmission();
  releaseSubmission = null;
  await expect(
    page.locator(".ranking-job").getByText("本次排名已完成", { exact: true }),
  ).toBeVisible({
    timeout: 90000,
  });
  await expect(page.locator(".tasks-panel")).toHaveCount(0);
  await expect(page.locator(".ranking-job-meta")).toContainText("本次执行");
  await page.locator(".ranking-job-timings summary").click();
  await expect(page.locator(".ranking-job-timings")).toContainText(
    "校验评分与名次",
  );
  await expect(page.locator(".ranking-job-timings")).toContainText("G");
  await page.screenshot({ path: resolve(run, "15-phase-timings.png") });
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
    .getByRole("button", { name: "保存筛选为工作集", exact: true })
    .click();
  await page.getByLabel("工作集名称", { exact: true }).fill("界面导出的前75名");
  await expect(page.getByRole("dialog")).toContainText("G 分级");
  await expect(page.getByRole("dialog")).toContainText("前 75 名");
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
  await page.getByRole("button", { name: "打开工作集", exact: true }).click();
  await expect(page.locator(".asset-card")).toHaveCount(48, { timeout: 30000 });
  await expect(page.getByLabel("浏览排序", { exact: true })).toHaveValue(
    "ranking:saved",
  );
  // The historical preparation-state replay exercises the generic ID path.
  await page
    .getByLabel("浏览排序", { exact: true })
    .selectOption("post_id_desc");
  await expect(page.locator(".asset-card")).toHaveCount(48, { timeout: 30000 });
  await expect(
    page.getByRole("button", { name: "保存当前范围", exact: true }),
  ).toBeVisible();
  const savedCollection = collections.find(
    (c) => c.name === "界面导出的前75名",
  );
  const realPage = await engine.api(
    base +
      "/assets?collection_id=" +
      savedCollection.id +
      "&order=post_id_desc&limit=48",
  );
  const replayResult = {
    id: crypto.randomUUID(),
    project_id: project.id,
    state: "queued",
    processed: 0,
    count: 75,
    created_at: String(Date.now()),
    error: null,
  };
  for (const mode of ["scan", "query", "failure"]) {
    browseReplay = {
      mode,
      collectionId: savedCollection.id,
      ready: realPage,
      result: replayResult,
      cursors: [],
      polls: 0,
      released: false,
    };
    await page.getByTitle("刷新当前范围", { exact: true }).click();
    if (mode === "failure") {
      await expect(
        page.getByText("读取超时，可缩小查询范围后重试。", { exact: true }),
      ).toBeVisible({ timeout: 15000 });
      await page.locator(".error-details summary").click();
      await expect(page.locator(".error-details pre")).toContainText(
        "SOURCE_TIMEOUT：范围排序超时",
      );
      assert.deepEqual(browseReplay.cursors, [null]);
      await page.getByRole("button", { name: "重新读取", exact: true }).click();
      await expect(page.locator(".asset-card")).toHaveCount(48, {
        timeout: 15000,
      });
      assert.equal(browseReplay.released, true);
      assert.deepEqual(browseReplay.cursors, [null, null]);
    } else {
      await expect(
        page.getByRole("progressbar", { name: "范围成员定位进度" }),
      ).toBeVisible();
      await expect(page.locator(".asset-card")).toHaveCount(48, {
        timeout: 15000,
      });
      assert.deepEqual(
        browseReplay.cursors,
        mode === "scan" ? [null, "ui-scan", "ui-ready"] : [null, "ui-ready"],
      );
      assert.equal(browseReplay.polls, mode === "scan" ? 0 : 3);
    }
    browseReplay = null;
  }
  checks.push(
    "scope preparation follows bounded scan cursors; fallback polls only its result ID; failure preserves the timeout reason and retry releases the failed result (fixture presentation replay)",
  );
  await page.getByRole("button", { name: "计算工具", exact: true }).click();
  await expect(
    page.getByRole("table", { name: "元数据排名榜单" }),
  ).toBeVisible();
  checks.push(
    "a saved ranking filter opens as a browsable workset and the global range-save action is clearly separate",
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
  const finished = (await engine.api(base + "/jobs")).items.find(
    (j) => j.operator === "danbooru.metarecall",
  );
  assert.equal(finished.status, "succeeded");
  jobOverride = {
    ...finished,
    status: "preparing",
    total: 11493687,
    completed: 11493687,
    stage: { name: "metadata_snapshot", completed: 7629312, total: 11493687 },
  };
  await page.setViewportSize({ width: 1540, height: 1000 });
  const progressCard = page.getByRole("region", {
    name: "排名任务进度",
    exact: true,
  });
  await expect(progressCard).toContainText("66.4%", { timeout: 15000 });
  await expect(
    page.getByRole("button", { name: "排名任务执行中", exact: true }),
  ).toBeDisabled();
  await expect(page.getByLabel("输入范围", { exact: true })).toBeDisabled();
  await expect(
    page.getByLabel("输入范围", { exact: true }).locator("option:checked"),
  ).toContainText("11,493,687");
  await expect(progressCard.getByRole("progressbar")).toHaveAttribute(
    "aria-valuenow",
    /^66\./,
  );
  await page.screenshot({ path: resolve(run, "10-progress-wide.png") });
  await page.getByRole("button", { name: "项目成果", exact: true }).click();
  await expect(page.locator(".task-activity")).toContainText("66.4%");
  await sleep(1100);
  await page.reload();
  await expect(page.locator(".task-activity")).toContainText("66.4%", {
    timeout: 15000,
  });
  await page.getByRole("button", { name: "计算工具", exact: true }).click();
  await expect(progressCard).toContainText("66.4%");
  checks.push(
    "submission acknowledges immediately and only once; current-stage progress, frozen count and duplicate prevention survive module switching and reconnect",
  );
  jobsOffline = true;
  await expect(progressCard).toContainText("状态同步中断", { timeout: 20000 });
  await expect(progressCard).toContainText("66.4%");
  await expect(
    progressCard.getByRole("button", { name: "取消任务", exact: true }),
  ).toBeDisabled();
  await page.screenshot({ path: resolve(run, "11-progress-disconnected.png") });
  jobsOffline = false;
  await expect(progressCard.getByText(/状态同步中断/)).toHaveCount(0, {
    timeout: 15000,
  });
  jobOverride = {
    ...jobOverride,
    status: "running",
    completed: 11493687,
    stage: { name: "publishing", completed: 0, total: 1 },
  };
  await expect(progressCard).toContainText("保存项目成果", {
    timeout: 15000,
  });
  await expect(progressCard.getByRole("progressbar")).not.toHaveAttribute(
    "aria-valuenow",
    /.+/,
  );
  await expect(progressCard).not.toContainText("100.0%");
  await page.locator(".task-activity").click();
  await expect(page.locator(".task-row")).toContainText("保存项目成果");
  await expect(
    page.locator(".task-row").getByRole("progressbar"),
  ).not.toHaveAttribute("aria-valuenow", /.+/);
  await page.screenshot({ path: resolve(run, "12-finalizing-tasks.png") });
  await page.getByRole("button", { name: "查看任务", exact: true }).click();
  await expect(page.locator(".tasks-panel")).toHaveCount(0);
  holdAction = true;
  await progressCard
    .getByRole("button", { name: "取消任务", exact: true })
    .click();
  await expect(
    progressCard.getByRole("button", { name: "正在取消…", exact: true }),
  ).toBeDisabled();
  await expect.poll(() => typeof releaseAction).toBe("function");
  assert.equal(actionRequests.cancel, 1);
  holdAction = false;
  releaseAction();
  releaseAction = null;
  await expect(progressCard).toContainText("排名已取消");
  await progressCard
    .getByRole("button", { name: "重试固定输入", exact: true })
    .click();
  await expect(progressCard).toContainText("排队中");
  assert.equal(actionRequests.retry, 1);
  jobOverride = {
    ...jobOverride,
    status: "failed",
    error: "RANKING_MEMORY_LIMIT：测试内存预算不足（界面验收）",
  };
  await expect(progressCard).toContainText("排名失败", { timeout: 15000 });
  await expect(progressCard).toContainText("测试内存预算不足");
  await expect(progressCard.locator(".job-progress-track > span")).toHaveCSS(
    "width",
    "0px",
  );
  await expect(page.locator(".ranking-inspector")).not.toContainText(
    "本成果的范围与参数已固定",
  );
  await page.setViewportSize({ width: 1000, height: 760 });
  await page.screenshot({ path: resolve(run, "13-failed-narrow.png") });
  assert.ok(
    await page.locator("html").evaluate((e) => e.scrollWidth <= e.clientWidth),
  );
  jobOverride = finished;
  await expect(progressCard).toContainText("本次排名已完成", {
    timeout: 15000,
  });
  await expect(
    progressCard.getByRole("button", { name: "查看结果", exact: true }),
  ).toBeEnabled();
  await progressCard
    .getByRole("button", { name: "查看结果", exact: true })
    .click();
  await expect(
    page.getByRole("table", { name: "元数据排名榜单" }),
  ).toBeVisible();
  await page.screenshot({ path: resolve(run, "14-complete-narrow.png") });
  const actualJobs = (await engine.api(base + "/jobs")).items;
  assert.equal(actualJobs.length, 1);
  assert.equal(actualJobs[0].status, "succeeded");
  checks.push(
    "offline state preserves the last progress; finalization never reports whole-job completion; cancel/retry acknowledge once; failure and completion have working actions at narrow width (fixture presentation replay)",
  );

  // V2 is tested against the real engine after the v1 presentation-replay checks.
  jobOverride = null;
  await page.setViewportSize({ width: 1540, height: 1000 });
  await page.getByRole("button", { name: "参数配置", exact: true }).click();
  const scheme = page.getByRole("combobox", { name: "筛选方案", exact: true });
  await scheme.selectOption("v2");
  await page
    .getByRole("spinbutton", { name: "G 年代榜权重", exact: true })
    .fill("0.65");
  await page
    .getByRole("spinbutton", { name: "跨年羽化半宽（天）", exact: true })
    .fill("45");
  await scheme.selectOption("v1");
  await expect(
    page.getByRole("spinbutton", { name: "G 年代榜权重", exact: true }),
  ).toHaveCount(0);
  await scheme.selectOption("v2");
  await expect(
    page.getByRole("spinbutton", { name: "G 年代榜权重", exact: true }),
  ).toHaveValue("0.65");
  await expect(
    page.getByRole("spinbutton", { name: "跨年羽化半宽（天）", exact: true }),
  ).toHaveValue("45");
  await page
    .getByRole("spinbutton", { name: "年代有效样本下限", exact: true })
    .fill("2");
  await page.getByRole("radio", { name: "仅计算排名", exact: true }).check();
  await engine.wait(
    base + "/drafts/core.tools/ranking",
    (response) =>
      response.draft?.value.parameters.v2?.feather_days === 45 &&
      response.draft?.value.parameters.v2?.minimum_effective === 2,
    15000,
  );
  await page.screenshot({ path: resolve(run, "15-v2-config.png") });
  await page.reload();
  await expect(
    page.getByRole("combobox", { name: "筛选方案", exact: true }),
  ).toHaveValue("v2", { timeout: 15000 });
  await expect(
    page.getByRole("spinbutton", { name: "G 年代榜权重", exact: true }),
  ).toHaveValue("0.65");
  await page.getByRole("button", { name: "计算排名", exact: true }).click();
  const v2Jobs = await engine.wait(
    base + "/jobs",
    (response) =>
      response.items.some(
        (j) =>
          j.operator === "danbooru.metarecall_v2" &&
          ["succeeded", "failed"].includes(j.status),
      ),
    90000,
  );
  const v2Job = v2Jobs.items.find(
    (j) => j.operator === "danbooru.metarecall_v2",
  );
  assert.equal(v2Job.status, "succeeded", JSON.stringify(v2Job));
  await expect(page.locator(".ranking-job")).toContainText("本次排名已完成", {
    timeout: 15000,
  });
  await page
    .locator(".ranking-job")
    .getByRole("button", { name: "查看结果", exact: true })
    .click();
  await page
    .getByRole("combobox", { name: "排序依据", exact: true })
    .selectOption("direct");
  await expect(
    page.getByRole("columnheader", { name: "直算排名", exact: true }),
  ).toBeVisible();
  await page
    .getByRole("combobox", { name: "排序依据", exact: true })
    .selectOption("fused");
  await expect(
    page.getByRole("columnheader", { name: "融合排名", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "统计诊断", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "v2 · 元数据多阶段", exact: true }),
  ).toBeVisible();
  await page.screenshot({ path: resolve(run, "16-v2-diagnostics.png") });
  await page.setViewportSize({ width: 1000, height: 760 });
  assert.ok(
    await page.locator("html").evaluate((e) => e.scrollWidth <= e.clientWidth),
  );
  await page.screenshot({ path: resolve(run, "17-v2-diagnostics-narrow.png") });
  await page.getByRole("button", { name: "参数配置", exact: true }).click();
  await page
    .getByRole("combobox", { name: "筛选方案", exact: true })
    .selectOption("v1");
  await expect(
    page.getByRole("spinbutton", { name: "G 年代榜权重", exact: true }),
  ).toHaveCount(0);
  checks.push(
    "v1/v2 switch preserves v2 configuration across reload; real v2 submission exposes direct/fused orders and year diagnostics at wide and narrow widths",
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
  releaseSubmission?.();
  releaseAction?.();
  await browser?.close();
  vite?.kill();
  await engine.stop();
}
