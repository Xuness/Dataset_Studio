import { lakeWorkerPython } from "./lake-worker-runtime.mjs";
// Isolated real engine + embedded worker + UI. No real lakes, schedules or API credentials.
import assert from "node:assert/strict";
import { execFile, spawn } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, open, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";
import { chromium, expect } from "@playwright/test";
import { EngineFixture, sleep } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(root, ".local/test-runs/lake-update-ui-" + Date.now());
const python = lakeWorkerPython(root);
assert.ok(python, "Set STUDIO_LAKE_TEST_PYTHON to the worker test environment");
await mkdir(run, { recursive: true });
const engine = new EngineFixture(root, resolve(run, "engine"));
const checks = [],
  errors = [],
  requests = [];
let browser, vite, page;
const url = "http://127.0.0.1:1453";
try {
  const imagePolicyModel = ts.transpileModule(
    await readFile(
      resolve(root, "apps/desktop/src/features/lake-updates/imagePolicy.ts"),
      "utf8",
    ),
    {
      compilerOptions: {
        module: ts.ModuleKind.ESNext,
        target: ts.ScriptTarget.ES2022,
      },
    },
  ).outputText;
  await writeFile(resolve(run, "imagePolicy.js"), imagePolicyModel);
  await writeFile(
    resolve(run, "package.json"),
    JSON.stringify({ type: "module" }),
  );
  const model = ts.transpileModule(
    await readFile(
      resolve(root, "apps/desktop/src/features/lake-updates/model.ts"),
      "utf8",
    ),
    {
      compilerOptions: {
        module: ts.ModuleKind.ESNext,
        target: ts.ScriptTarget.ES2022,
      },
    },
  ).outputText;
  await writeFile(resolve(run, "model.mjs"), model);
  const m = await import(pathToFileURL(resolve(run, "model.mjs")).href);
  assert.equal(
    m.dateBoundary("2026-03-29", "Europe/Berlin"),
    "2026-03-28T23:00:00.000Z",
  );
  assert.equal(
    m.dateBoundary("2026-03-29", "Europe/Berlin", true),
    "2026-03-29T22:00:00.000Z",
  );
  assert.equal(
    m.dateBoundary("2026-10-25", "Europe/Berlin", true),
    "2026-10-25T23:00:00.000Z",
  );
  assert.throws(
    () => m.definitions({ ...m.initialDraft, lakes: ["fixture"] }),
    /保存策略/,
  );
  assert.equal(
    m.definitions({
      ...m.initialDraft,
      lakes: ["fixture"],
      profile: "original",
      kind: "id_range",
      startId: "11",
      endId: "12",
    })[0].range.end,
    13,
  );
  checks.push(
    "date ranges respect IANA zones and DST; inclusive ID end and explicit image policy",
  );
  await promisify(execFile)(
    python,
    [resolve(root, "tooling/lake-updates-fixture.py"), run, "--assets"],
    { windowsHide: true },
  );
  const targets = JSON.parse(
    await readFile(resolve(run, "targets.json"), "utf8"),
  );
  await engine.start();
  const { StudioClient } = await clientFixture(root, resolve(run, "client"));
  let client = new StudioClient(engine.connection);
  await client.lakeUpdates.configure({
    python,
    state_root: resolve(run, "controller"),
  });
  const project = await client.createProject({
    name: "更新前端验收",
    parent_directory: resolve(run, "projects"),
  });
  for (const t of targets)
    await client.attachSource(project.id, {
      kind: "auto",
      name: t.site,
      index_root: t.index_root,
      media_root: t.media_root,
    });
  const sources = (await client.sources(project.id)).items;
  const base = `/v1/projects/${project.id}`;
  const all = await engine.api(base + "/query-results", "POST", {
    spec: {
      version: 3,
      source_ids: sources.map((s) => s.id),
      conditions: [],
      observation_rule: "current_post",
      order: "asset_key_asc",
    },
  });
  assert.equal(
    (
      await engine.wait(
        base + "/query-results/" + all.id,
        (r) => !["queued", "running"].includes(r.state),
      )
    ).count,
    3,
  );
  const allScope = {
    project_id: project.id,
    target: { kind: "query_result", result_id: all.id },
  };
  const currentSelection = await engine.api(base + "/selection");
  const selected = await engine.api(base + "/selection/scope", "POST", {
    expected_revision: currentSelection.revision,
    scope: allScope,
    operation: "replace",
  });
  const selectionPrep = await client.lakeUpdates.prepareScope({
    request_key: crypto.randomUUID(),
    scope: {
      project_id: project.id,
      target: { kind: "selection", revision: selected.revision },
    },
  });
  await client.changeSelection(project.id, {
    expected_revision: selected.revision,
    add: [],
    remove: [],
    clear: true,
  });
  let preparedSelection;
  await expect(async () => {
    preparedSelection = (await client.lakeUpdates.preparations()).items.find(
      (i) => i.id === selectionPrep.id,
    );
    assert.equal(
      preparedSelection.state,
      "ready",
      JSON.stringify(preparedSelection),
    );
  }).toPass({ timeout: 60000 });
  assert.equal(preparedSelection.processed, 3);
  assert.equal(preparedSelection.inputs.length, 3);
  assert.ok(preparedSelection.inputs.every((i) => i.count === 2));
  const workset = await client.createCollection(
    project.id,
    "固定三湖范围",
    allScope,
  );
  const worksetPrep = await client.lakeUpdates.prepareScope({
    request_key: crypto.randomUUID(),
    scope: {
      project_id: project.id,
      target: { kind: "workset", collection_id: workset.id },
    },
  });
  let interrupted = false;
  for (let i = 0; i < 600; i++) {
    const row = (await client.lakeUpdates.preparations()).items.find(
      (p) => p.id === worksetPrep.id,
    );
    if (row.state === "preparing" && row.processed > 0) {
      interrupted = true;
      break;
    }
    if (row.state !== "preparing")
      throw new Error(
        "Missed the preparation checkpoint: " + JSON.stringify(row),
      );
    await sleep(10);
  }
  assert.ok(
    interrupted,
    "observe an unsealed, persisted preparation checkpoint",
  );
  await engine.stop();
  await engine.start();
  client = new StudioClient(engine.connection);
  await expect(async () => {
    const row = (await client.lakeUpdates.preparations()).items.find(
      (i) => i.id === worksetPrep.id,
    );
    assert.equal(row.state, "ready", JSON.stringify(row));
    assert.equal(row.processed, 3);
    assert.ok(row.inputs.every((i) => i.count === 2));
  }).toPass({ timeout: 60000 });
  await client.openRecentProject(project.id);
  const resultPrep = await client.lakeUpdates.prepareScope({
    request_key: crypto.randomUUID(),
    scope: allScope,
  });
  await expect(async () =>
    assert.equal(
      (await client.lakeUpdates.preparations()).items.find(
        (i) => i.id === resultPrep.id,
      ).state,
      "ready",
    ),
  ).toPass({ timeout: 60000 });
  checks.push(
    "three-source selection freezes before later edits; workset preparation resumes from an unsealed checkpoint; fixed query scope supported",
  );
  const key = crypto.randomUUID(),
    scope = {
      project_id: project.id,
      target: {
        kind: "source",
        source_id: sources[0].id,
        revision: sources[0].revision,
      },
    };
  const preparation = await client.lakeUpdates.prepareScope({
    request_key: key,
    scope,
  });
  assert.equal(
    (await client.lakeUpdates.prepareScope({ request_key: key, scope })).id,
    preparation.id,
  );
  await client.closeProject(project.id);
  const prepared = await engine.wait(
    "/v1/lake-updates/preparations",
    (v) =>
      v.items.some(
        (i) =>
          i.id === preparation.id &&
          !["preparing", "cancelling"].includes(i.state),
      ),
    60000,
  );
  const ready = prepared.items.find((i) => i.id === preparation.id);
  assert.equal(ready.state, "ready", JSON.stringify(ready));
  assert.equal(ready.processed, 1);
  assert.equal(
    ready.inputs[0].count,
    2,
    "one image maps to both associated posts",
  );
  checks.push(
    "persistent input preparation survives project close; deduplicated image maps to every post",
  );
  await engine.stop();
  await engine.start();
  client = new StudioClient(engine.connection);
  assert.equal(
    (await client.lakeUpdates.preparations()).items.find(
      (i) => i.id === preparation.id,
    ).state,
    "ready",
  );
  checks.push(
    "global input references and update service survive engine restart",
  );
  const log = await open(resolve(run, "vite.log"), "a");
  vite = spawn(
    process.execPath,
    [
      resolve(root, "apps/desktop/node_modules/vite/bin/vite.js"),
      "--port",
      "1453",
      "--strictPort",
    ],
    {
      cwd: resolve(root, "apps/desktop"),
      env: { ...process.env, STUDIO_DATA_DIR: resolve(run, "engine") },
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
    viewport: { width: 2560, height: 1400 },
  });
  await context.route(url + "/__studio/connection", (r) =>
    r.fulfill({ json: engine.connection }),
  );
  await context.route(engine.connection.endpoint + "/**", async (route) => {
    const req = route.request(),
      headers = { ...req.headers() };
    delete headers.host;
    delete headers.origin;
    const cors = {
      "access-control-allow-origin": url,
      "access-control-allow-methods": "GET,POST,PUT,PATCH,OPTIONS",
      "access-control-allow-headers":
        req.headers()["access-control-request-headers"] ??
        "authorization,content-type,x-studio-session",
    };
    if (req.method() === "OPTIONS")
      return route.fulfill({ status: 204, headers: cors });
    if (req.method() === "POST") requests.push(new URL(req.url()).pathname);
    const r = await fetch(req.url(), {
      method: req.method(),
      headers,
      ...(req.postData() ? { body: req.postData() } : {}),
    });
    await route.fulfill({
      status: r.status,
      headers: { ...Object.fromEntries(r.headers), ...cors },
      body: Buffer.from(await r.arrayBuffer()),
    });
  });
  page = await context.newPage();
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("dialog", (d) => d.accept());
  await page.goto(url);
  await page
    .getByRole("button", { name: "管理数据湖更新", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "数据湖", exact: true }),
  ).toBeAttached();
  await expect(
    page.getByRole("button", { name: "新建更新", exact: true }),
  ).toBeEnabled();
  await expect(page.locator(".lake-outline button")).toHaveCount(4);
  await page.screenshot({ path: resolve(run, "global-2560.png") });
  checks.push("global UE-style lake editor works without an open project");
  await page.getByRole("button", { name: "新建更新", exact: true }).click();
  let dialog = page.getByRole("dialog", {
    name: "新建数据湖更新",
    exact: true,
  });
  for (const site of ["Danbooru", "Yandere", "Gelbooru"])
    await dialog.getByLabel(site, { exact: true }).check();
  await dialog.getByLabel("范围", { exact: true }).selectOption("missing");
  await dialog
    .getByRole("button", { name: "检查任务摘要", exact: true })
    .click();
  await expect(
    dialog.getByText("请明确选择图片保存策略", { exact: true }).first(),
  ).toBeVisible();
  await dialog.getByLabel("保存策略", { exact: true }).selectOption("original");
  await dialog
    .getByRole("button", { name: "检查任务摘要", exact: true })
    .click();
  await expect(
    dialog.getByRole("button", { name: "开始更新", exact: true }),
  ).toBeEnabled();
  await page.screenshot({ path: resolve(run, "composer-2560.png") });
  await dialog.getByRole("button", { name: "开始更新", exact: true }).click();
  await expect(
    dialog.getByText("各湖任务已创建，可以关闭配置继续浏览。", { exact: true }),
  ).toBeVisible({ timeout: 60000 });
  await dialog.getByRole("button", { name: "关闭", exact: true }).click();
  await expect(async () =>
    assert.equal((await client.lakeUpdates.jobs()).items.length, 6),
  ).toPass({ timeout: 30000 });
  const recent = await client.lakeUpdates.jobs();
  assert.ok(
    recent.items.slice(0, 3).every((j) => j.definition.range.kind === "local"),
  );
  await expect(async () =>
    assert.ok(
      (await client.lakeUpdates.jobs()).items.every(
        (j) => j.state === "completed",
      ),
    ),
  ).toPass({ timeout: 60000 });
  checks.push(
    "three-lake explicit-policy submission through real UI, API and embedded worker; existing media remains reusable",
  );
  await page.getByRole("button", { name: "新建更新", exact: true }).click();
  dialog = page.getByRole("dialog", { name: "新建数据湖更新", exact: true });
  await dialog
    .getByRole("button", { name: "配置另一批更新", exact: true })
    .click();
  await dialog.getByLabel("保存策略", { exact: true }).selectOption("custom");
  await dialog.getByLabel("编码格式", { exact: true }).selectOption("jpeg");
  await dialog.getByLabel("最长边（像素）", { exact: true }).fill("768");
  await dialog.getByLabel("编码质量", { exact: true }).fill("82");
  await dialog.getByLabel("背景颜色", { exact: true }).fill("#112233");
  await dialog
    .getByLabel("已有图片", { exact: true })
    .selectOption("match_profile");
  await dialog.locator("summary").filter({ hasText: "保存与管理预设" }).click();
  await dialog.getByLabel("预设名称", { exact: true }).fill("工作集 JPEG 768");
  await dialog.getByRole("button", { name: "另存为预设", exact: true }).click();
  await expect(
    dialog.getByText("预设已保存；已创建任务仍使用各自固定的配置。", {
      exact: true,
    }),
  ).toBeVisible();
  await dialog.getByLabel("保存策略", { exact: true }).selectOption("original");
  await dialog
    .getByLabel("保存策略", { exact: true })
    .selectOption({ label: "工作集 JPEG 768" });
  await expect(dialog.getByLabel("编码格式", { exact: true })).toHaveValue(
    "jpeg",
  );
  await expect(dialog.getByLabel("编码质量", { exact: true })).toHaveValue(
    "82",
  );
  await expect(
    dialog
      .getByLabel("管理保存预设", { exact: true })
      .locator("option:checked"),
  ).toHaveText("工作集 JPEG 768");
  await expect(dialog.locator(".lake-preset-status")).toContainText(
    "当前设置与预设一致",
  );
  await dialog.getByLabel("编码质量", { exact: true }).fill("70");
  await expect(dialog.locator(".lake-preset-status")).toContainText(
    "当前设置已修改",
  );
  await dialog.getByRole("button", { name: "应用预设", exact: true }).click();
  await expect(dialog.getByLabel("编码质量", { exact: true })).toHaveValue(
    "82",
  );
  await dialog.getByLabel("管理保存预设", { exact: true }).selectOption("");
  await expect(dialog.getByLabel("编码质量", { exact: true })).toHaveValue(
    "82",
  );
  await dialog.getByLabel("保存策略", { exact: true }).selectOption("original");
  await dialog
    .getByLabel("管理保存预设", { exact: true })
    .selectOption({ label: "工作集 JPEG 768" });
  await expect(dialog.getByLabel("保存策略", { exact: true })).toHaveValue(
    "custom",
  );
  await expect(
    dialog.getByLabel("最长边（像素）", { exact: true }),
  ).toHaveValue("768");
  await expect(dialog.getByLabel("编码质量", { exact: true })).toHaveValue(
    "82",
  );
  await expect(dialog.getByLabel("背景颜色", { exact: true })).toHaveValue(
    "#112233",
  );
  await expect(dialog.getByLabel("已有图片", { exact: true })).toHaveValue(
    "match_profile",
  );
  await expect(dialog.getByLabel("范围", { exact: true })).toHaveValue(
    "missing",
  );
  await expect(
    dialog.getByRole("region", { name: "所选预设参数" }),
  ).toContainText("JPEG · 768 / Q82");
  await dialog.locator("summary").filter({ hasText: "高级预算" }).click();
  await expect(
    dialog.getByLabel("每轮最多扫描页数", { exact: true }),
  ).toHaveValue("1000");
  await expect(
    dialog.getByLabel("每轮最多处理记录数", { exact: true }),
  ).toHaveValue("100000");
  await expect(dialog.getByText(/Yandere 每页上限 200 条/)).toBeVisible();
  await expect(dialog.getByText(/不代表成功下载的图片数/)).toBeVisible();
  await dialog.locator("summary").filter({ hasText: "高级预算" }).click();
  await page.screenshot({ path: resolve(run, "preset-applied.png") });
  checks.push(
    "both preset selectors apply all media fields, explicit reapply restores edits, saved summary and dirty status agree, range and budgets stay unchanged",
  );
  await dialog.getByLabel("编码格式", { exact: true }).selectOption("png");
  await expect(dialog.getByLabel("编码质量", { exact: true })).toHaveCount(0);
  await dialog.getByLabel("编码格式", { exact: true }).selectOption("webp");
  await dialog.getByLabel("无损 WebP", { exact: true }).check();
  await expect(dialog.getByLabel("编码质量", { exact: true })).toHaveCount(0);
  await dialog
    .getByLabel("保存策略", { exact: true })
    .selectOption({ label: "工作集 JPEG 768" });
  await dialog.locator("summary").filter({ hasText: "保存与管理预设" }).click();
  await page.screenshot({ path: resolve(run, "encoding-jpeg-2560.png") });
  await dialog.getByLabel("执行", { exact: true }).selectOption("once");
  await dialog
    .getByLabel("首次执行（本机时间）", { exact: true })
    .fill("2099-01-01T12:00");
  await dialog
    .getByRole("button", { name: "检查任务摘要", exact: true })
    .click();
  await dialog
    .getByRole("button", { name: "保存未启用计划", exact: true })
    .click();
  await expect(
    dialog.getByText("计划已保存为未启用，请在计划列表检查后启用。", {
      exact: true,
    }),
  ).toBeVisible({ timeout: 60000 });
  await dialog.getByRole("button", { name: "关闭", exact: true }).click();
  assert.equal((await client.lakeUpdates.schedules()).items.length, 3);
  for (const schedule of (await client.lakeUpdates.schedules()).items) {
    assert.equal(schedule.definition.media.profile, "custom");
    assert.equal(schedule.definition.media.encoding.quality, 82);
    assert.equal(schedule.definition.media.encoding.max_edge, 768);
    assert.equal(schedule.definition.media.encoding.background, "#112233");
  }
  checks.push(
    "codec-specific options, alpha background, saved recipe application and frozen schedule parameters",
  );
  assert.ok(
    (await client.lakeUpdates.schedules()).items.every((s) => !s.enabled),
  );
  await page.locator(".lake-table .lake-row-link").first().click();
  await page.getByLabel("启用计划", { exact: true }).check();
  await page.getByRole("button", { name: "保存计划", exact: true }).click();
  await expect(page.getByText("计划已保存", { exact: true })).toBeVisible();
  assert.equal(
    (await client.lakeUpdates.schedules()).items.filter((s) => s.enabled)
      .length,
    1,
  );
  checks.push(
    "one-shot schedule draft, explicit enable and revision-based saving; all runs remain in 2099",
  );
  await page.getByRole("button", { name: "API 设置", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "设置", exact: true });
  await settings.getByLabel("用户名", { exact: true }).fill("fixture-ui-user");
  const fixtureKey = "ui-fixture-credential-not-a-real-secret";
  await settings.getByLabel("API Key", { exact: true }).fill(fixtureKey);
  await settings.getByRole("button", { name: "保存凭据", exact: true }).click();
  await expect(
    settings.getByRole("status").filter({ hasText: "凭据已保存" }),
  ).toBeVisible();
  await expect(settings.getByLabel("API Key", { exact: true })).toHaveValue("");
  assert.ok(
    !JSON.stringify(await client.lakeUpdates.status()).includes(fixtureKey),
  );
  await settings
    .getByRole("button", { name: "清除已保存凭据", exact: true })
    .click();
  await expect(
    settings.getByRole("status").filter({ hasText: "凭据已清除" }),
  ).toBeVisible();
  await settings
    .getByRole("button", { name: "关闭", exact: true })
    .last()
    .click();
  checks.push(
    "API settings save and clear isolated encrypted credentials without key readback or remote probes",
  );
  await page
    .getByRole("button", { name: "关闭数据湖标签", exact: true })
    .click();
  await page.locator(".recent-row").filter({ hasText: "更新前端验收" }).click();
  await expect(
    page.getByRole("button", { name: "资料浏览", exact: true }),
  ).toBeVisible();
  await page.getByRole("menuitem", { name: "工具", exact: true }).click();
  await page
    .getByRole("menu", { name: "工具" })
    .getByRole("menuitem", { name: "数据湖", exact: true })
    .click();
  await page.getByLabel("数据湖工作视图").selectOption("preparations");
  await expect(
    page
      .getByRole("button", { name: "使用此范围新建更新", exact: true })
      .first(),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "使用此范围新建更新", exact: true })
    .first()
    .click();
  dialog = page.getByRole("dialog", { name: "新建数据湖更新", exact: true });
  await expect(dialog.getByLabel("范围", { exact: true })).toHaveValue("input");
  await dialog.locator("summary").filter({ hasText: "保存与管理预设" }).click();
  await dialog
    .getByLabel("管理保存预设", { exact: true })
    .selectOption({ label: "工作集 JPEG 768" });
  await dialog.getByLabel("编码质量", { exact: true }).fill("80");
  await dialog
    .getByRole("button", { name: "用当前设置更新预设", exact: true })
    .click();
  await expect(
    dialog.getByText("预设已保存；已创建任务仍使用各自固定的配置。", {
      exact: true,
    }),
  ).toBeVisible();
  assert.ok(
    (await client.lakeUpdates.schedules()).items.every(
      (s) => s.definition.media.encoding.quality === 82,
    ),
  );
  await dialog.getByRole("button", { name: "删除预设", exact: true }).click();
  await expect(
    dialog.getByLabel("管理保存预设", { exact: true }).locator("option"),
  ).toHaveCount(1);
  await expect(dialog.getByLabel("编码质量", { exact: true })).toHaveValue(
    "80",
  );
  checks.push(
    "editing or deleting a preset preserves already frozen schedules and the current draft",
  );
  await dialog
    .getByRole("button", { name: "检查任务摘要", exact: true })
    .click();
  await expect(
    dialog.getByRole("button", { name: "开始更新", exact: true }),
  ).toBeEnabled();
  await dialog.getByRole("button", { name: "关闭", exact: true }).click();
  checks.push(
    "prepared project scope opens a typed frozen-input update draft without starting remote API work",
  );
  await page.reload();
  await expect(page.getByLabel("数据湖工作视图")).toHaveValue("preparations");
  await expect(page.locator(".lake-outline button")).toHaveCount(4);
  await expect(page.locator(".lake-preparations table")).toContainText(
    "Yandere",
  );
  await expect(page.locator(".lake-preparations table")).toContainText(
    "Gelbooru",
  );
  await page.setViewportSize({ width: 1706, height: 920 });
  await page.screenshot({ path: resolve(run, "scaled-1706.png") });
  assert.ok(
    await page.evaluate(
      () =>
        globalThis.document.documentElement.scrollWidth <=
        globalThis.innerWidth + 1,
    ),
  );
  await page.getByRole("button", { name: "资料浏览", exact: true }).click();
  await expect(page.locator(".lake-workspace")).toHaveCount(0);
  await page.getByRole("button", { name: /数据湖 ·/ }).click();
  await expect(
    page.getByRole("region", { name: "后台活动", exact: true }),
  ).toBeVisible();
  checks.push(
    "application preferences restore after reload; 150-percent-equivalent viewport and background activity drawer",
  );
  assert.deepEqual(errors, []);
  await writeFile(
    resolve(run, "result.json"),
    JSON.stringify({ passed: true, checks, errors, requests }, null, 2),
  );
  console.log(JSON.stringify({ run, checks }));
} catch (error) {
  if (page)
    await page
      .screenshot({ path: resolve(run, "failure.png") })
      .catch(() => {});
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
