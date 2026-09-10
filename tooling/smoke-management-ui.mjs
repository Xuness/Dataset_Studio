// Isolated headless Edge; no user browser profile, clipboard or desktop window is controlled.
import assert from "node:assert/strict";
import { spawn, execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, open, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";
import { EngineFixture, sleep } from "./engine-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(root, ".local/test-runs/smoke-management-ui-" + Date.now());
const state = resolve(run, "state");
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
  errors = [];
let browser, page, vite;
const url = "http://127.0.0.1:1431";
async function shot(name) {
  await page.screenshot({ path: resolve(run, name + ".png") });
}
async function more(label, item) {
  await page
    .getByRole("button", { name: label + "更多操作", exact: true })
    .click();
  await page.getByRole("menuitem", { name: item, exact: true }).click();
}
try {
  await engine.start();
  const project = await engine.api("/v1/projects", "POST", {
    name: "对象管理界面验收",
  });
  const base = "/v1/projects/" + project.id;
  const source = await engine.api(base + "/sources", "POST", {
    kind: "danbooru",
    name: "验收数据湖",
    index_root: fixture.lake,
    media_root: fixture.lake,
  });
  const assets = (
    await engine.api(
      base + "/assets?source_id=" + source.id + "&order=asset_key_asc&limit=8",
    )
  ).items;
  await engine.api(base + "/selection", "PATCH", {
    expected_revision: 0,
    add: assets.slice(0, 4).map((a) => a.key),
    remove: [],
    clear: false,
  });
  const plain = await engine.api(base + "/collections", "POST", {
    name: "保留的工作集",
  });
  const definition = await engine.api(base + "/queries", "POST", {
    name: "稍后删除的查询",
    spec: {
      version: 2,
      source_ids: [source.id],
      conditions: [],
      observation_rule: "current_post",
      order: "asset_key_asc",
      input_scope: {
        project_id: project.id,
        target: { kind: "workset", collection_id: plain.id },
      },
    },
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
      parameters: { mode: "rank", artist_enabled: false, cohort_minimum: 8 },
    },
  });
  await engine.wait(
    base + "/jobs",
    (r) => r.items.some((j) => j.id === job.id && j.status === "succeeded"),
    60000,
  );
  const ranking = (await engine.api(base + "/artifacts?limit=32")).items.find(
    (a) => a.kind === "ranking_table",
  );
  await engine.api(base + "/objects/artifact/" + ranking.id, "PATCH", {
    expected_revision: 0,
    name: "排名成果UI",
    notes: "保留原始参数",
  });
  const workset = await engine.api(
    base + "/artifacts/" + ranking.id + "/ranking/worksets",
    "POST",
    {
      idempotency_key: crypto.randomUUID(),
      name: "排名候选UI",
      filter: { rating: "g", eligibility: "eligible", top: 8, order: "main" },
    },
  );
  await engine.api(base + "/drafts/studio.session/default", "PUT", {
    schema_version: 1,
    expected_revision: 0,
    value: {
      order: "asset_key_asc",
      moduleId: "core.browser",
      panels: [],
      scope: { kind: "source", id: source.id, name: source.name },
      focusKey: null,
      view: "grid",
      position: null,
      inspectorTab: "properties",
      managementTarget: null,
    },
  });
  await engine.api(base + "/selection/history", "POST", {
    action: "clear",
    expected_revision: 1,
  });
  await engine.api(base + "/close", "POST");

  const log = await open(resolve(run, "vite.log"), "a");
  vite = spawn(
    process.execPath,
    [
      resolve(root, "apps/desktop/node_modules/vite/bin/vite.js"),
      "--port",
      "1431",
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
    if (vite.exitCode !== null) throw new Error("Fixture Vite exited");
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
  await expect(page.locator(".asset-card")).toHaveCount(48, { timeout: 30000 });
  const firstChoice = page
    .locator(".asset-card")
    .first()
    .locator(".asset-check");
  await firstChoice.click();
  await expect
    .poll(async () => (await engine.api(base + "/selection")).count)
    .toBe(3);
  await expect(
    page.getByRole("button", { name: "撤销选择", exact: true }),
  ).toBeEnabled();
  await page.keyboard.press("Control+z");
  await expect
    .poll(async () => (await engine.api(base + "/selection")).count)
    .toBe(4);
  await expect(
    page.getByRole("button", { name: "重做选择", exact: true }),
  ).toBeEnabled();
  await page.keyboard.press("Control+y");
  await expect
    .poll(async () => (await engine.api(base + "/selection")).count)
    .toBe(3);
  checks.push("actual checkbox selection supports Ctrl+Z and Ctrl+Y");
  await page
    .locator(".browser-view")
    .getByRole("button", { name: "下一页", exact: true })
    .click();
  await expect(page.locator(".browser-view").getByText("第 2 页", { exact: true })).toContainText(
    /第\s*2\s*页/,
  );
  const secondPageAnchor = await page
    .locator(".asset-card")
    .first()
    .getByRole("button", { name: /^查看 / })
    .getAttribute("aria-label");
  assert.ok(secondPageAnchor);

  await more("验收数据湖", "重命名与备注…");
  await expect(
    page.getByRole("tab", { name: "管理", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  const selectionRevision = (await engine.api(base + "/selection")).revision;
  const nameInput = page.getByLabel("对象名称", { exact: true });
  await expect(nameInput).toHaveValue("验收数据湖");
  await nameInput.press("End");
  await nameInput.pressSequentially("X");
  await nameInput.press("Control+z");
  await expect(nameInput).toHaveValue("验收数据湖");
  assert.equal(
    (await engine.api(base + "/selection")).revision,
    selectionRevision,
  );
  await nameInput.fill("重新命名的数据湖");
  await page.getByLabel("对象备注", { exact: true }).fill("给当前项目的备注");
  await page.getByRole("button", { name: "保存修改", exact: true }).click();
  await expect
    .poll(async () => (await engine.api(base + "/sources")).items[0].name)
    .toBe("重新命名的数据湖");
  await expect(page.locator(".browser-view").getByText("第 2 页", { exact: true })).toContainText(
    /第\s*2\s*页/,
  );
  await expect(page.locator(".asset-card").first().getByRole("button", { name: /^查看 / })).toHaveAttribute(
    "aria-label",
    secondPageAnchor,
  );
  await expect(page.locator(".asset-card")).toHaveCount(48);
  await shot("01-source-management");
  checks.push(
    "row menu opens adjacent management tab; text undo does not alter selection; names and notes save",
  );

  await page.getByLabel("搜索工作集", { exact: true }).fill("排名候选UI");
  await expect(page.locator(".workset-tree .managed-list-row")).toHaveCount(1);
  await more("排名候选UI", "管理与来源详情");
  await expect(page.locator(".management-panel")).toContainText(
    "此工作集保存自元数据排名筛选",
  );
  await expect(page.locator(".management-panel")).toContainText("前 8 名");
  await page
    .locator(".management-reference")
    .filter({ hasText: "排名成果UI" })
    .click();
  await expect(page.locator(".management-heading")).toContainText("排名成果UI");
  await page
    .getByRole("button", { name: "删除此计算成果…", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "确认删除", exact: true }),
  ).toBeDisabled();
  await expect(page.locator(".management-panel")).toContainText(
    "仍有对象需要使用它",
  );
  await shot("02-dependency-block");
  await page
    .locator(".management-reference")
    .filter({ hasText: "排名候选UI" })
    .click();
  await page
    .getByRole("button", { name: "删除此工作集…", exact: true })
    .click();
  await page.getByRole("button", { name: "确认删除", exact: true }).click();
  await expect
    .poll(async () =>
      (await engine.api(base + "/collections")).items.some(
        (w) => w.id === workset.id,
      ),
    )
    .toBe(false);
  await page.getByLabel("搜索工作集", { exact: true }).fill("");
  checks.push(
    "workset search and provenance navigate to dependencies; deleting a workset releases its artifact reference",
  );

  await page.getByRole("button", { name: "项目查询", exact: true }).click();
  await page
    .getByLabel("已保存查询", { exact: true })
    .selectOption(definition.id);
  await more("已保存查询", "删除保存的查询…");
  await page.getByRole("button", { name: "确认删除", exact: true }).click();
  await expect(page.getByLabel("已保存查询", { exact: true })).toHaveValue("");
  await expect(page.locator(".query-panel")).toContainText("原保存查询已删除");
  checks.push(
    "deleting a saved query retains the current editable conditions as a new draft",
  );

  await page.getByRole("button", { name: "项目成果", exact: true }).click();
  await page.getByLabel("搜索计算成果", { exact: true }).fill("排名成果UI");
  await expect(page.locator(".artifact-list .managed-list-row")).toHaveCount(1);
  await more("排名成果UI", "重命名与备注…");
  await page.getByLabel("对象名称", { exact: true }).fill("已整理的排名");
  await page.getByLabel("对象备注", { exact: true }).fill("可直接复用原始参数");
  await page.getByRole("button", { name: "保存修改", exact: true }).click();
  await expect
    .poll(
      async () => (await engine.api(base + "/artifacts/" + ranking.id)).name,
    )
    .toBe("已整理的排名");
  await page.getByLabel("搜索计算成果", { exact: true }).fill("已整理");
  await expect(page.locator(".artifact-list .managed-list-row")).toHaveCount(1);
  await expect(
    page
      .locator(".management-panel")
      .getByRole("button", { name: "复制路径", exact: true })
      .first(),
  ).toBeVisible();
  const jobsBefore = (await engine.api(base + "/jobs")).items.length;
  await page
    .locator(".management-panel")
    .getByRole("button", { name: "用这些参数配置新任务", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: /Danbooru 元数据排名/ }),
  ).toBeVisible();
  await expect(
    page.getByRole("radio", { name: "仅计算排名", exact: true }),
  ).toBeChecked();
  assert.equal((await engine.api(base + "/jobs")).items.length, jobsBefore);
  checks.push(
    "artifact search, renamed labels and file-location controls are visible; reusing parameters does not submit a job",
  );

  await page.getByRole("button", { name: "保存当前参数", exact: true }).click();
  await page.getByLabel("预设名称", { exact: true }).fill("界面参数预设");
  await page
    .getByLabel("预设备注", { exact: true })
    .fill("应保留原来的邻域设置");
  await page.getByRole("button", { name: "保存预设", exact: true }).click();
  await expect(
    page.getByLabel("选择参数预设", { exact: true }),
  ).not.toHaveValue("");
  await page.locator(".ranking-advanced summary").click();
  await page.getByLabel("邻域最低有效数量", { exact: true }).fill("64");
  await more("参数预设", "修改名称与备注…");
  await page.getByLabel("预设名称", { exact: true }).fill("改名后的预设");
  await page.getByRole("button", { name: "保存预设", exact: true }).click();
  await page.getByRole("button", { name: "应用预设", exact: true }).click();
  await expect(
    page.getByLabel("邻域最低有效数量", { exact: true }),
  ).toHaveValue("8");
  await shot("03-ranking-preset");
  await more("参数预设", "删除预设…");
  await page.getByRole("button", { name: "确认删除预设", exact: true }).click();
  await expect
    .poll(
      async () =>
        (await engine.api(base + "/presets?operator_id=danbooru.metarecall"))
          .items.length,
    )
    .toBe(0);
  assert.equal((await engine.api(base + "/jobs")).items.length, jobsBefore);
  checks.push(
    "preset create, metadata edit, apply and delete retain the saved parameter values without execution",
  );

  await more("重新命名的数据湖", "取消与项目关联…");
  await page.getByRole("button", { name: "确认取消关联", exact: true }).click();
  await expect
    .poll(async () => (await engine.api(base + "/sources")).items.length)
    .toBe(0);
  await expect(page.locator(".management-panel")).toContainText("未关联");
  await page
    .getByRole("button", { name: "重新关联到项目", exact: true })
    .click();
  await expect
    .poll(async () => (await engine.api(base + "/sources")).items.length)
    .toBe(1);
  await expect(page.locator(".source-tree-row")).toContainText(
    "重新命名的数据湖",
  );
  checks.push(
    "source unlink and reconnect are available through the same management panel",
  );

  await page.getByRole("menuitem", { name: "设置", exact: true }).click();
  await page
    .locator(".menu-popup button")
    .filter({ hasText: "编辑与撤销" })
    .click();
  await page.getByLabel("撤销步数上限", { exact: true }).fill("2");
  await page.getByRole("button", { name: "保存撤销设置", exact: true }).click();
  await expect
    .poll(async () => (await engine.api("/v1/settings/editing")).undo_limit)
    .toBe(2);
  await shot("04-undo-settings");
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "关闭", exact: true })
    .last()
    .click();
  await page.reload();
  await expect(page.locator(".document-tab.active")).toContainText(
    project.name,
  );
  assert.equal((await engine.api("/v1/settings/editing")).undo_limit, 2);
  assert.ok((await engine.api(base + "/selection/history")).undo_steps <= 2);
  checks.push("undo step limit is configurable and survives reload");

  await page.getByRole("button", { name: "项目成果", exact: true }).click();
  await page.getByLabel("搜索计算成果", { exact: true }).fill("已整理");
  await more("已整理的排名", "删除计算结果…");
  await page.getByRole("button", { name: "确认删除", exact: true }).click();
  await expect
    .poll(
      async () => (await engine.api(base + "/artifacts/" + ranking.id)).state,
    )
    .toBe("released");
  await page.getByLabel("成果状态", { exact: true }).selectOption("released");
  await expect(page.locator(".artifact-list")).toContainText("已整理的排名");
  await expect(page.locator(".management-panel")).toContainText("0 B");
  await shot("05-released-artifact");
  checks.push(
    "actual result deletion updates the visible list and retains an inspectable released record",
  );

  await page.getByTitle("项目任务", { exact: true }).click();
  await expect(page.locator(".tasks-panel")).toBeVisible();
  await expect(page.locator(".tasks-panel")).toContainText("排名已删除");
  const taskName = (await engine.api(base + "/job-history")).items[0].object
    .name;
  await more(taskName, "归档此任务");
  await page
    .getByLabel("任务状态筛选", { exact: true })
    .selectOption("archived");
  await expect(page.locator(".task-row")).toHaveCount(1);
  await more(taskName, "恢复到任务列表");
  await page.getByLabel("任务状态筛选", { exact: true }).selectOption("");
  await expect(page.locator(".task-row")).toHaveCount(1);
  await shot("06-task-history");
  checks.push(
    "task filtering, archive and restore preserve history and reflect removed results",
  );

  assert.deepEqual(errors, []);
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify(
      {
        passed: true,
        checks,
        errors,
        screenshots: [
          "01-source-management",
          "02-dependency-block",
          "03-ranking-preset",
          "04-undo-settings",
          "05-released-artifact",
          "06-task-history",
        ],
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
  if (page) await shot("failure").catch(() => {});
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
}
