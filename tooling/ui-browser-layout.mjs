import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { expect } from "@playwright/test";

export async function openBrowserPanel(page, name) {
  await page.getByRole("tab", { name, exact: true }).click();
  await expect(page.getByRole("tab", { name, exact: true })).toHaveAttribute(
    "aria-selected",
    "true",
  );
}

export async function verifyBrowserLayout(page, run) {
  const sizes = [
    { width: 2560, height: 1440, scale: 1 },
    { width: 2560, height: 1392, scale: 1 },
    { width: 1707, height: 960, scale: 1.5 },
    { width: 1707, height: 928, scale: 1.5 },
    { width: 1540, height: 1000, scale: 1 },
    { width: 1280, height: 720, scale: 1 },
    { width: 1000, height: 680, scale: 1 },
    { width: 1280, height: 560, scale: 1 },
  ];
  const originalSize = page.viewportSize();
  const display = await page.context().newCDPSession(page);
  const samples = [];
  const grid = page.locator(".asset-scroll");
  for (const viewport of sizes) {
    await page.setViewportSize({
      width: viewport.width,
      height: viewport.height,
    });
    await display.send("Emulation.setDeviceMetricsOverride", {
      width: viewport.width,
      height: viewport.height,
      deviceScaleFactor: viewport.scale,
      mobile: false,
    });
    for (const panel of ["检查器", "筛选", "定位"]) {
      await openBrowserPanel(page, panel);
      await expect(
        page.locator(
          ".browser-view .quick-filters, .browser-view .ranking-start-controls",
        ),
      ).toHaveCount(0);
      await grid.evaluate((element) => {
        element.scrollTop = 0;
      });
      const box = await grid.boundingBox();
      await page.mouse.move(box.x + box.width / 2, box.y + 40);
      await page.mouse.wheel(0, 640);
      await expect
        .poll(() => grid.evaluate((element) => element.scrollTop))
        .toBeGreaterThan(0);
      const scrollTop = await grid.evaluate((element) => element.scrollTop);
      if (panel === "筛选") {
        for (const name of ["包含标签", "排除标签"]) {
          const field = page.getByLabel(name, { exact: true });
          await field.scrollIntoViewIfNeeded();
          await expect(field).toBeInViewport({ ratio: 1 });
        }
        await page
          .getByRole("button", { name: "应用筛选", exact: true })
          .scrollIntoViewIfNeeded();
      }
      if (panel === "定位") {
        await expect(page.getByLabel("起点 Danbooru ID")).toBeVisible();
        await expect(
          page.getByLabel("起点排名", { exact: true }),
        ).toBeVisible();
      }
      const side = page.locator(".wb-right > .wb-panel-content:not([hidden])");
      if (
        await side.evaluate(
          (element) => element.scrollHeight > element.clientHeight + 1,
        )
      ) {
        await side.evaluate((element) => {
          element.scrollTop = 0;
        });
        const sideBox = await side.boundingBox();
        await page.mouse.move(sideBox.x + sideBox.width - 20, sideBox.y + 10);
        await page.mouse.wheel(0, 320);
        await expect
          .poll(() => side.evaluate((element) => element.scrollTop))
          .toBeGreaterThan(0);
      }
      const elements = await page.evaluate(() =>
        Object.fromEntries(
          [
            ".studio-app",
            ".wb-canvas",
            ".browser-controls",
            ".asset-scroll",
            ".paging",
            ".status-bar",
            ".wb-right",
          ].map((selector) => {
            const element = globalThis.document.querySelector(selector);
            const rect = element.getBoundingClientRect();
            return [
              selector,
              {
                top: rect.top,
                bottom: rect.bottom,
                width: rect.width,
                height: rect.height,
                clientWidth: element.clientWidth,
                scrollWidth: element.scrollWidth,
              },
            ];
          }),
        ),
      );
      const label = `${viewport.width}x${viewport.height} @ ${viewport.scale}, ${panel}`;
      assert.ok(
        elements[".status-bar"].bottom <= viewport.height + 1,
        `Status bar clipped: ${label}`,
      );
      assert.ok(
        elements[".paging"].bottom <= elements[".wb-canvas"].bottom + 1,
        `Paging clipped: ${label}`,
      );
      assert.ok(
        elements[".asset-scroll"].height >= 120,
        `No image viewport: ${label}`,
      );
      for (const selector of [
        ".studio-app",
        ".browser-controls",
        ".paging",
        ".wb-right",
      ]) {
        assert.ok(
          elements[selector].scrollWidth <= elements[selector].clientWidth + 1,
          `Horizontal controls clipped in ${selector}: ${label}`,
        );
      }
      samples.push({ viewport, panel, scrollTop, elements });
      await grid.evaluate((element) => {
        element.scrollTop = 0;
      });
      await side.evaluate((element) => {
        element.scrollTop = 0;
      });
      if (panel === "筛选") {
        await expect
          .poll(
            () =>
              page.locator(".asset-thumb").evaluateAll((tiles) => {
                const viewport = globalThis.document
                  .querySelector(".asset-scroll")
                  .getBoundingClientRect();
                return tiles
                  .filter((tile) => {
                    const bounds = tile.getBoundingClientRect();
                    return (
                      bounds.bottom > viewport.top &&
                      bounds.top < viewport.bottom
                    );
                  })
                  .every((tile) => {
                    const image = tile.querySelector("img");
                    return image?.complete && image.naturalWidth > 0;
                  });
              }),
            { timeout: 15000 },
          )
          .toBe(true);
        await page.screenshot({
          path: resolve(
            run,
            `browser-layout-${viewport.width}x${viewport.height}.png`,
          ),
        });
      }
    }
  }
  await writeFile(
    resolve(run, "browser-layout.json"),
    JSON.stringify(samples, null, 2),
  );
  await display.send("Emulation.clearDeviceMetricsOverride");
  await display.detach();
  await page.setViewportSize(originalSize);

  await openBrowserPanel(page, "筛选");
  const include = page.getByLabel("包含标签", { exact: true });
  await include.fill("jpeg_artifacts");
  await openBrowserPanel(page, "定位");
  await page.getByLabel("起点排名", { exact: true }).fill("7");
  await openBrowserPanel(page, "筛选");
  await expect(include).toHaveValue("jpeg_artifacts");
  await page
    .getByRole("button", { name: "筛选面板更多操作", exact: true })
    .click();
  await page.getByRole("menuitem", { name: "停靠到底部", exact: true }).click();
  await expect(
    page.locator(".wb-bottom").getByLabel("包含标签", { exact: true }),
  ).toHaveValue("jpeg_artifacts");
  await page.getByRole("button", { name: "隐藏筛选面板", exact: true }).click();
  await page.getByRole("button", { name: "显示筛选", exact: true }).click();
  await expect(
    page.locator(".wb-right").getByLabel("包含标签", { exact: true }),
  ).toHaveValue("jpeg_artifacts");
  await page.getByRole("button", { name: "隐藏筛选面板", exact: true }).click();
  await page
    .getByRole("button", { name: "Rating / Tag 筛选", exact: true })
    .click();
  await expect(include).toHaveValue("jpeg_artifacts");
  await expect(page.locator(".status-bar")).toContainText("项目已保存");
  await page.reload();
  await expect(
    page.getByRole("tab", { name: "筛选", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(include).toHaveValue("jpeg_artifacts");
  await include.fill("");
  await openBrowserPanel(page, "定位");
  await page.getByLabel("起点排名", { exact: true }).fill("");

  await page
    .locator(".options-bar")
    .getByRole("button", { name: "查询", exact: true })
    .click();
  const query = page.getByRole("region", { name: "项目查询", exact: true });
  await expect(query).toBeVisible();
  await expect(page.locator(".data-workspace .query-panel")).toHaveCount(0);
  for (const viewport of sizes.filter((size) => size.height <= 680)) {
    await page.setViewportSize({
      width: viewport.width,
      height: viewport.height,
    });
    await expect(grid).toBeInViewport({ ratio: 1 });
    await expect(page.locator(".paging")).toBeInViewport({ ratio: 1 });
    await expect(page.getByLabel("查询名称", { exact: true })).toBeInViewport({
      ratio: 1,
    });
    assert.ok(
      await query.evaluate(
        (element) => element.scrollWidth <= element.clientWidth + 1,
      ),
      "Query side panel overflows horizontally",
    );
  }
  await page.getByRole("button", { name: "收起查询", exact: true }).click();
  await openBrowserPanel(page, "检查器");
  await page.setViewportSize(originalSize);
  await grid.evaluate((element) => {
    element.scrollTop = 0;
  });
}
