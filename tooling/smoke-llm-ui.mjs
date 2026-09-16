// Isolated engine + browser context + local mock provider; no external API calls.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, open, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";
import { EngineFixture, sleep } from "./engine-fixture.mjs";
import { llmFixture } from "./llm-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(root, ".local/test-runs/smoke-llm-ui-" + Date.now());
await mkdir(run, { recursive: true });
const engine = new EngineFixture(root, resolve(run, "state")),
  mock = await llmFixture();
const url = "http://127.0.0.1:1437",
  checks = [],
  errors = [];
let browser, page, vite;
try {
  await engine.start();
  const log = await open(resolve(run, "vite.log"), "a");
  vite = spawn(
    process.execPath,
    [
      resolve(root, "apps/desktop/node_modules/vite/bin/vite.js"),
      "--port",
      "1437",
      "--strictPort",
    ],
    {
      cwd: resolve(root, "apps/desktop"),
      env: { ...process.env, STUDIO_DATA_DIR: resolve(run, "state") },
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
  browser = await chromium.launch({ channel: "msedge", headless: true });
  const context = await browser.newContext({
    viewport: { width: 1440, height: 1050 },
  });
  await context.route(url + "/__studio/connection", (route) =>
    route.fulfill({ json: engine.connection }),
  );
  await context.route(engine.connection.endpoint + "/**", async (route) => {
    const request = route.request(),
      headers = { ...request.headers() };
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
  await page.getByRole("menuitem", { name: "设置", exact: true }).click();
  await page
    .locator(".menu-popup button")
    .filter({ hasText: "缓存与存储" })
    .click();
  await page.getByRole("button", { name: "API 与模型", exact: true }).click();
  await page.getByRole("button", { name: "添加连接", exact: true }).click();
  await page.getByLabel("连接名称", { exact: true }).fill("界面验收连接");
  await page.getByRole("button", { name: "编辑与撤销", exact: true }).click();
  await page.getByRole("button", { name: "API 与模型", exact: true }).click();
  await expect(page.getByLabel("连接名称", { exact: true })).toHaveValue(
    "界面验收连接",
  );
  checks.push("connection draft survives settings page switch");
  await page.getByLabel("服务类型", { exact: true }).selectOption("openrouter");
  await page
    .getByLabel("API 基础地址", { exact: true })
    .fill(mock.url + "/router");
  await page.getByLabel("API Key", { exact: true }).fill("ui-fixture-secret");
  await page.getByRole("button", { name: "保存连接", exact: true }).click();
  await expect(
    page.getByText("供应商连接已保存。", { exact: true }),
  ).toBeVisible();
  const provider = (await engine.api("/v1/llm/providers")).items[0];
  assert.equal(provider.credential_set, true);
  assert.equal(JSON.stringify(provider).includes("ui-fixture-secret"), false);
  await page.getByRole("button", { name: "获取模型", exact: true }).click();
  await expect(page.locator(".llm-catalog .llm-model-row")).toHaveCount(2);
  checks.push(
    "connection is persisted and model catalog is fetched through SDK",
  );
  await page
    .locator(".llm-catalog .llm-model-row")
    .first()
    .getByRole("button", { name: "添加配置" })
    .click();
  await page.getByLabel("模型显示名称", { exact: true }).fill("模型一");
  await page
    .getByLabel("Temperature设置方式", { exact: true })
    .selectOption("value");
  await page.locator('[id="llm-value-temperature"]').fill("0");
  await page
    .getByLabel("最大输出 Token设置方式", { exact: true })
    .selectOption("value");
  await page.locator('[id="llm-value-max_output_tokens"]').fill("200");
  await page.getByRole("button", { name: "保存模型配置", exact: true }).click();
  await expect(
    page.getByText("模型配置已保存。", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "手动添加模型", exact: true }).click();
  await page.getByLabel("模型显示名称", { exact: true }).fill("模型二");
  await page.getByLabel("远端模型 ID", { exact: true }).fill("other-model");
  await page
    .getByLabel("Temperature设置方式", { exact: true })
    .selectOption("value");
  await page.locator('[id="llm-value-temperature"]').fill("0.7");
  await page.locator("summary").filter({ hasText: "命名参数预设" }).click();
  await page.getByLabel("参数预设名称", { exact: true }).fill("界面参数预设");
  await page.getByRole("button", { name: "保存参数预设", exact: true }).click();
  await expect(
    page.getByText("参数预设已保存。", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "保存模型配置", exact: true }).click();
  await expect(page.locator(".llm-model-list .llm-model-row")).toHaveCount(2);
  const models = (
    await engine.api("/v1/llm/providers/" + provider.id + "/models")
  ).items;
  assert.equal(
    models.find((m) => m.config.name === "模型一").config.parameters
      .temperature,
    0,
  );
  assert.equal(
    models.find((m) => m.config.name === "模型二").config.parameters
      .temperature,
    0.7,
  );
  checks.push(
    "zero value and separate model parameters persist; named parameter preset saved",
  );
  await page
    .locator(".llm-model-list .llm-model-row")
    .filter({ hasText: "模型一" })
    .getByRole("button", { name: "调用检查" })
    .click();
  await page.getByRole("button", { name: "发送测试请求", exact: true }).click();
  await expect(page.locator(".llm-probe")).toContainText("调用完成");
  await expect(page.locator(".llm-probe pre").first()).toContainText("你好 OK");
  checks.push(
    "explicit inference probe shows streamed content and token usage",
  );
  await page.screenshot({ path: resolve(run, "api-models.png") });
  await page
    .locator(".settings-footer")
    .getByRole("button", { name: "关闭", exact: true })
    .click();
  await page.getByRole("menuitem", { name: "设置", exact: true }).click();
  await page
    .locator(".menu-popup button")
    .filter({ hasText: "缓存与存储" })
    .click();
  await page.getByRole("button", { name: "API 与模型", exact: true }).click();
  await expect(page.locator(".llm-model-list .llm-model-row")).toHaveCount(2);
  await page
    .locator(".llm-model-list .llm-model-row")
    .filter({ hasText: "模型一" })
    .getByRole("button", { name: "配置", exact: true })
    .click();
  await expect(page.locator('[id="llm-value-temperature"]')).toHaveValue("0");
  await page.screenshot({ path: resolve(run, "model-parameters.png") });
  checks.push(
    "saved model configuration is restored after settings close and reopen",
  );
  assert.deepEqual(errors, []);
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify({ passed: true, checks, errors }, null, 2),
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
      { passed: false, checks, errors, error: String(error) },
      null,
      2,
    ),
  );
  throw error;
} finally {
  await browser?.close();
  vite?.kill();
  await engine.stop();
  await mock.close();
}
