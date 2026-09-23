// Exact SDK -> engine -> native wire checks against local mock providers only.
import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { EngineFixture, sleep } from "./engine-fixture.mjs";
import { clientFixture } from "./client-fixture.mjs";
import { llmFixture, network } from "./llm-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local/test-runs/integration-system-prompts-" + Date.now(),
);
await mkdir(run, { recursive: true });
const engine = new EngineFixture(root, resolve(run, "state"));
const mock = await llmFixture();
const { StudioClient } = await clientFixture(root, resolve(run, "sdk"));
const checks = [];
let client;
const text =
  '  系统指令 🧪\r\n保留 {{literal}}、"引号"、\\路径\n\t尾部空白  \n';
const user = "本次任务 A：图片描述（不保存）";
const message = (text, role = "user") => ({
  role,
  content: [{ type: "text", text }],
});
const config = { name: "多语言指令", description: "跨模型复用", text };
const invalid = (e) => e.code === "INVALID_INPUT";
const conflict = (e) => e.code === "REVISION_CONFLICT";
function wire(body, protocol, userText = user) {
  if (protocol === "gemini") {
    assert.deepEqual(body.systemInstruction, { parts: [{ text }] });
    assert.deepEqual(body.contents, [
      { role: "user", parts: [{ text: userText }] },
    ]);
  } else if (protocol === "openai_responses") {
    assert.deepEqual(body.input, [
      { role: "system", content: [{ type: "input_text", text }] },
      { role: "user", content: [{ type: "input_text", text: userText }] },
    ]);
  } else {
    assert.deepEqual(body.messages, [
      { role: "system", content: text },
      { role: "user", content: userText },
    ]);
  }
  assert.equal("system_prompt_id" in body, false);
  assert.equal("expected_system_prompt_revision" in body, false);
}
try {
  await engine.start();
  client = new StudioClient(engine.connection);
  const prompts = client.llm.systemPrompts;
  let prompt = await prompts.save({ expected_revision: 0, config });
  assert.deepEqual((await prompts.get(prompt.id)).config, config);
  assert.equal((await prompts.list()).items.length, 1);
  for (const changed of [
    { text: " \n\t " },
    { text: "汉".repeat(21846) },
    { name: " " },
    { description: "x".repeat(2001) },
  ]) {
    await assert.rejects(
      () =>
        prompts.save({
          expected_revision: 0,
          config: { ...config, ...changed },
        }),
      invalid,
    );
  }
  await assert.rejects(
    () =>
      prompts.save({
        expected_revision: 0,
        config,
        user_prompt: "must not persist",
      }),
    invalid,
  );
  prompt = await prompts.save({
    id: prompt.id,
    expected_revision: 1,
    config: { ...config, name: "已重命名" },
  });
  assert.equal(prompt.revision, 2);
  await assert.rejects(
    () => prompts.save({ id: prompt.id, expected_revision: 1, config }),
    conflict,
  );
  await assert.rejects(() => prompts.remove(prompt.id, 1), conflict);
  checks.push(
    "literal text persistence, rename, UTF-8 limits, no User Prompt field, revision conflicts",
  );
  const models = [];
  for (const [kind, protocol, path] of [
    ["openai", "openai_chat", "openai"],
    ["openai", "openai_responses", "openai-responses"],
    ["openai_compatible", "openai_chat", "compatible-chat"],
    ["openai_compatible", "openai_responses", "compatible"],
    ["openrouter", "openai_chat", "router"],
    ["gemini", "gemini", "gemini"],
  ]) {
    const provider = await client.llm.providers.save({
      expected_revision: 0,
      config: {
        name: path,
        kind,
        base_url: mock.url + "/" + path,
        enabled: true,
        headers: {},
        network,
      },
    });
    const model = await client.llm.models.save({
      provider_id: provider.id,
      expected_revision: 0,
      config: {
        name: path,
        remote_model_id: "text-model",
        protocol,
        enabled: true,
        parameters: {},
        capability_overrides: {},
      },
    });
    const input = {
      model_id: model.id,
      system_prompt_id: prompt.id,
      expected_system_prompt_revision: prompt.revision,
      messages: [message(user)],
    };
    const count = mock.state.calls.length;
    const prepared = await client.llm.prepare(input);
    assert.equal(
      mock.state.calls.length,
      count,
      "preview must not contact upstream",
    );
    assert.equal(prepared.snapshot.schema_version, 2);
    assert.equal(prepared.snapshot.system_prompt_id, prompt.id);
    assert.equal(prepared.snapshot.system_prompt_revision, 2);
    assert.deepEqual(prepared.snapshot.messages, [
      message(text, "system"),
      message(user),
    ]);
    wire(prepared.native_request, protocol);
    const response = await client.llm.generate(input);
    wire(mock.state.calls.at(-1).body, protocol);
    assert.deepEqual(response.snapshot.messages, prepared.snapshot.messages);
    const userB = "本次任务 B：只需简短回答";
    const events = [];
    for await (const event of client.llm.stream({
      ...input,
      messages: [message(userB)],
    }))
      events.push(event);
    assert.equal(events.at(-1).type, "completed");
    assert.equal(events.at(-1).response.snapshot.system_prompt_revision, 2);
    assert.deepEqual(events.at(-1).response.snapshot.messages, [
      message(text, "system"),
      message(userB),
    ]);
    wire(mock.state.calls.at(-1).body, protocol, userB);
    models.push({ model, provider, input });
    checks.push(
      kind +
        "/" +
        protocol +
        ": preview, complete and streaming native wire preserve system role and task-specific user input",
    );
  }
  const { input, model, provider } = models[0];
  const count = mock.state.calls.length;
  for (const role of ["system", "developer"]) {
    await assert.rejects(
      () =>
        client.llm.generate({
          ...input,
          messages: [message("collision", role), message(user)],
        }),
      invalid,
    );
  }
  await assert.rejects(
    () => client.llm.prepare({ ...input, system_prompt_id: null }),
    invalid,
  );
  await assert.rejects(
    () =>
      client.llm.prepare({ ...input, system_prompt_id: crypto.randomUUID() }),
    (e) => e.code === "NOT_FOUND",
  );
  await assert.rejects(
    () => client.llm.generate({ ...input, expected_system_prompt_revision: 1 }),
    conflict,
  );
  await assert.rejects(async () => {
    for await (const event of client.llm.stream({
      ...input,
      expected_system_prompt_revision: 1,
    }))
      assert.fail(event.type);
  }, conflict);
  await assert.rejects(
    () =>
      client.llm.prepare({
        ...input,
        messages: Array.from({ length: 256 }, () => message("x")),
      }),
    invalid,
  );
  await assert.rejects(
    () =>
      client.llm.prepare({
        ...input,
        messages: [message("x".repeat(64 * 1024 * 1024 - 100))],
      }),
    invalid,
  );
  assert.equal(mock.state.calls.length, count);
  const direct = await client.llm.prepare({
    model_id: model.id,
    messages: [message("caller-owned", "system"), message(user)],
  });
  assert.equal(direct.snapshot.system_prompt_id, null);
  assert.equal(direct.native_request.messages[0].content, "caller-owned");
  const none = await client.llm.prepare({
    model_id: model.id,
    messages: [message(user)],
  });
  assert.equal(none.native_request.messages.length, 1);
  const parameterPreset = await client.llm.presets.save({
    expected_revision: 0,
    config: {
      name: "独立参数预设",
      protocol: "openai_chat",
      parameters: { temperature: 0 },
    },
  });
  const combined = await client.llm.prepare({
    ...input,
    preset_id: parameterPreset.id,
    expected_preset_revision: parameterPreset.revision,
  });
  assert.equal(combined.native_request.temperature, 0);
  wire(combined.native_request, "openai_chat");
  checks.push(
    "conflicting roles, stale/deleted references and resolved input limits reject before upstream; legacy messages and parameter presets remain independent",
  );
  const copy = await prompts.save({
    expected_revision: 0,
    config: { ...config, name: "独立副本" },
  });
  const slow = await client.llm.models.save({
    provider_id: provider.id,
    expected_revision: 0,
    config: { ...model.config, remote_model_id: "slow-model" },
  });
  const before = mock.state.calls.length;
  const pending = client.llm.generate({
    ...input,
    model_id: slow.id,
    system_prompt_id: copy.id,
    expected_system_prompt_revision: copy.revision,
  });
  for (let i = 0; i < 100 && mock.state.calls.length === before; i++)
    await sleep(10);
  assert.ok(mock.state.calls.length > before);
  const changed = await prompts.save({
    id: copy.id,
    expected_revision: copy.revision,
    config: { ...copy.config, text: "changed after dispatch" },
  });
  await prompts.remove(changed.id, changed.revision);
  const completed = await pending;
  assert.equal(completed.snapshot.system_prompt_revision, 1);
  assert.equal(completed.snapshot.messages[0].content[0].text, text);
  await assert.rejects(
    () =>
      client.llm.generate({
        ...input,
        system_prompt_id: copy.id,
        expected_system_prompt_revision: null,
      }),
    (e) => e.code === "NOT_FOUND",
  );
  assert.equal((await prompts.get(prompt.id)).config.text, text);
  checks.push(
    "copy independence and immutable in-flight snapshot after prompt update and deletion",
  );
  client.dispose();
  await engine.stop();
  await engine.start();
  client = new StudioClient(engine.connection);
  assert.deepEqual(await client.llm.systemPrompts.get(prompt.id), prompt);
  assert.equal((await client.llm.systemPrompts.list()).items.length, 1);
  await client.llm.generate(input);
  wire(mock.state.calls.at(-1).body, "openai_chat");
  client.dispose();
  await engine.stop();
  const db = new DatabaseSync(resolve(run, "state/registry.sqlite"), {
    readOnly: true,
  });
  assert.equal(db.prepare("PRAGMA user_version").get().user_version, 4);
  const rows = db.prepare("SELECT json FROM llm_system_prompts").all();
  assert.equal(rows.length, 1);
  assert.equal(JSON.parse(rows[0].json).config.text, text);
  assert.equal(
    rows.some((r) => r.json.includes(user)),
    false,
  );
  db.close();
  assert.equal(
    (await readFile(resolve(run, "state/engine.log"), "utf8")).includes(
      "{{literal}}",
    ),
    false,
  );
  checks.push(
    "restart persistence, registry v4, no stored user input or prompt body in engine logs",
  );
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify({ passed: true, checks }, null, 2),
  );
  console.log(
    JSON.stringify({
      passed: true,
      checks: checks.length,
      report: resolve(run, "report.json"),
    }),
  );
} catch (error) {
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify({ passed: false, checks, error: String(error) }, null, 2),
  );
  throw error;
} finally {
  client?.dispose();
  await engine.stop();
  await mock.close();
}
