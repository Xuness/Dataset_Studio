// Real UI with an isolated engine and headless Edge profile.
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
  ".local/test-runs/smoke-ranking-browse-ui-" + Date.now(),
);
const state = resolve(run, "state");
const url = "http://127.0.0.1:1432";
await mkdir(run, { recursive: true });
await promisify(execFile)(
  "python",
  [resolve(root, "tooling/ranking-fixture.py"), resolve(run, "fixture"), "512"],
  { cwd: root, windowsHide: true },
);
const fixture = JSON.parse(
  await readFile(resolve(run, "fixture/fixture.json"), "utf8"),
);
const engine = new EngineFixture(root, state);
const checks = [],
  errors = [],
  screenshots = [],
  browseRequests = [];
let browser, page, vite, base, artifact;
// Four-member neighbour prefetch is independent of refreshing the displayed page.
const displayedPageReads = () =>
  browseRequests.filter((request) => request.limit > 4).length;
async function shot(name) {
  if (
    name !== "failure" &&
    (await page.locator(".asset-card, .image-canvas").count())
  ) {
    const picture = page.locator(".asset-card img, .image-canvas img").first();
    await expect(picture).toBeVisible({ timeout: 15000 });
    await expect
      .poll(() =>
        picture.evaluate((img) => img.complete && img.naturalWidth > 0),
      )
      .toBe(true);
  }
  await page.screenshot({ path: resolve(run, name + ".png") });
  screenshots.push(name);
}
async function sorted(order) {
  const items = [];
  let cursor;
  do {
    const value = await engine.api(
      base + "/artifacts/" + artifact.id + "/ranking/rows",
      "POST",
      {
        filter: { eligibility: "eligible", order },
        limit: 128,
        ...(cursor ? { cursor } : {}),
      },
    );
    items.push(...value.items);
    cursor = value.next_cursor;
  } while (cursor);
  return items;
}
async function visible(expected) {
  await expect(page.locator(".asset-card")).toHaveCount(expected.length, {
    timeout: 30000,
  });
  await expect
    .poll(
      async () =>
        page.locator(".asset-card .asset-caption > span").allTextContents(),
      { timeout: 30000 },
    )
    .toEqual(expected.map((row) => "Danbooru #" + row.input.post_id));
  const ranks = await page
    .locator(".asset-card .asset-ranking")
    .evaluateAll((nodes) =>
      nodes.map((node) => [
        node.getAttribute("data-main-rank"),
        node.getAttribute("data-rescue-rank"),
      ]),
    );
  assert.deepEqual(
    ranks,
    expected.map((row) => [
      String(row.scores.main_rank ?? "none"),
      String(row.scores.rescue_rank ?? "none"),
    ]),
  );
}
try {
  await engine.start();
  const project = await engine.api("/v1/projects", "POST", {
    name: "排名浏览界面验收",
  });
  base = "/v1/projects/" + project.id;
  const source = await engine.api(base + "/sources", "POST", {
    name: "排名浏览资料",
    kind: "danbooru",
    index_root: fixture.lake,
    media_root: fixture.lake,
  });
  const job = await engine.api(base + "/tools/jobs", "POST", {
    idempotency_key: crypto.randomUUID(),
    scope: {
      project_id: project.id,
      target: {
        kind: "source",
        source_id: source.id,
        revision: source.revision,
      },
    },
    run: {
      operator_id: "danbooru.metarecall",
      operator_version: 1,
      parameters_version: 1,
      parameters: { mode: "rank", cohort_minimum: 8 },
    },
  });
  const finished = await engine.wait(
    base + "/jobs",
    (value) =>
      value.items.some(
        (item) =>
          item.id === job.id && ["succeeded", "failed"].includes(item.status),
      ),
    60000,
  );
  assert.equal(
    finished.items.find((item) => item.id === job.id).status,
    "succeeded",
  );
  artifact = await engine.api(base + "/jobs/" + job.id + "/ranking");
  const workset = await engine.api(
    base + "/artifacts/" + artifact.id + "/ranking/worksets",
    "POST",
    {
      idempotency_key: crypto.randomUUID(),
      name: "评分后的工作集",
      filter: { eligibility: "eligible", order: "main" },
    },
  );
  const main = await sorted("main");
  const rescue = await sorted("rescue");
  await engine.api(base + "/drafts/studio.session/default", "PUT", {
    schema_version: 1,
    expected_revision: 0,
    value: {
      order: "post_id_desc",
      moduleId: "core.browser",
      panels: [],
      scope: { kind: "collection", id: workset.id, name: workset.name },
      focusKey: null,
      view: "grid",
      position: null,
      inspectorTab: "properties",
      managementTarget: null,
    },
  });
  await engine.api(base + "/close", "POST");
  const log = await open(resolve(run, "vite.log"), "a");
  vite = spawn(
    process.execPath,
    [
      resolve(root, "apps/desktop/node_modules/vite/bin/vite.js"),
      "--port",
      "1432",
      "--strictPort",
    ],
    {
      cwd: resolve(root, "apps/desktop"),
      env: { ...process.env, STUDIO_DATA_DIR: state },
      windowsHide: true,
      stdio: ["ignore", log.fd, log.fd],
    },
  );
  await log.close();
  for (let attempt = 0; attempt < 100; attempt++) {
    if (
      (await fetch(url, { signal: AbortSignal.timeout(500) }).catch(() => null))
        ?.ok
    )
      break;
    if (vite.exitCode !== null) throw new Error("Fixture frontend exited");
    await sleep(100);
  }
  browser = await chromium.launch({ channel: "msedge", headless: true });
  const context = await browser.newContext({
    viewport: { width: 1540, height: 1000 },
  });
  await context.route(url + "/__studio/connection", (route) =>
    route.fulfill({ json: engine.connection }),
  );
  await context.route(engine.connection.endpoint + "/**", async (route) => {
    const request = route.request();
    if (
      request.method() === "POST" &&
      request.url().endsWith("/ranking-browse/assets")
    )
      browseRequests.push(JSON.parse(request.postData()));
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
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(url);
  await page.locator(".recent-row").filter({ hasText: project.name }).click();
  await visible(main.slice(0, 48));
  await expect(page.getByLabel("浏览排序", { exact: true })).toHaveValue(
    "ranking:saved",
  );
  await expect(page.getByLabel("排名查看方向")).toHaveValue("asc");
  await shot("01-default-rankings");
  checks.push(
    "an existing workset with an old generic order opens in its saved ranking order and displays scores",
  );

  for (const limit of [12, 96, 48]) {
    await page.getByLabel("每页数量").selectOption(String(limit));
    await visible(main.slice(0, limit));
  }
  checks.push(
    "page-size changes retain the saved ranking and display the correct first members",
  );
  await sleep(200);
  const beforeSelection = displayedPageReads();
  const firstThumb = page.locator(".asset-thumb").first();
  await firstThumb.focus();
  await firstThumb.press("Space");
  await expect(page.locator(".asset-card.selected")).toHaveCount(1);
  await firstThumb.press("Space");
  await expect(page.locator(".asset-card.selected")).toHaveCount(0);
  assert.equal(displayedPageReads(), beforeSelection);
  checks.push(
    "selecting and deselecting a card refreshes its state without rereading the displayed ranking page",
  );

  await page
    .locator(".browser-view .content-bar")
    .getByRole("button", { name: "单图查看", exact: true })
    .click();
  await expect(page.locator(".canvas-caption .asset-ranking")).toHaveAttribute(
    "data-main-rank",
    String(main[0].scores.main_rank),
  );
  await expect(
    page.getByRole("button", { name: "下一张", exact: true }),
  ).toBeEnabled();
  await page.getByRole("button", { name: "下一张", exact: true }).click();
  await expect(page.locator(".canvas-caption .asset-ranking")).toHaveAttribute(
    "data-main-rank",
    String(main[1].scores.main_rank),
  );
  await shot("06-single-image-rankings");
  await page.getByRole("button", { name: "返回网格", exact: true }).click();
  await visible(main.slice(0, 48));
  checks.push(
    "single-image viewing keeps score information while stepping through the ranked page",
  );

  await page.getByLabel("排名查看方向").selectOption("desc");
  const reversedMain = [...main].reverse();
  await visible(reversedMain.slice(0, 48));
  await page
    .locator(".browser-view")
    .getByRole("button", { name: "下一页", exact: true })
    .click();
  await visible(reversedMain.slice(48, 96));
  await page
    .locator(".browser-view")
    .getByRole("button", { name: "上一页", exact: true })
    .click();
  await visible(reversedMain.slice(0, 48));
  checks.push(
    "descending ranking and normal page navigation show the expected consecutive members",
  );

  await page
    .getByLabel("浏览排序", { exact: true })
    .selectOption("ranking:rescue");
  await visible([...rescue].reverse().slice(0, 48));
  await page.getByLabel("排名查看方向").selectOption("asc");
  await visible(rescue.slice(0, 48));
  await shot("02-rescue-order");
  checks.push(
    "view order and direction are independent and switch to actual rescue ranks",
  );

  const target = rescue[Math.floor(rescue.length / 2) + 13];
  const targetId = target.input.post_id;
  const locate = (rows) =>
    rows.findIndex((row) => row.input.asset_id === target.input.asset_id);
  await page.getByLabel("起点 Danbooru ID").fill(targetId);
  await page.getByLabel("起点 Danbooru ID").press("Enter");
  await visible(rescue.slice(locate(rescue), locate(rescue) + 48));
  await page
    .locator(".browser-view")
    .getByRole("button", { name: "下一页", exact: true })
    .click();
  await visible(rescue.slice(locate(rescue) + 48, locate(rescue) + 96));
  await page.getByLabel("排名查看方向").selectOption("desc");
  const reversedRescue = [...rescue].reverse();
  await visible(
    reversedRescue.slice(locate(reversedRescue), locate(reversedRescue) + 48),
  );
  await expect(page.getByLabel("起点 Danbooru ID")).toHaveValue(targetId);
  checks.push(
    "entering a Danbooru ID includes the located image and changing direction keeps that starting ID",
  );

  await page
    .getByLabel("浏览排序", { exact: true })
    .selectOption("ranking:main");
  await visible(
    reversedMain.slice(locate(reversedMain), locate(reversedMain) + 48),
  );
  await page
    .locator(".browser-view")
    .getByRole("button", { name: "下一页", exact: true })
    .click();
  const expectedSecond = reversedMain.slice(
    locate(reversedMain) + 48,
    locate(reversedMain) + 96,
  );
  await visible(expectedSecond);
  await expect
    .poll(
      async () =>
        (await engine.api(base + "/drafts/studio.session/default")).draft?.value
          ?.position?.pageNumber,
    )
    .toBe(2);
  await page.reload();
  await visible(expectedSecond);
  await expect(page.getByLabel("浏览排序", { exact: true })).toHaveValue(
    "ranking:main",
  );
  await expect(page.getByLabel("排名查看方向")).toHaveValue("desc");
  await expect(page.getByLabel("起点 Danbooru ID")).toHaveValue(targetId);
  await shot("03-anchor-after-reload");
  checks.push(
    "order, direction, resolved start and the second page survive a reload",
  );

  await page
    .getByRole("button", { name: workset.name + "更多操作", exact: true })
    .click();
  await page
    .getByRole("menuitem", { name: "重命名与备注…", exact: true })
    .click();
  await page.getByLabel("对象名称", { exact: true }).fill("带排名的候选集");
  await page.getByRole("button", { name: "保存修改", exact: true }).click();
  await expect(page.locator(".content-bar").first()).toContainText(
    "带排名的候选集",
  );
  await visible(expectedSecond);
  checks.push(
    "renaming a ranked workset preserves its current page and starting position",
  );

  await page.getByRole("button", { name: "选择全部范围", exact: true }).click();
  await expect
    .poll(async () => (await engine.api(base + "/selection")).count)
    .toBe(workset.count);
  await page.getByRole("tab", { name: "属性", exact: true }).click();
  await page.locator(".browser-view").click({ position: { x: 30, y: 20 } });
  await page.keyboard.press("Control+z");
  await expect
    .poll(async () => (await engine.api(base + "/selection")).count)
    .toBe(0);
  checks.push(
    "a viewing anchor does not change the full workset selection scope or selection undo",
  );

  await page
    .getByLabel("浏览排序", { exact: true })
    .selectOption("post_id_desc");
  await expect(page.getByLabel("排名查看方向")).toHaveCount(0);
  await expect(page.getByLabel("起点 Danbooru ID")).toHaveCount(0);
  await expect(page.locator(".asset-card .asset-ranking")).toHaveCount(48, {
    timeout: 30000,
  });
  await page
    .getByLabel("浏览排序", { exact: true })
    .selectOption("ranking:saved");
  await page.getByLabel("排名查看方向").selectOption("asc");
  await visible(main.slice(0, 48));
  checks.push(
    "generic ID sorting remains available with score badges, and ranking order can be restored",
  );

  await page.getByLabel("起点 Danbooru ID").fill("-1");
  await page.getByRole("button", { name: "从此图开始", exact: true }).click();
  await expect(
    page.getByText("请填写有效的 Danbooru ID 正整数。", { exact: true }),
  ).toBeVisible();
  await page.getByLabel("起点 Danbooru ID").fill("9223372036854775807");
  await page.getByRole("button", { name: "从此图开始", exact: true }).click();
  await expect(
    page.getByText("当前范围中未找到这个 Danbooru ID，请检查 ID 或筛选范围", {
      exact: true,
    }),
  ).toBeVisible();
  await shot("04-invalid-start");
  await page.getByRole("button", { name: "重置起点", exact: true }).click();
  await visible(main.slice(0, 48));
  checks.push(
    "invalid and missing IDs explain the problem and can return to the full ranking without changing members",
  );

  await page.getByRole("button", { name: "分级 G", exact: true }).click();
  await page.getByRole("button", { name: "应用筛选", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "清除筛选", exact: true }),
  ).toBeVisible();
  await expect
    .poll(
      async () => page.locator(".asset-card .ranking-rating").allTextContents(),
      { timeout: 30000 },
    )
    .toEqual(Array(48).fill("G"));
  await expect(page.getByLabel("浏览排序", { exact: true })).toHaveValue(
    "ranking:saved",
  );
  await shot("05-filtered-ranking");
  for (const limit of [12, 96, 48]) {
    await page.getByLabel("每页数量").selectOption(String(limit));
    await expect(page.locator(".asset-card")).toHaveCount(limit, {
      timeout: 30000,
    });
    await expect
      .poll(() => page.locator(".asset-card .ranking-rating").allTextContents())
      .toEqual(Array(limit).fill("G"));
  }
  await sleep(200);
  const beforeFilteredSelection = displayedPageReads();
  await page.locator(".asset-thumb").first().focus();
  await page.locator(".asset-thumb").first().press("Space");
  await expect(page.locator(".asset-card.selected")).toHaveCount(1);
  await page.locator(".asset-thumb").first().press("Space");
  await expect(page.locator(".asset-card.selected")).toHaveCount(0);
  assert.equal(displayedPageReads(), beforeFilteredSelection);
  checks.push(
    "filtered G results preserve page sizes and selection changes do not restart member reads",
  );
  await page.getByRole("button", { name: "清除筛选", exact: true }).click();
  await visible(main.slice(0, 48));
  checks.push(
    "quick filters within the workset retain ranked viewing and clearing them restores the workset",
  );
  assert.deepEqual(errors, []);
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      {
        passed: true,
        checks,
        errors,
        screenshots,
        ranking_browse_requests: browseRequests.length,
        workset: workset.id,
        anchor_post_id: targetId,
      },
      null,
      2,
    ),
  );
  console.log(JSON.stringify({ passed: true, checks: checks.length, run }));
} catch (error) {
  if (page) await shot("failure").catch(() => {});
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      {
        passed: false,
        checks,
        errors,
        error: String(error),
        screenshots,
        browseRequests,
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
