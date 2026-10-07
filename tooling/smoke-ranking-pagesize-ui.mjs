import { browserOptions } from "./platform.mjs";
// Uses the completed ranking integration fixture, never a user project.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, readFile, writeFile, open } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";
import { EngineFixture, sleep, within } from "./engine-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
if (!process.argv[2])
  throw new Error(
    "Pass a completed integration-ranking-browse fixture directory",
  );
const fixtureRun = within(
  resolve(root, ".local/test-runs"),
  resolve(process.argv[2]),
);
const previous = JSON.parse(
  await readFile(resolve(fixtureRun, "report.json"), "utf8"),
);
assert.equal(previous.passed, true);
assert.ok(previous.gap_result_id && previous.gap_post_ids.length > 110 * 96);
const run = resolve(
  root,
  ".local/test-runs",
  "smoke-ranking-pagesize-ui-" + Date.now(),
);
await mkdir(run, { recursive: true });
const state = resolve(fixtureRun, "state");
const engine = new EngineFixture(root, state, run);
const url = "http://127.0.0.1:1434";
const base = "/v1/projects/" + previous.project_id;
const trace = [],
  checks = [],
  errors = [];
let browser,
  page,
  vite,
  phase = "preparing";
async function visible(limit, number = 0) {
  await expect(page.locator(".asset-card")).toHaveCount(limit, {
    timeout: 60000,
  });
  await expect
    .poll(() =>
      page.locator(".asset-card .asset-caption > span").allTextContents(),
    )
    .toEqual(
      previous.gap_post_ids
        .slice(number * limit, (number + 1) * limit)
        .map((id) => "Danbooru #" + id),
    );
}
try {
  await engine.start();
  const project = await engine.api(base + "/open", "POST");
  const draft = await engine.api(base + "/drafts/studio.session/default");
  await engine.api(base + "/drafts/studio.session/default", "PUT", {
    schema_version: 1,
    expected_revision: draft.draft?.revision ?? 0,
    value: {
      order: "asset_key_asc",
      moduleId: "core.browser",
      panels: [],
      scope: {
        kind: "result",
        id: previous.gap_result_id,
        name: "含稀疏排名成员的筛选",
      },
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
      "1434",
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
  for (let i = 0; i < 100; i++) {
    if (
      (await fetch(url, { signal: AbortSignal.timeout(500) }).catch(() => null))
        ?.ok
    )
      break;
    if (vite.exitCode !== null) throw Error("Fixture frontend exited");
    await sleep(100);
  }
  browser = await chromium.launch({ ...browserOptions(), headless: true });
  const context = await browser.newContext({
    viewport: { width: 1540, height: 1000 },
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
      "access-control-allow-methods": "GET,POST,PUT,PATCH,DELETE,OPTIONS",
      "access-control-allow-headers":
        headers["access-control-request-headers"] ??
        "authorization,content-type,x-studio-session,x-studio-read-id",
    };
    if (request.method() === "OPTIONS")
      return route.fulfill({ status: 204, headers: cors });
    const response = await fetch(request.url(), {
      method: request.method(),
      headers,
      ...(request.postData() ? { body: request.postData() } : {}),
    });
    const bytes = Buffer.from(await response.arrayBuffer());
    if (
      request.url().endsWith("/ranking-browse/assets") &&
      request.method() === "POST"
    ) {
      const body = JSON.parse(request.postData());
      const value = JSON.parse(bytes.toString());
      if (body.limit > 4 && response.ok) {
        trace.push({
          phase,
          limit: body.limit,
          supplied_cursor: !!body.cursor,
          preparing: !!value.preparing,
          scanned: value.scan?.scanned ?? null,
          items: value.items.length,
        });
      }
    }
    await route
      .fulfill({
        status: response.status,
        headers: { ...Object.fromEntries(response.headers), ...cors },
        body: bytes,
      })
      .catch(() => {});
  });
  page = await context.newPage();
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(url);
  await page.locator(".recent-row").filter({ hasText: project.name }).click();
  await visible(48);
  await page.getByLabel("每页数量").selectOption("96");
  await visible(96);
  for (let number = 1; number < 110; number++) {
    await page
      .locator(".browser-view")
      .getByRole("button", { name: "下一页", exact: true })
      .click();
    await visible(96, number);
  }
  checks.push(
    "110 actual next-page clicks cross the sparse gap with no per-page preparation",
  );
  await expect
    .poll(
      async () =>
        (await engine.api(base + "/drafts/studio.session/default")).draft?.value
          ?.position?.pageNumber,
    )
    .toBe(110);
  phase = "after_reload";
  await page.reload();
  await visible(96, 109);
  for (let number = 108; number >= 80; number--) {
    await page
      .locator(".browser-view")
      .getByRole("button", { name: "上一页", exact: true })
      .click();
    await visible(96, number);
  }
  checks.push(
    "backward paging still seeks directly after the frontend page cache has been discarded",
  );
  for (const limit of [12, 96, 48, 12]) {
    await page.getByLabel("每页数量").selectOption(String(limit));
    await visible(limit);
  }
  assert.ok(trace.every((row) => !row.preparing));
  const metrics = (await engine.api("/v1/resources")).query_cache;
  assert.equal(
    metrics.ranked_index_builds,
    0,
    "the restarted engine must reuse existing disk indexes",
  );
  checks.push(
    "page sizes and frontend reload reuse the same persisted scope index",
  );
  await page.screenshot({ path: resolve(run, "sparse-page-size.png") });
  assert.deepEqual(errors, []);
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      { passed: true, checks, trace, errors, fixture: fixtureRun },
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
      { passed: false, checks, trace, errors, error: String(error) },
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
