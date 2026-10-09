import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";
import { createServer } from "../apps/desktop/node_modules/vite/dist/node/index.js";
import { EngineFixture, within } from "./engine-fixture.mjs";
import { browserOptions } from "./platform.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
if (!process.argv[2])
  throw new Error("需要 integration-worksets 的隔离运行目录");
const fixture = within(
  resolve(root, ".local/test-runs"),
  resolve(process.argv[2]),
);
const report = JSON.parse(
  await readFile(resolve(fixture, "report.json"), "utf8"),
);
assert.equal(report.passed, true);
const run = resolve(
  root,
  ".local/test-runs",
  "workset-members-ui-" + Date.now(),
);
await mkdir(run, { recursive: true });
const engine = new EngineFixture(root, resolve(fixture, "state"), run);
const { project, collection, ranked, source } = report;
const base = `/v1/projects/${project.id}`;
const images = JSON.parse(
  await readFile(resolve(fixture, "images/fixture.json"), "utf8"),
);
const key = (n) => ({
  source_id: source.id,
  asset_id: images.images.find((i) => i.number === n).sha,
});
const checks = [],
  errors = [],
  requests = [];
let browser, server, page;
process.on("exit", () => engine.child?.kill());
try {
  await engine.start();
  await engine.api("/v1/projects/open", "POST", {
    directory: project.directory,
  });
  for (const saved of [collection, ranked]) {
    const current = (await engine.api(base + "/collections")).items.find(
      (c) => c.id === saved.id,
    );
    await engine.api(base + `/collections/${saved.id}/members`, "POST", {
      request_id: crypto.randomUUID(),
      expected_revision: current.revision,
      change: { kind: "restore", revision: saved.revision },
    });
  }
  const selection = await engine.api(base + "/selection");
  const selected = await engine.api(base + "/selection", "PATCH", {
    expected_revision: selection.revision,
    add: [key(3), key(4)],
    remove: [],
    clear: true,
  });
  const previous = (await engine.api(base + "/drafts/studio.session/default"))
    .draft;
  await engine.api(base + "/drafts/studio.session/default", "PUT", {
    schema_version: 1,
    expected_revision: previous?.revision ?? 0,
    value: {
      moduleId: "core.browser",
      openViews: ["core.browser"],
      panels: [],
      scope: { kind: "collection", id: collection.id, name: collection.name },
      focusKey: null,
      view: "grid",
      position: null,
      order: "asset_key_asc",
      booruOrder: "post_id_desc",
      rankedBrowse: null,
      inspectorTab: "properties",
      managementTarget: null,
    },
  });
  await engine.api(base + "/close", "POST");
  server = await createServer({
    configFile: resolve(root, "apps/desktop/vite.config.ts"),
    root: resolve(root, "apps/desktop"),
    cacheDir: resolve(run, "vite-cache"),
    server: { host: "127.0.0.1", port: 0, strictPort: false },
  });
  await server.listen();
  const url = `http://127.0.0.1:${server.httpServer.address().port}`;
  browser = await chromium.launch({ ...browserOptions(), headless: true });
  const context = await browser.newContext({
    viewport: { width: 2560, height: 1440 },
    deviceScaleFactor: 1,
  });
  await context.route(url + "/__studio/connection", (route) =>
    route.fulfill({ json: engine.connection }),
  );
  const lostUrl =
    engine.connection.endpoint + base + `/collections/${collection.id}/members`;
  let lose = false;
  // The test frontend has an ephemeral loopback origin. Proxy only this
  // isolated engine, leaving the production origin allowlist unchanged.
  await context.route(engine.connection.endpoint + "/**", async (route) => {
    const request = route.request();
    const headers = { ...request.headers() };
    delete headers.origin;
    delete headers.host;
    delete headers["content-length"];
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
    if (lose && request.url() === lostUrl && request.method() === "POST") {
      lose = false;
      assert.equal(response.ok, true);
      await response.arrayBuffer();
      return route.abort("failed");
    }
    await route.fulfill({
      status: response.status,
      headers: { ...Object.fromEntries(response.headers), ...cors },
      body: Buffer.from(await response.arrayBuffer()),
    });
  });
  page = await context.newPage();
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("request", (request) => {
    if (
      request.method() === "POST" &&
      /\/collections\/[^/]+\/members$/.test(request.url())
    )
      requests.push(request.postDataJSON());
  });
  await page.goto(url);
  await page.locator(".recent-row").filter({ hasText: project.name }).click();
  await expect(page.locator(".asset-card")).toHaveCount(2, { timeout: 30000 });
  const dialog = () => page.getByRole("dialog");
  const collectionCount = async (id) =>
    (await engine.api(base + "/collections")).items.find((c) => c.id === id);
  await page
    .locator(".options-bar")
    .getByRole("button", { name: "加入已有工作集…", exact: true })
    .click();
  await expect(dialog().getByLabel("图片范围")).toHaveValue("selection");
  await dialog().getByLabel("目标工作集").selectOption(collection.id);
  await page.screenshot({ path: resolve(run, "add-dialog.png") });
  await dialog()
    .getByRole("button", { name: "加入工作集", exact: true })
    .click();
  await expect(
    dialog().getByText("已加入 2 项。", { exact: true }),
  ).toBeVisible({ timeout: 30000 });
  await expect(page.locator(".asset-card")).toHaveCount(4);
  await dialog()
    .getByRole("button", { name: "撤销本次操作", exact: true })
    .click();
  await expect(
    dialog().getByText("已撤销本次操作。", { exact: true }),
  ).toBeVisible({ timeout: 30000 });
  await expect(page.locator(".asset-card")).toHaveCount(2);
  await dialog().getByRole("button", { name: "完成", exact: true }).click();
  assert.equal(
    (await engine.api(base + "/selection")).revision,
    selected.revision,
  );
  checks.push(
    "toolbar adds the current selection to an existing workset and undo refreshes the grid without changing selection",
  );

  await page
    .locator(".asset-card .asset-thumb")
    .first()
    .click({ button: "right" });
  await page
    .getByRole("menuitem", { name: "从此工作集移除…", exact: true })
    .click();
  await expect(dialog().getByLabel("目标工作集")).toHaveValue(collection.id);
  await dialog()
    .getByRole("button", { name: "从工作集移除", exact: true })
    .click();
  await expect(
    dialog().getByText("已移除 1 项。", { exact: true }),
  ).toBeVisible({ timeout: 30000 });
  await expect(page.locator(".asset-card")).toHaveCount(1);
  await dialog()
    .getByRole("button", { name: "撤销本次操作", exact: true })
    .click();
  await expect(
    dialog().getByText("已撤销本次操作。", { exact: true }),
  ).toBeVisible({ timeout: 30000 });
  await dialog().getByRole("button", { name: "完成", exact: true }).click();
  await expect(page.locator(".asset-card")).toHaveCount(2);
  checks.push(
    "image context menu removes only the clicked image and restores it atomically",
  );

  await page.locator(".asset-card .asset-thumb").first().dblclick();
  await expect(page.locator(".image-canvas img").first()).toBeVisible({
    timeout: 30000,
  });
  await page.locator(".image-canvas img").first().click({ button: "right" });
  await page
    .getByRole("menuitem", { name: "加入已有工作集…", exact: true })
    .click();
  await dialog().getByLabel("目标工作集").selectOption(ranked.id);
  await dialog()
    .getByRole("button", { name: "加入工作集", exact: true })
    .click();
  await expect(
    dialog().getByText("已加入 1 项。", { exact: true }),
  ).toBeVisible({ timeout: 30000 });
  await dialog()
    .getByRole("button", { name: "撤销本次操作", exact: true })
    .click();
  await expect(
    dialog().getByText("已撤销本次操作。", { exact: true }),
  ).toBeVisible({ timeout: 30000 });
  await dialog().getByRole("button", { name: "完成", exact: true }).click();
  checks.push(
    "large-image menu can add to a different ranked workset and undo",
  );

  const before = await collectionCount(collection.id);
  lose = true;
  await page
    .locator(".options-bar")
    .getByRole("button", { name: "加入已有工作集…", exact: true })
    .click();
  await dialog().getByLabel("目标工作集").selectOption(collection.id);
  const start = requests.length;
  await dialog()
    .getByRole("button", { name: "加入工作集", exact: true })
    .click();
  await expect
    .poll(() => collectionCount(collection.id).then((c) => c.count))
    .toBe(4);
  await expect(
    dialog().getByRole("button", { name: "加入工作集", exact: true }),
  ).toBeEnabled({ timeout: 30000 });
  await dialog()
    .getByRole("button", { name: "加入工作集", exact: true })
    .click();
  await expect(
    dialog().getByText("已加入 2 项。", { exact: true }),
  ).toBeVisible({ timeout: 30000 });
  const replay = requests.slice(start);
  assert.ok(replay.length >= 2);
  assert.equal(new Set(replay.map((r) => r.request_id)).size, 1);
  assert.equal(
    (await collectionCount(collection.id)).revision,
    before.revision + 1,
  );
  await dialog()
    .getByRole("button", { name: "撤销本次操作", exact: true })
    .click();
  await expect(
    dialog().getByText("已撤销本次操作。", { exact: true }),
  ).toBeVisible({ timeout: 30000 });
  await dialog().getByRole("button", { name: "完成", exact: true }).click();
  checks.push(
    "a lost successful response retries the same request ID and creates only one member revision",
  );

  await page
    .locator(".options-bar")
    .getByRole("button", { name: "从工作集移除…", exact: true })
    .click();
  await dialog().getByLabel("图片范围").selectOption(collection.id);
  await dialog()
    .getByRole("button", { name: "从工作集移除", exact: true })
    .click();
  await expect(
    dialog().getByText("已移除 2 项。", { exact: true }),
  ).toBeVisible({ timeout: 30000 });
  await dialog().getByRole("button", { name: "完成", exact: true }).click();
  await expect(
    page.getByText("这个工作集没有图片", { exact: true }),
  ).toBeVisible({ timeout: 30000 });
  await page.screenshot({ path: resolve(run, "empty-workset.png") });
  await page
    .locator(".options-bar")
    .getByRole("button", { name: "加入已有工作集…", exact: true })
    .click();
  await dialog().getByLabel("目标工作集").selectOption(collection.id);
  await dialog()
    .getByRole("button", { name: "加入工作集", exact: true })
    .click();
  await expect(
    dialog().getByText("已加入 2 项。", { exact: true }),
  ).toBeVisible({ timeout: 30000 });
  assert.equal((await collectionCount(collection.id)).count, 2);
  await dialog().getByRole("button", { name: "完成", exact: true }).click();
  assert.equal(
    (await engine.api(base + "/selection")).revision,
    selected.revision,
  );
  checks.push(
    "removing every member retains the workset and the same target accepts new members",
  );
  assert.deepEqual(errors, []);
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      {
        passed: true,
        checks,
        errors,
        viewport: { width: 2560, height: 1440, scale: 1 },
        fixture,
      },
      null,
      2,
    ),
  );
  console.log(JSON.stringify({ passed: true, checks: checks.length, run }));
} catch (error) {
  if (page)
    await page
      .screenshot({ path: resolve(run, "failure.png") })
      .catch(() => {});
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
  await server?.close();
  await engine.stop();
}
