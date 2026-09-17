import assert from "node:assert/strict";
import { resolve } from "node:path";
import { expect } from "@playwright/test";

export async function checkSystemPromptsUI(page, engine, mock, run, checks) {
  const text = '  你是数据集助手 🧪\n保留 {{literal}} 与 "引号"。\n  ';
  const promptsPage = () =>
    page.getByRole("button", { name: "System Prompt", exact: true }).click();
  await promptsPage();
  await page.getByRole("button", { name: "新建预设", exact: true }).click();
  await page.getByLabel("预设名称", { exact: true }).fill("图像描述指令");
  await page.getByLabel("预设备注", { exact: true }).fill("用于图片描述任务");
  await page.getByLabel("System Prompt 正文", { exact: true }).fill(text);
  await page.getByRole("button", { name: "编辑与撤销", exact: true }).click();
  await promptsPage();
  await expect(
    page.getByLabel("System Prompt 正文", { exact: true }),
  ).toHaveValue(text);
  await page.screenshot({ path: resolve(run, "system-prompt-editor.png") });
  await page.getByRole("button", { name: "保存预设", exact: true }).click();
  await expect(
    page.getByText("System Prompt 预设已保存。", { exact: true }),
  ).toBeVisible();
  let prompt = (await engine.api("/v1/llm/system-prompts")).items[0];
  assert.equal(prompt.config.text, text);
  checks.push(
    "system prompt create, exact multiline Unicode persistence and draft retention across settings pages",
  );
  await page.getByRole("button", { name: "复制预设", exact: true }).click();
  await page.getByLabel("预设名称", { exact: true }).fill("副本指令");
  await expect(
    page.getByLabel("System Prompt 正文", { exact: true }),
  ).toHaveValue(text);
  await page.getByRole("button", { name: "保存预设", exact: true }).click();
  await expect(page.locator(".system-prompt-list button")).toHaveCount(2);
  await page.getByRole("button", { name: "编辑预设", exact: true }).click();
  await page.getByLabel("预设名称", { exact: true }).fill("已重命名副本");
  await page.getByRole("button", { name: "保存预设", exact: true }).click();
  await expect(page.locator(".system-prompt-detail h4")).toHaveText(
    "已重命名副本",
  );
  await page
    .getByLabel("搜索 System Prompt 预设", { exact: true })
    .fill("已重命名");
  await expect(page.locator(".system-prompt-list button")).toHaveCount(1);
  await page.getByRole("button", { name: "删除预设", exact: true }).click();
  await page.getByRole("button", { name: "取消删除", exact: true }).click();
  assert.equal((await engine.api("/v1/llm/system-prompts")).items.length, 2);
  await page.getByRole("button", { name: "删除预设", exact: true }).click();
  await page.getByRole("button", { name: "确认删除预设", exact: true }).click();
  await expect(
    page.getByText("System Prompt 预设已删除。", { exact: true }),
  ).toBeVisible();
  await page.getByLabel("搜索 System Prompt 预设", { exact: true }).fill("");
  await expect(page.locator(".system-prompt-list button")).toHaveCount(1);
  checks.push(
    "independent copy, rename, search, deletion confirmation and cancellation",
  );
  await page.getByRole("button", { name: "编辑预设", exact: true }).click();
  await page
    .getByLabel("System Prompt 正文", { exact: true })
    .fill("未保存的本地草稿");
  prompt = await engine.api("/v1/llm/system-prompts", "POST", {
    id: prompt.id,
    expected_revision: prompt.revision,
    config: { ...prompt.config, description: "另一窗口更新的备注" },
  });
  await page.getByRole("button", { name: "保存预设", exact: true }).click();
  await expect(page.locator(".settings-main")).toContainText(
    "REVISION_CONFLICT",
  );
  await expect(
    page.getByLabel("System Prompt 正文", { exact: true }),
  ).toHaveValue("未保存的本地草稿");
  assert.equal(
    (await engine.api("/v1/llm/system-prompts/" + prompt.id)).config.text,
    text,
  );
  await page.getByRole("button", { name: "放弃修改", exact: true }).click();
  await page
    .locator(".settings-footer")
    .getByRole("button", { name: "关闭", exact: true })
    .click();
  await page.getByRole("menuitem", { name: "设置", exact: true }).click();
  await page
    .locator(".menu-popup button")
    .filter({ hasText: "System Prompt" })
    .click();
  await expect(page.locator(".system-prompt-preview")).toHaveText(text, {
    useInnerText: false,
  });
  await expect(page.locator(".system-prompt-detail")).toContainText(
    "另一窗口更新的备注",
  );
  await page.screenshot({ path: resolve(run, "system-prompts.png") });
  checks.push(
    "concurrent edit rejects stale revision without losing draft or overwriting saved text; reopen restores persisted preset",
  );
  await page.getByRole("button", { name: "API 与模型", exact: true }).click();
  await page
    .locator(".llm-model-list .llm-model-row")
    .filter({ hasText: "模型一" })
    .getByRole("button", { name: "调用检查" })
    .click();
  await page
    .getByLabel("测试 System Prompt 预设", { exact: true })
    .selectOption(prompt.id);
  await page.getByLabel("测试消息", { exact: true }).fill("这次任务请描述图片");
  const before = mock.state.calls.length;
  await page
    .getByRole("button", { name: "预览请求（不联网）", exact: true })
    .click();
  await expect(page.locator(".llm-request-preview")).toContainText(
    "原生请求预览",
  );
  assert.equal(mock.state.calls.length, before);
  const native = JSON.parse(
    await page.locator(".llm-request-preview pre").textContent(),
  );
  assert.deepEqual(native.messages, [
    { role: "system", content: text },
    { role: "user", content: "这次任务请描述图片" },
  ]);
  prompt = await engine.api("/v1/llm/system-prompts", "POST", {
    id: prompt.id,
    expected_revision: prompt.revision,
    config: { ...prompt.config, description: "预览之后再次更新" },
  });
  await page.getByRole("button", { name: "刷新状态", exact: true }).click();
  await expect(
    page
      .getByLabel("测试 System Prompt 预设", { exact: true })
      .locator("option:checked"),
  ).toHaveText("图像描述指令 · v3");
  await page.getByRole("button", { name: "发送测试请求", exact: true }).click();
  await expect(page.locator(".llm-probe")).toContainText("REVISION_CONFLICT");
  assert.equal(
    mock.state.calls.length,
    before,
    "preview revision must remain fixed even when the selector refreshes",
  );
  await page
    .getByRole("button", { name: "预览请求（不联网）", exact: true })
    .click();
  await expect(page.locator(".llm-request-preview pre")).toBeVisible();
  checks.push(
    "changing a preset after preview blocks transmission until the user prepares a fresh request",
  );
  await page.getByRole("button", { name: "发送测试请求", exact: true }).click();
  await expect(page.locator(".llm-probe")).toContainText("调用完成");
  assert.deepEqual(mock.state.calls.at(-1).body.messages, native.messages);
  await page.locator(".llm-request-preview").scrollIntoViewIfNeeded();
  await page.screenshot({
    path: resolve(run, "system-prompt-transmission.png"),
  });
  checks.push(
    "selected preset and transient test user input reach actual upstream mock after network-free preview",
  );
  await page.getByRole("button", { name: "编辑与撤销", exact: true }).click();
  await page.getByRole("button", { name: "API 与模型", exact: true }).click();
  await page
    .locator(".llm-model-list .llm-model-row")
    .filter({ hasText: "模型一" })
    .getByRole("button", { name: "调用检查" })
    .click();
  await expect(page.getByLabel("测试消息", { exact: true })).toHaveValue(
    "Reply with OK.",
  );
  assert.equal(
    (await engine.api("/v1/llm/system-prompts/" + prompt.id)).config.text,
    text,
  );
  checks.push(
    "test user input is ephemeral and never added to the saved system prompt",
  );
}
