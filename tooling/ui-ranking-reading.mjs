import assert from "node:assert/strict";
import { resolve } from "node:path";
import { expect } from "@playwright/test";

const tab = (page, name) => page.getByRole("tab", { name, exact: true });
const viewer = (page) => page.locator(".ranking-full-image");
const selected = (page) =>
  page.locator('.ranking-image-tile[aria-pressed="true"]');
async function saved(page) {
  await expect(page.locator(".status-bar")).toContainText("项目已保存");
}
export async function checkRankingReading(
  page,
  { engine, rootPath, run, checks },
) {
  await page
    .getByLabel("美学工作视图", { exact: true })
    .selectOption("ranking");
  await tab(page, "详情").click();
  await page.getByLabel("排名 Rating", { exact: true }).selectOption("g");
  await page.getByLabel("排名每页图片数").selectOption("12");
  const size = page.getByLabel("排名缩略图大小");
  await size.focus();
  await size.press("End");
  await expect(size).toHaveValue("320");
  const tile = page.locator(".ranking-image-tile").first();
  assert.ok((await tile.boundingBox()).width >= 320);
  await size.press("Home");
  await size.press("ArrowRight");
  await expect(size).toHaveValue("144");
  await saved(page);
  await page.reload();
  await expect(size).toHaveValue("144");
  checks.push(
    "thumbnail density is adjustable independently of bounded page size and persists after reload",
  );

  await page.locator(".ranking-image-tile").last().click();
  const last = await selected(page).getAttribute("aria-label");
  await selected(page).press("Enter");
  await expect(viewer(page)).toBeVisible();
  const before = await viewer(page).getAttribute("aria-label");
  await viewer(page).press("ArrowRight");
  await expect(page.locator(".ranking-pagebar")).toContainText("第 2 页");
  await expect(viewer(page)).not.toHaveAttribute("aria-label", before);
  const second = await viewer(page).getAttribute("aria-label");
  await viewer(page).press("ArrowLeft");
  await expect(page.locator(".ranking-pagebar")).toContainText("第 1 页");
  await expect(viewer(page)).toHaveAttribute("aria-label", before);
  await viewer(page).press("Escape");
  await expect(selected(page)).toHaveAttribute("aria-label", last);
  await expect(selected(page)).toBeFocused();
  await selected(page).press("Home");
  await expect(page.locator(".ranking-image-tile").first()).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await size.focus();
  await size.press("End");
  await selected(page).press("ArrowDown");
  await expect(page.locator(".ranking-image-tile").first()).toHaveAttribute(
    "aria-pressed",
    "false",
  );
  checks.push(
    "keyboard navigation follows grid columns, crosses pages in both directions and returns focus to the selected image",
  );

  await page.setViewportSize({ width: 1280, height: 720 });
  await size.focus();
  await size.press("End");
  const scroller = page.locator(".ranking-image-scroll");
  await scroller.hover();
  await page.mouse.wheel(0, 650);
  await expect
    .poll(() => scroller.evaluate((e) => e.scrollTop))
    .toBeGreaterThan(100);
  const visibleTile = page.locator(".ranking-image-tile").nth(4);
  await visibleTile.click();
  const scrollTop = await scroller.evaluate((e) => e.scrollTop);
  await visibleTile.press("Enter");
  const viewport = page.locator(".ranking-full-image .zoom-viewport");
  await viewport.hover();
  await page.mouse.wheel(0, -220);
  await expect
    .poll(() => viewport.getAttribute("data-zoom").then(Number))
    .toBeGreaterThan(1);
  const box = await viewport.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(
    box.x + box.width / 2 + 35,
    box.y + box.height / 2 + 30,
  );
  await page.mouse.up();
  await expect
    .poll(() => viewport.getAttribute("data-pan-x").then(Number))
    .toBeGreaterThan(0);
  await page.getByRole("button", { name: "图片适应窗口", exact: true }).click();
  await expect(viewport).toHaveAttribute("data-zoom", "1");
  await viewer(page).press("Escape");
  await expect
    .poll(async () =>
      Math.abs((await scroller.evaluate((e) => e.scrollTop)) - scrollTop),
    )
    .toBeLessThanOrEqual(2);
  await saved(page);
  await page.reload();
  await expect
    .poll(async () =>
      Math.abs((await scroller.evaluate((e) => e.scrollTop)) - scrollTop),
    )
    .toBeLessThanOrEqual(2);
  checks.push(
    "wheel zoom and pointer pan stay in the image viewport; grid scroll position survives large-image return and application reload",
  );

  await page.setViewportSize({ width: 2560, height: 1440 });
  await size.focus();
  await size.press("Home");
  await page.locator(".ranking-image-tile").last().click();
  await selected(page).press("Enter");
  await tab(page, "保护复核").click();
  const reason = page.getByLabel("复核理由", { exact: true });
  await page.getByLabel("复核人", { exact: true }).fill("连续复核 fixture");
  const reasonA = "图 A 的草稿 " + Date.now();
  await reason.fill(reasonA);
  await reason.press("ArrowLeft");
  await expect(viewer(page)).toHaveAttribute("aria-label", before);
  await page.getByRole("button", { name: "下一张图片", exact: true }).click();
  await expect(viewer(page)).toHaveAttribute("aria-label", second);
  await expect(reason).toHaveValue("");
  await reason.fill("图 B 的独立草稿");
  await page.getByRole("button", { name: "上一张图片", exact: true }).click();
  await expect(reason).toHaveValue(reasonA);
  await tab(page, "详情").click();
  await tab(page, "保护复核").click();
  await expect(reason).toHaveValue(reasonA);
  await page
    .getByRole("button", { name: "保护复核面板更多操作", exact: true })
    .click();
  await page.getByRole("menuitem", { name: "停靠到底部", exact: true }).click();
  await expect(reason).toHaveValue(reasonA);
  await page
    .getByRole("button", { name: "保护复核面板更多操作", exact: true })
    .click();
  await page.getByRole("menuitem", { name: "停靠到右侧", exact: true }).click();
  await saved(page);
  await page.reload();
  await expect(reason).toHaveValue(reasonA);
  await expect(viewer(page)).toHaveAttribute("aria-label", before);
  checks.push(
    "per-image review drafts survive image navigation, panel switching, docking and reload; typing arrows never changes the selected image",
  );

  const requests = [];
  let dropped = false;
  const reviewUrl = engine.connection.endpoint + rootPath + "/analysis/reviews";
  await page.route(reviewUrl, async (route) => {
    const body = JSON.parse(route.request().postData());
    requests.push(body);
    if (!dropped) {
      dropped = true;
      await engine.api(rootPath + "/analysis/reviews", "POST", body);
      await route.abort("failed");
    } else await route.fallback();
  });
  await page.getByRole("button", { name: "保存复核决定", exact: true }).click();
  await expect(
    page.locator(".ranking-review-panel .error-details"),
  ).toBeVisible();
  await expect(reason).toHaveValue(reasonA);
  await saved(page);
  await page.reload();
  await expect(reason).toHaveValue(reasonA);
  await page.getByRole("button", { name: "保存复核决定", exact: true }).click();
  await expect(reason).toHaveValue("");
  assert.equal(requests.length, 2);
  assert.equal(requests[0].idempotency_key, requests[1].idempotency_key);
  const history = await engine.api(
    rootPath +
      `/analysis/snapshots/${requests[0].snapshot_id}/reviews?ordinal=${requests[0].ordinal}`,
  );
  assert.equal(
    history.items.filter((r) => r.request.reason === reasonA).length,
    1,
  );
  await page.unroute(reviewUrl);
  await reason.fill("保存后跨页继续");
  await page.getByRole("button", { name: "保存并下一张", exact: true }).click();
  await expect(viewer(page)).toHaveAttribute("aria-label", second);
  await expect(reason).toHaveValue("图 B 的独立草稿");
  await expect(viewer(page).locator("img")).toBeVisible();
  await expect
    .poll(() =>
      viewer(page)
        .locator("img")
        .evaluate((image) => image.complete && image.naturalWidth > 0),
    )
    .toBe(true);
  await saved(page);
  await page.screenshot({ path: resolve(run, "continuous-review-2560.png") });
  checks.push(
    "lost review response is retried with the persisted idempotency key after reload; save-and-next crosses pages without replacing another image's draft",
  );

  await page
    .getByLabel("美学工作视图", { exact: true })
    .selectOption("protected");
  await expect(page.locator(".ranking-image-tile").first()).toBeVisible();
  await page.locator(".ranking-image-tile").first().click();
  const protectedLabel = await selected(page).getAttribute("aria-label");
  await tab(page, "保护复核").click();
  await page.getByLabel("保护决定", { exact: true }).selectOption("release");
  await reason.fill("从保护池释放，队列保留到刷新");
  await page.getByRole("button", { name: "保存复核决定", exact: true }).click();
  await expect(reason).toHaveValue("");
  await expect(selected(page)).toHaveAttribute("aria-label", protectedLabel);
  await expect(page.locator(".ranking-review-panel")).toContainText("解除保护");
  await page
    .getByRole("button", { name: "刷新排名工作台", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: protectedLabel, exact: true }),
  ).toHaveCount(0);
  checks.push(
    "effective protection updates immediately while the review queue remains pinned until explicit refresh",
  );

  await page
    .getByLabel("美学工作视图", { exact: true })
    .selectOption("ranking");
  await page
    .getByRole("button", { name: "返回排名网格", exact: true })
    .first()
    .click();
  await tab(page, "详情").click();
  const cdp = await page.context().newCDPSession(page);
  for (const [width, height, deviceScaleFactor] of [
    [2560, 1440, 1],
    [2560, 1392, 1],
    [1707, 928, 1.5],
    [1280, 560, 1],
  ]) {
    await page.setViewportSize({ width, height });
    await cdp.send("Emulation.setDeviceMetricsOverride", {
      width,
      height,
      deviceScaleFactor,
      mobile: false,
    });
    for (const name of ["详情", "评审依据", "保护复核"]) {
      await tab(page, name).click();
      const bounds = await page.evaluate(() => {
        const doc = globalThis.document;
        const canvas = doc
          .querySelector(".ranking-canvas")
          .getBoundingClientRect();
        const footer = doc
          .querySelector(".ranking-pagebar")
          .getBoundingClientRect();
        return {
          height: canvas.height,
          bottom: footer.bottom,
          viewport: globalThis.innerHeight,
          overflow: doc.documentElement.scrollWidth > globalThis.innerWidth,
        };
      });
      assert.ok(
        bounds.height >= 120 &&
          bounds.bottom <= bounds.viewport &&
          !bounds.overflow,
        JSON.stringify({ width, height, name, bounds }),
      );
    }
  }
  await page.setViewportSize({ width: 2560, height: 1440 });
  await tab(page, "详情").click();
  await cdp.send("Emulation.setDeviceMetricsOverride", {
    width: 2560,
    height: 1440,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await page.getByRole("button", { name: "恢复默认布局", exact: true }).click();
  await page.getByLabel("排名每页图片数").selectOption("48");
  await size.focus();
  await size.press("Home");
  for (let n = 0; n < 5; n++) await size.press("ArrowRight");
  await page.waitForFunction(() => {
    const images = [
      ...globalThis.document.querySelectorAll(".ranking-image-tile img"),
    ];
    return (
      images.length === 16 &&
      images.every((image) => image.complete && image.naturalWidth > 0)
    );
  });
  await saved(page);
  await page.screenshot({ path: resolve(run, "ranking-reading-2560.png") });
  await cdp.detach();
  checks.push(
    "all three right panels keep the canvas and bottom controls visible at primary, taskbar, remote-size and short-window viewports",
  );
}

export async function checkComparison(page, { engine, rootPath, run, checks }) {
  const jobs = (await engine.api(rootPath + "/analysis/jobs?limit=32")).items;
  const left = jobs.find((j) => j.request.name === "Davidson");
  const right = jobs.find((j) => j.request.name === "Borda");
  assert.ok(left && right);
  await page
    .getByLabel("美学工作视图", { exact: true })
    .selectOption("comparison");
  await tab(page, "对照设置").click();
  await page.getByLabel("基准快照 A", { exact: true }).selectOption(left.id);
  await page.getByLabel("对照快照 B", { exact: true }).selectOption(right.id);
  const name = "UI 快照对照 " + Date.now();
  await page.getByLabel("对照名称", { exact: true }).fill(name);
  await page.getByRole("button", { name: "生成离线对照", exact: true }).click();
  await expect(page.locator(".ranking-task-state")).toContainText("已完成", {
    timeout: 60000,
  });
  await expect(page.locator(".comparison-table tbody tr")).toHaveCount(32);
  const comparison = (
    await engine.api(rootPath + "/analysis/jobs?limit=32")
  ).items.find((j) => j.request.name === name);
  const result = await engine.api(
    rootPath + `/analysis/jobs/${comparison.id}/comparison?limit=32`,
  );
  const first = result.items[0];
  await page
    .getByRole("button", {
      name: `查看对照图片 ${first.position}`,
      exact: true,
    })
    .click();
  await expect(page.locator(".comparison-detail")).toContainText(
    first.key.asset_id,
  );
  if (!first.comparable)
    await expect(page.locator(".comparison-detail")).toContainText(
      first.reason,
    );
  else {
    const delta = (first.right_percentile - first.left_percentile) * 100;
    const text =
      Math.abs(delta) < 0.005
        ? "无变化"
        : `${delta < 0 ? "上移" : "下移"} ${Math.abs(delta).toFixed(2)} 个百分点`;
    await expect(page.locator(".comparison-detail")).toContainText(text);
  }
  await expect(page.locator(".comparison-detail img")).toBeVisible();
  await expect
    .poll(() =>
      page
        .locator(".comparison-detail img")
        .evaluate((image) => image.complete && image.naturalWidth > 0),
    )
    .toBe(true);
  await expect(
    page.locator('.ranking-snapshot-list button[aria-pressed="true"]'),
  ).toContainText("已完成");
  await saved(page);
  await page.screenshot({ path: resolve(run, "comparison-2560.png") });
  await page.getByRole("button", { name: "下一页对照", exact: true }).click();
  const rowIdentity = await page
    .locator(".comparison-select-image")
    .first()
    .getAttribute("aria-label");
  await saved(page);
  await page.reload();
  await expect(
    page.locator(".comparison-select-image").first(),
  ).toHaveAttribute("aria-label", rowIdentity);
  await expect(page.locator(".ranking-task-state")).toContainText(name);
  checks.push(
    "real SDK comparison publishes a bounded image table, explains comparable changes and restores the selected result and cursor after reload",
  );
  await page
    .getByLabel("美学工作视图", { exact: true })
    .selectOption("ranking");
}
