import { pythonCommand, browserOptions } from "./platform.mjs";
// Real preset APIs and streaming project events against an isolated engine.
// Only the old browse result's expired status is replayed at the HTTP boundary.
import assert from "node:assert/strict";
import { execFile, spawn } from "node:child_process";
import { mkdir, open, readFile, writeFile } from "node:fs/promises";
import { createServer, request as httpRequest } from "node:http";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { chromium, expect } from "@playwright/test";
import { EngineFixture, sleep } from "./engine-fixture.mjs";
import { openEditor } from "./ui-workbench.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(root, ".local/test-runs/preset-recovery-ui-" + Date.now());
const state = resolve(run, "state");
await mkdir(run, { recursive: true });
await promisify(execFile)(
  pythonCommand(),
  [resolve(root, "tooling/ui-fixture.py"), resolve(run, "fixture")],
  { cwd: root, windowsHide: true },
);
const fixture = JSON.parse(
  await readFile(resolve(run, "fixture/fixture.json"), "utf8"),
);
const engine = new EngineFixture(root, state);
const url = "http://127.0.0.1:1459";
const checks = [],
  errors = [],
  events = [];
let browser, page, vite, proxy;
let expired = false,
  resultReads = 0;
try {
  await engine.start();
  const project = await engine.api("/v1/projects", "POST", {
    name: "参数预设与浏览恢复验收",
    parent_directory: resolve(run, "projects"),
  });
  const base = "/v1/projects/" + project.id;
  const source = await engine.api(base + "/sources", "POST", {
    kind: "danbooru",
    name: "Danbooru",
    index_root: fixture.lake,
    media_root: fixture.lake,
  });
  const submitted = await engine.api(base + "/query-results", "POST", {
    spec: {
      version: 3,
      source_ids: [source.id],
      conditions: [],
      observation_rule: "current_post",
      order: "asset_key_asc",
    },
  });
  await engine.wait(
    base + "/query-results/" + submitted.id,
    (r) => r.state === "ready",
  );
  const resultPath = base + "/query-results/" + submitted.id;
  const workspacePath = base + "/drafts/studio.session/default";
  await engine.api(workspacePath, "PUT", {
    schema_version: 1,
    expected_revision: 0,
    value: {
      order: "asset_key_asc",
      moduleId: "core.tools",
      openViews: ["core.browser", "core.tools"],
      panels: [],
      scope: { kind: "result", id: submitted.id, name: "旧浏览结果" },
      focusKey: null,
      view: "grid",
      position: null,
      inspectorTab: "properties",
      managementTarget: null,
    },
  });

  // Forward SSE as a stream: buffering it would hide the preset.changed bug.
  proxy = createServer((request, reply) => {
    const path = request.url;
    const cors = {
      "access-control-allow-origin": url,
      "access-control-allow-methods": "GET,POST,PUT,PATCH,DELETE,OPTIONS",
      "access-control-allow-headers":
        request.headers["access-control-request-headers"] ??
        "authorization,content-type,x-studio-session,x-studio-read-id",
    };
    if (request.method === "OPTIONS") {
      reply.writeHead(204, cors);
      reply.end();
      return;
    }
    const headers = { ...request.headers, "accept-encoding": "identity" };
    delete headers.host;
    delete headers.origin;
    const resultRequest = request.method === "GET" && path === resultPath;
    if (resultRequest) resultReads++;
    const upstream = httpRequest(
      engine.connection.endpoint + path,
      { method: request.method, headers },
      (response) => {
        const responseHeaders = { ...response.headers, ...cors };
        delete responseHeaders["content-length"];
        delete responseHeaders["transfer-encoding"];
        if (resultRequest && expired) {
          let body = "";
          response.setEncoding("utf8");
          response.on("data", (chunk) => {
            body += chunk;
          });
          response.on("end", () => {
            const result = JSON.parse(body);
            reply.writeHead(response.statusCode, responseHeaders);
            reply.end(
              JSON.stringify({ ...result, state: "released", count: null }),
            );
          });
          return;
        }
        if (path.startsWith(base + "/events")) {
          let buffer = "";
          response.on("data", (chunk) => {
            buffer += chunk.toString("utf8").replace(/\r\n/g, "\n");
            let boundary;
            while ((boundary = buffer.indexOf("\n\n")) >= 0) {
              const frame = buffer.slice(0, boundary);
              buffer = buffer.slice(boundary + 2);
              const data = frame
                .split("\n")
                .find((line) => line.startsWith("data:"));
              if (data) events.push(JSON.parse(data.slice(5)));
            }
          });
        }
        reply.writeHead(response.statusCode, responseHeaders);
        response.pipe(reply);
      },
    );
    upstream.on("error", () => {
      if (!reply.destroyed) {
        if (!reply.headersSent) reply.writeHead(502, cors);
        reply.end();
      }
    });
    reply.on("close", () => upstream.destroy());
    request.pipe(upstream);
  });
  await new Promise((done) => proxy.listen(0, "127.0.0.1", done));
  const endpoint = "http://127.0.0.1:" + proxy.address().port;
  const log = await open(resolve(run, "vite.log"), "a");
  vite = spawn(
    process.execPath,
    [
      resolve(root, "apps/desktop/node_modules/vite/bin/vite.js"),
      "--port",
      "1459",
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
    if ((await fetch(url).catch(() => null))?.ok) break;
    if (vite.exitCode !== null) throw new Error("Fixture Vite exited");
    await sleep(100);
  }
  browser = await chromium.launch({ ...browserOptions(), headless: true });
  const context = await browser.newContext({
    viewport: { width: 1920, height: 1080 },
  });
  await context.route(url + "/__studio/connection", (route) =>
    route.fulfill({ json: { ...engine.connection, endpoint } }),
  );
  page = await context.newPage();
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(url);
  await page.locator(".recent-row").filter({ hasText: project.name }).click();
  const panel = page.getByRole("region", { name: "Danbooru 元数据排名" });
  const scope = panel.getByLabel("输入范围", { exact: true });
  await scope.selectOption(source.id);
  await panel.getByLabel("筛选方案", { exact: true }).selectOption("v2");
  await panel
    .getByRole("radio", { name: "按名额生成候选集", exact: true })
    .check();
  await panel
    .getByRole("checkbox", { name: "启用成品最短边门槛", exact: true })
    .check();
  await panel.getByLabel("成品最短边（px）", { exact: true }).fill("384");
  await panel.getByLabel("剩余池随机审计（%）", { exact: true }).fill("0");
  await expect(page.locator(".status-bar")).toContainText("项目已保存");
  await expect
    .poll(() => events.some((event) => event.kind === "project.sync"))
    .toBe(true);
  const draftPath = base + "/drafts/core.tools/ranking";
  const expectedDraft = (await engine.api(draftPath)).draft.value;
  const previousReads = resultReads;
  expired = true;

  await panel
    .getByRole("button", { name: "保存当前参数", exact: true })
    .click();
  await page.getByLabel("预设名称", { exact: true }).fill("V2 参数保存验收");
  await page.getByRole("button", { name: "保存预设", exact: true }).click();
  await expect(panel.locator(".preset-notice")).toHaveText("参数预设已保存。");
  const presetSelect = panel.getByLabel("选择参数预设", { exact: true });
  const presetId = await presetSelect.inputValue();
  const presetListPath = base + "/presets?operator_id=danbooru.metarecall_v2";
  const saved = (await engine.api(presetListPath)).items.find(
    (p) => p.id === presetId,
  );
  assert.ok(saved);
  assert.deepEqual(saved.run.parameters, expectedDraft.parameters);

  // An external preset change proves the UI consumed the real event stream.
  const external = await engine.api(base + "/presets", "POST", {
    id: null,
    name: "来自另一窗口的预设",
    notes: "",
    expected_revision: 0,
    run: saved.run,
  });
  const externalOption = presetSelect.locator(`option[value="${external.id}"]`);
  await expect(externalOption).toHaveCount(1);
  await expect(page.locator(".error-banner")).toHaveCount(0);
  assert.equal(
    resultReads,
    previousReads,
    "Preset events must not refetch the unrelated browse result",
  );
  await engine.api(base + "/presets/" + external.id + "/delete", "POST", {
    expected_revision: external.revision,
  });
  await expect(externalOption).toHaveCount(0);
  assert.equal(resultReads, previousReads);
  checks.push(
    "V2 preset values are saved exactly; live preset create/delete events refresh only presets",
  );

  // Other project changes may refresh the old result while Tools stays open.
  const sourcePath = base + "/objects/source/" + source.id;
  const sourceInfo = await engine.api(sourcePath);
  const sourceName = "Danbooru · 已更新名称";
  await engine.api(sourcePath, "PATCH", {
    expected_revision: sourceInfo.object.revision,
    name: sourceName,
    notes: "隔离验证中的后台名称更新",
  });
  await expect(scope.locator("option:checked")).toContainText(sourceName);
  await expect.poll(() => resultReads).toBeGreaterThan(previousReads);
  await expect(page.locator(".error-banner")).toHaveCount(0);
  assert.equal(
    (await engine.api(workspacePath)).draft.value.scope.id,
    submitted.id,
  );
  assert.deepEqual((await engine.api(draftPath)).draft.value, expectedDraft);
  await page.screenshot({
    path: resolve(run, "preset-saved-with-expired-background-result.png"),
  });
  checks.push(
    "Expired background browse results do not reset scope or disturb ranking parameters on unrelated project refresh",
  );

  await page.reload();
  await expect(panel).toBeVisible();
  await expect(scope).toHaveValue(source.id);
  await expect(
    panel.getByLabel("成品最短边（px）", { exact: true }),
  ).toHaveValue("384");
  await expect(
    panel.getByLabel("剩余池随机审计（%）", { exact: true }),
  ).toHaveValue("0");
  await expect(presetSelect.locator(`option[value="${presetId}"]`)).toHaveCount(
    1,
  );
  await expect(page.locator(".error-banner")).toHaveCount(0);
  assert.equal(
    (await engine.api(workspacePath)).draft.value.scope.id,
    submitted.id,
  );
  checks.push(
    "Reloading the tools page preserves the preset and parameters without unrelated browse recovery",
  );

  await openEditor(page, "资料浏览");
  await expect(page.locator(".error-banner")).toContainText(
    "已保存的查询结果已释放或尚未完成",
  );
  await engine.wait(workspacePath, (r) => r.draft.value.scope.kind === "all");
  await expect(page.locator(".asset-grid article").first()).toBeVisible();
  await page.getByRole("button", { name: "关闭错误提示", exact: true }).click();
  await openEditor(page, "计算工具");
  await expect(scope).toHaveValue(source.id);
  await expect(presetSelect.locator(`option[value="${presetId}"]`)).toHaveCount(
    1,
  );
  assert.equal((await engine.api(base + "/jobs")).items.length, 0);
  assert.deepEqual(errors, []);
  checks.push(
    "Returning to Browser still recovers the expired result; ranking scope and saved preset remain intact and no job is submitted",
  );

  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      {
        passed: true,
        checks,
        events: events.filter((event) => event.kind.startsWith("preset.")),
        resultReads,
        errors,
        scope:
          "Isolated synthetic lake and project; real preset API and SSE; old result expiration is an HTTP presentation replay",
      },
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
  if (page) {
    await page
      .screenshot({ path: resolve(run, "failure.png") })
      .catch(() => {});
    await writeFile(
      resolve(run, "failure-dom.txt"),
      await page
        .locator("body")
        .innerText()
        .catch(() => ""),
    );
  }
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
  if (proxy) {
    proxy.closeAllConnections();
    await new Promise((done) => proxy.close(done));
  }
  if (vite && vite.exitCode === null) {
    const exited = new Promise((done) => vite.once("exit", done));
    vite.kill();
    await exited;
  }
  await engine.stop();
}
