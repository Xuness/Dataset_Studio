/* global document, innerWidth */
// Isolated 16-image workflow: real SDK/HTTP/SQLite and a loopback provider.
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { execFile, spawn } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, open, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { chromium, expect } from "@playwright/test";
import { EngineFixture, sleep } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";
import { network } from "./llm-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local/test-runs/smoke-evaluation-workflow-" + Date.now(),
);
await mkdir(run, { recursive: true });
await promisify(execFile)(
  "python",
  [resolve(root, "tooling/ranking-fixture.py"), resolve(run, "fixture"), "128"],
  { cwd: root, windowsHide: true },
);
const fixture = JSON.parse(
  await readFile(resolve(run, "fixture/fixture.json"), "utf8"),
);
const engine = new EngineFixture(root, resolve(run, "state"));
const { StudioClient } = await clientFixture(root, resolve(run, "sdk"));
const mock = { calls: [], abstain: true };
const server = createServer(async (req, res) => {
  try {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    const body = JSON.parse(Buffer.concat(chunks));
    const parts = body.messages
      .filter((m) => m.role === "user")
      .flatMap((m) => m.content);
    const ids = parts.map((p) => p.text).filter((v) => /^img\d{2}$/.test(v));
    assert.ok(ids.length >= 2 && ids.length <= 16);
    assert.equal(parts.filter((v) => v.image_url).length, ids.length);
    const unjudgeable = mock.abstain ? ids.slice(0, 2) : [];
    mock.calls.push({ ids, unjudgeable });
    const text = JSON.stringify({
      schema_version: 1,
      tiers: [ids.filter((v) => !unjudgeable.includes(v))],
      elite_candidates: [ids.at(-1)],
      unjudgeable: unjudgeable.map((id) => ({
        id,
        reason: "合成测试：暂时无法可靠判断",
      })),
    });
    res.writeHead(200, {
      "content-type": "application/json",
      "x-request-id": "workflow-mock",
    });
    res.end(
      JSON.stringify({
        id: "fixture",
        model: "fixture",
        choices: [
          {
            index: 0,
            message: { content: text },
            finish_reason: "stop",
          },
        ],
        usage: { prompt_tokens: 12, completion_tokens: 6, total_tokens: 18 },
      }),
    );
  } catch (e) {
    res.writeHead(500);
    res.end(String(e));
  }
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
const checks = [],
  errors = [],
  requests = [];
const faults = new Set();
let client, project, browser, page, vite;
const url = "http://127.0.0.1:1449";
const stagePath = (id) =>
  "/v1/projects/" + project.id + "/aesthetic/stages/" + id;
async function openModule() {
  await page.getByRole("button", { name: "美学排序", exact: true }).click();
}
async function view(name) {
  await page.getByLabel("美学工作视图", { exact: true }).selectOption(name);
}
async function screenshot(name) {
  await page.screenshot({ path: resolve(run, name + ".png") });
}
try {
  await engine.start();
  client = new StudioClient(engine.connection);
  project = await client.createProject({
    name: "评审接入小样本",
    parent_directory: null,
  });
  const source = await engine.api(
    "/v1/projects/" + project.id + "/sources",
    "POST",
    {
      kind: "danbooru",
      name: "合成图源",
      index_root: fixture.lake,
      media_root: fixture.lake,
    },
  );
  const keys = fixture.objects
    .filter((o) => o.number >= 32 && o.number < 96 && o.number % 4 === 0)
    .map((o) => ({ source_id: source.id, asset_id: o.sha }));
  assert.equal(keys.length, 16);
  const selection = await client.selection(project.id);
  await client.changeSelection(project.id, {
    expected_revision: selection.revision,
    add: keys,
    remove: [],
    clear: true,
  });
  const collection = await client.createCollection(project.id, "十六张 G 图");
  const provider = await client.llm.providers.save({
    expected_revision: 0,
    api_key: "fixture-only",
    config: {
      name: "本机工作流验证",
      kind: "openai_compatible",
      base_url: "http://127.0.0.1:" + server.address().port,
      enabled: true,
      headers: {},
      network: {
        ...network,
        request_timeout_ms: 30000,
        idle_timeout_ms: 30000,
        rate_limit_retries: 0,
      },
    },
  });
  const model = await client.llm.models.save({
    expected_revision: 0,
    provider_id: provider.id,
    config: {
      name: "本机合成评审",
      remote_model_id: "fixture",
      protocol: "openai_chat",
      enabled: true,
      parameters: {},
      capability_overrides: { input_image: "supported" },
    },
  });
  const prompt = await client.llm.systemPrompts.save({
    expected_revision: 0,
    config: {
      name: "测试标准",
      description: "仅验证协议",
      text: "按任务格式返回。此预设仅用于本机 mock 测试。",
    },
  });
  const removedPrompt = await client.llm.systemPrompts.save({
    expected_revision: 0,
    config: {
      name: "将删除的测试标准",
      description: "仅用于缺失依赖验证",
      text: "mock",
    },
  });
  const log = await open(resolve(run, "vite.log"), "a");
  vite = spawn(
    process.execPath,
    [
      resolve(root, "apps/desktop/node_modules/vite/bin/vite.js"),
      "--port",
      "1449",
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
  for (let n = 0; n < 100; n++) {
    if ((await fetch(url).catch(() => null))?.ok) break;
    if (vite.exitCode !== null) throw new Error("Vite stopped");
    await sleep(100);
  }
  browser = await chromium.launch({ channel: "msedge", headless: true });
  const context = await browser.newContext({
    viewport: { width: 2560, height: 1440 },
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
    const path = new URL(request.url()).pathname;
    const operation =
      request.method() === "POST"
        ? path.endsWith("/stages")
          ? "create"
          : path.endsWith("/disposition")
            ? "decision"
            : path.endsWith("/experiments")
              ? "experiment"
              : ""
        : "";
    if (operation) requests.push({ operation, value: request.postDataJSON() });
    const response = await fetch(request.url(), {
      method: request.method(),
      headers,
      ...(request.postData() ? { body: request.postData() } : {}),
    });
    const body = Buffer.from(await response.arrayBuffer());
    if (operation && response.ok && faults.has(operation)) {
      faults.delete(operation);
      return route.abort("failed");
    }
    await route.fulfill({
      status: response.status,
      headers: { ...Object.fromEntries(response.headers), ...cors },
      body,
    });
  });
  page = await context.newPage();
  page.setDefaultTimeout(20000);
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(url);
  await page.locator(".recent-row").filter({ hasText: project.name }).click();
  await openModule();
  await page.getByRole("button", { name: "新建评审", exact: true }).click();
  await page.getByLabel("阶段名称", { exact: true }).fill("小样本评审");
  await page
    .getByLabel("候选工作集", { exact: true })
    .selectOption(collection.id);
  await page.getByLabel("评审模型", { exact: true }).selectOption(model.id);
  await page
    .getByLabel("审美标准（System Prompt）", { exact: true })
    .selectOption(prompt.id);
  await page.getByLabel("调用次数上限", { exact: true }).fill("6");
  await page.getByLabel("请求并发上限", { exact: true }).fill("1");
  await page.getByRole("button", { name: "预检输入", exact: true }).click();
  await expect(page.getByLabel("输入预检结果")).toContainText(
    "基础容量预检通过",
  );
  assert.equal(mock.calls.length, 0);
  await page.getByLabel("阶段名称", { exact: true }).fill("小样本评审 A");
  await expect(page.getByLabel("输入预检结果")).toHaveCount(0);
  await page.getByRole("button", { name: "预检输入", exact: true }).click();
  await expect(page.getByLabel("输入预检结果")).toContainText("16");
  await screenshot("preflight-2560");
  checks.push(
    "preflight reports authoritative capacity, invalidates on edit and sends no model request",
  );
  await page
    .getByLabel("审美标准（System Prompt）", { exact: true })
    .selectOption(removedPrompt.id);
  await page.getByRole("button", { name: "预检输入", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "创建并冻结候选", exact: true }),
  ).toBeEnabled();
  await client.llm.systemPrompts.remove(
    removedPrompt.id,
    removedPrompt.revision,
  );
  await page
    .getByRole("button", { name: "创建并冻结候选", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "放弃未完成创建", exact: true }),
  ).toBeEnabled();
  await page
    .getByRole("button", { name: "放弃未完成创建", exact: true })
    .click();
  await expect(
    page.getByLabel("审美标准（System Prompt）", { exact: true }),
  ).toBeEnabled();
  assert.equal((await client.aesthetic.stages(project.id)).items.length, 0);
  await page
    .getByLabel("审美标准（System Prompt）", { exact: true })
    .selectOption(prompt.id);
  await page.getByRole("button", { name: "预检输入", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "创建并冻结候选", exact: true }),
  ).toBeEnabled();
  checks.push(
    "deleted prompt after preflight releases a definitively failed creation and permits correction",
  );
  faults.add("create");
  await page
    .getByRole("button", { name: "创建并冻结候选", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "恢复此次创建", exact: true }),
  ).toBeEnabled();
  await page
    .getByRole("button", { name: "放弃未完成创建", exact: true })
    .click();
  await expect(page.locator(".evaluation-create")).toContainText("阶段已建成");
  await expect(
    page.getByRole("button", { name: "恢复此次创建", exact: true }),
  ).toBeEnabled();
  await page.reload();
  await page.getByRole("button", { name: "新建评审", exact: true }).click();
  await page.getByRole("button", { name: "恢复此次创建", exact: true }).click();
  await expect(page.locator(".aesthetic-main")).toContainText("待开始", {
    timeout: 30000,
  });
  const stage = (await client.aesthetic.stages(project.id)).items.find(
    (s) => s.name === "小样本评审 A",
  );
  assert.ok(stage);
  const creates = requests.filter(
    (r) => r.operation === "create" && r.value.system_prompt_id === prompt.id,
  );
  assert.equal(creates.length, 2);
  assert.deepEqual(creates[0].value, creates[1].value);
  assert.match(creates[0].value.expected_input_version, /^[a-f0-9]{64}$/);
  assert.equal((await client.aesthetic.stages(project.id)).items.length, 1);
  assert.equal(mock.calls.length, 0);
  checks.push(
    "lost creation response survives reload; exact request and input version recover one unstarted stage",
  );
  await page.getByRole("button", { name: "开始评审", exact: true }).click();
  await engine.wait(
    stagePath(stage.id),
    (s) => s.state === "needs_attention",
    60000,
  );
  const pending = await client.aesthetic.candidates(
    project.id,
    stage.id,
    false,
    undefined,
    undefined,
    "needs_review",
  );
  assert.equal(pending.items.length, 2);
  const excluded = pending.items[0],
    rejudge = pending.items[1];
  assert.equal(
    (await client.aesthetic.candidate(project.id, stage.id, excluded.ordinal))
      .disposition,
    "needs_review",
  );
  await assert.rejects(() =>
    client.aesthetic.candidate(project.id, stage.id, 100000),
  );
  await assert.rejects(() =>
    engine.api(stagePath(stage.id) + "/candidates?disposition=bogus"),
  );
  await page.getByRole("button", { name: "异常候选", exact: true }).click();
  await page
    .getByRole("button", {
      name: "处置候选 " + (excluded.ordinal + 1),
      exact: true,
    })
    .click();
  await page.getByLabel("候选后续安排").selectOption("exclude");
  await page.getByLabel("候选处置原因").fill("模".repeat(334));
  await expect(
    page.getByRole("button", { name: "保存候选处置", exact: true }),
  ).toBeDisabled();
  await page
    .getByLabel("候选处置原因")
    .fill("保留证据，本阶段明确排除这张合成图");
  await page.getByRole("tab", { name: "冻结标准", exact: true }).click();
  await page.getByRole("tab", { name: "候选处置", exact: true }).click();
  await expect(page.getByLabel("候选处置原因")).toHaveValue(
    "保留证据，本阶段明确排除这张合成图",
  );
  faults.add("decision");
  await page.getByRole("button", { name: "保存候选处置", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "重试确认处置", exact: true }),
  ).toBeEnabled();
  await page.reload();
  await page.getByRole("button", { name: "重试确认处置", exact: true }).click();
  await expect(page.getByLabel("候选处置原因")).toHaveValue("");
  const decisions = requests.filter((r) => r.operation === "decision");
  assert.equal(decisions.length, 2);
  assert.deepEqual(decisions[0].value, decisions[1].value);
  const db = new DatabaseSync(resolve(project.directory, "evaluation.sqlite"), {
    readOnly: true,
  });
  assert.equal(
    db
      .prepare(
        "SELECT count(*) n FROM candidate_decisions WHERE stage_id=? AND ordinal=?",
      )
      .get(stage.id, excluded.ordinal).n,
    1,
  );
  db.close();
  checks.push(
    "candidate filter and exact lookup, UTF-8 limit, panel draft and lost-response recovery append one decision",
  );
  await page
    .getByRole("button", {
      name: "处置候选 " + (rejudge.ordinal + 1),
      exact: true,
    })
    .click();
  await page.getByLabel("候选后续安排").selectOption("rejudge");
  await page.getByLabel("候选处置原因").fill("追加同 Rating 比较");
  const beforeRejudge = mock.calls.length;
  await page.getByRole("button", { name: "保存候选处置", exact: true }).click();
  await expect(page.getByLabel("候选处置原因")).toHaveValue("");
  assert.equal(mock.calls.length, beforeRejudge);
  assert.equal(
    (await client.aesthetic.candidate(project.id, stage.id, rejudge.ordinal))
      .disposition,
    "rejudge",
  );
  mock.abstain = false;
  await page.getByRole("button", { name: "开始评审", exact: true }).click();
  const finished = await engine.wait(
    stagePath(stage.id),
    (s) => s.state === "completed_with_exclusions",
    60000,
  );
  assert.equal(finished.excluded, 1);
  assert.equal(finished.unresolved, 0);
  assert.equal(finished.attempts, 2);
  await page.getByLabel("候选状态").selectOption("excluded");
  await expect(
    page.getByRole("button", {
      name: "处置候选 " + (excluded.ordinal + 1),
      exact: true,
    }),
  ).toBeVisible();
  await page
    .getByRole("button", {
      name: "处置候选 " + (excluded.ordinal + 1),
      exact: true,
    })
    .click();
  await expect(page.locator(".aesthetic-header")).toContainText(
    "本阶段已完成（含排除项）",
  );
  await expect(page.locator(".evaluation-candidate img").first()).toBeVisible();
  await screenshot("candidate-decisions-2560");
  checks.push(
    "explicit rejudge remains unsent until start; same-rating batch completes stage with one exclusion",
  );
  const beforeOffline = mock.calls.length;
  await view("experiments");
  await page.getByLabel("实验名称", { exact: true }).fill("同证据双估计器");
  await page.getByLabel("实验来源阶段").selectOption(stage.id);
  await page
    .getByRole("button", { name: "所有变体使用此阶段", exact: true })
    .click();
  faults.add("experiment");
  await page.getByRole("button", { name: "保存实验定义", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "恢复实验创建", exact: true }),
  ).toBeEnabled();
  await page.reload();
  await page.getByRole("button", { name: "恢复实验创建", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "运行全部变体", exact: true }),
  ).toBeEnabled();
  const experiment = (await client.aesthetic.analysis.experiments(project.id))
    .items[0];
  assert.equal(experiment.request.variants.length, 2);
  assert.equal(
    experiment.inputs[0].evidence_watermark,
    experiment.inputs[1].evidence_watermark,
  );
  const definitions = requests.filter((r) => r.operation === "experiment");
  assert.equal(definitions.length, 2);
  assert.deepEqual(definitions[0].value, definitions[1].value);
  await page.getByRole("button", { name: "运行全部变体", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "查看排名", exact: true }),
  ).toHaveCount(2, { timeout: 60000 });
  await page.getByRole("button", { name: "运行全部变体", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "运行全部变体", exact: true }),
  ).toBeEnabled();
  const jobs = (
    await client.aesthetic.analysis.jobs(project.id, {
      experiment_id: experiment.id,
    })
  ).items;
  assert.equal(jobs.length, 2);
  assert.ok(jobs.every((j) => j.state === "completed"));
  assert.equal(mock.calls.length, beforeOffline);
  await screenshot("experiment-results-2560");
  checks.push(
    "two variants freeze shared evidence, survive lost definition response and run idempotently without model calls",
  );
  await page
    .getByRole("button", { name: "查看排名", exact: true })
    .first()
    .click();
  await expect(page.getByLabel("美学工作视图", { exact: true })).toHaveValue(
    "ranking",
  );
  await expect(page.locator(".ranking-image-grid img").first()).toBeVisible();
  await view("experiments");
  await page.getByLabel("实验对照 A").selectOption(jobs[0].id);
  await page.getByLabel("实验对照 B").selectOption(jobs[1].id);
  await page
    .getByRole("button", { name: "在实验对照中打开", exact: true })
    .click();
  await expect(page.getByLabel("美学工作视图", { exact: true })).toHaveValue(
    "comparison",
  );
  await expect(page.getByLabel("基准快照 A")).toHaveValue(jobs[0].id);
  await expect(page.getByLabel("对照快照 B")).toHaveValue(jobs[1].id);
  checks.push(
    "completed experiment results open their ranking and preselect both snapshots for comparison",
  );
  await view("evaluation");
  await page.getByRole("button", { name: "异常候选", exact: true }).click();
  await page.setViewportSize({ width: 1707, height: 928 });
  const bounds = await page.evaluate(() => ({
    width: innerWidth,
    scroll: document.documentElement.scrollWidth,
    last:
      document.querySelector(".wb-status-bar")?.getBoundingClientRect()
        .bottom ?? 0,
  }));
  assert.ok(bounds.scroll <= bounds.width + 1);
  await screenshot("candidate-decisions-compact");
  checks.push("compact viewport has no document horizontal overflow");
  assert.equal(mock.calls.length, 2);
  assert.deepEqual(errors, []);
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      {
        passed: true,
        checks,
        errors,
        modelCalls: mock.calls.length,
        images: keys.length,
        stage: finished,
        experiment: experiment.id,
        project: project.directory,
      },
      null,
      2,
    ),
  );
  console.log(
    JSON.stringify({
      passed: true,
      checks: checks.length,
      modelCalls: mock.calls.length,
      report: resolve(run, "report.json"),
    }),
  );
} catch (e) {
  await page?.screenshot({ path: resolve(run, "failure.png") }).catch(() => {});
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      { passed: false, checks, errors, error: String(e) },
      null,
      2,
    ),
  );
  throw e;
} finally {
  await browser?.close();
  vite?.kill();
  client?.dispose();
  await engine.stop();
  server.closeAllConnections();
  await new Promise((done) => server.close(done));
}
