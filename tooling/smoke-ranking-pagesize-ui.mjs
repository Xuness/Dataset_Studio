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
assert.ok(
  previous.sparse_result_id && previous.sparse_first_members.length >= 97,
);
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
  slowPreparation = true,
  phase = "preparing";
async function visible(limit) {
  await expect(page.locator(".asset-card")).toHaveCount(limit, {
    timeout: 60000,
  });
  await expect
    .poll(() =>
      page.locator(".asset-card .asset-caption > span").allTextContents(),
    )
    .toEqual(
      previous.sparse_first_members
        .slice(0, limit)
        .map((row) => "Danbooru #" + row.post_id),
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
        id: previous.sparse_result_id,
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
  browser = await chromium.launch({ channel: "msedge", headless: true });
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
        if (slowPreparation && value.preparing) await sleep(600);
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
  await expect.poll(() => trace.some((row) => row.preparing)).toBe(true);
  await expect(page.getByLabel("每页数量")).toBeVisible();
  for (const limit of [12, 96, 48]) {
    const before = trace.length;
    await page.getByLabel("每页数量").selectOption(String(limit));
    await expect
      .poll(
        () =>
          trace
            .slice(before)
            .some((row) => row.limit === limit && row.preparing),
        { timeout: 15000 },
      )
      .toBe(true);
  }
  const scans = trace.filter((row) => row.preparing).map((row) => row.scanned);
  for (let i = 1; i < scans.length; i++)
    assert.ok(
      scans[i] > scans[i - 1],
      "page-size changes must continue the sparse scan",
    );
  checks.push(
    "switching 48/12/96/48 while preparation is in flight preserves scan progress",
  );
  slowPreparation = false;
  await visible(48);
  phase = "ready";
  for (const limit of [12, 96, 48, 12]) {
    await page.getByLabel("每页数量").selectOption(String(limit));
    await visible(limit);
  }
  assert.ok(
    trace.filter((row) => row.phase === "ready").every((row) => !row.preparing),
  );
  checks.push(
    "completed sparse prefixes serve every page size without repeating preparation",
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
