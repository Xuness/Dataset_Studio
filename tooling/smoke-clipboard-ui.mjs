// Optional Windows test: writes fixture text to the real clipboard and verifies
// the same history API used by Win+V. No engine, project or user WebView profile.
import assert from "node:assert/strict";
import { spawn, execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, writeFile, open } from "node:fs/promises";
import { resolve } from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath, pathToFileURL } from "node:url";
import { chromium, expect } from "@playwright/test";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(root, ".local/test-runs/clipboard-" + Date.now());
const execute = promisify(execFile);
const require = createRequire(resolve(root, "apps/desktop/package.json"));
const { createServer } = await import(pathToFileURL(require.resolve("vite")));
const { default: react } = await import(
  pathToFileURL(require.resolve("@vitejs/plugin-react"))
);
const sourceUrl = (path) => "/@fs/" + resolve(root, path).replaceAll("\\", "/");
const marker = "StudioClipboard-" + Date.now();
const checks = [];
const errors = [];
const fixtureTexts = new Set();
let server;
let child;
let browser;
let page;
const sleep = (ms) => new Promise((done) => setTimeout(done, ms));
assert.equal(
  process.platform,
  "win32",
  "This check uses Windows clipboard history",
);
await mkdir(run, { recursive: true });
const entry = resolve(run, "fixture.tsx");
const fixture = `
import React from 'react';
import {createRoot} from 'react-dom/client';
import {ClipboardProvider,CopyButton} from ${JSON.stringify(sourceUrl("packages/ui/src/index.ts"))};
import {nativeClipboard,writeClipboard} from ${JSON.stringify(sourceUrl("apps/desktop/src/platform/clipboard.ts"))};
import ${JSON.stringify(sourceUrl("packages/ui/src/styles.css"))};
window.clipboardSmoke={calls:[],failure:false};
const write=async(content)=>{
  window.clipboardSmoke.calls.push(content);
  if(window.clipboardSmoke.failure) throw new Error('fixture clipboard failure');
  await writeClipboard(content);
};
createRoot(document.getElementById('root')).render(
  <React.StrictMode><ClipboardProvider write={write} captureSelection={nativeClipboard}>
    <h1>Dataset Studio 剪贴板验收</h1>
    <p>独立窗口，不连接项目。</p>
    <input id="plain" aria-label="普通文本" defaultValue=""/>
    <textarea id="multiline" aria-label="多行文本" defaultValue=""/>
    <div id="rich"><strong>中文粗体</strong> and <em>italic</em><br/>second line &amp; 😀</div>
    <div id="editable" contentEditable suppressContentEditableWarning>可编辑的中文与 English</div>
    <input id="password" aria-label="密码测试" type="password" defaultValue="test-only-password"/>
    <input id="empty" aria-label="空输入框" defaultValue=""/>
    <input id="number" aria-label="数字输入框" type="number" defaultValue="123456789"/>
    <input id="email" aria-label="邮箱输入框" type="email" defaultValue="test@example.com"/>
    <input id="custom" aria-label="自定义复制" defaultValue="component selection" onCopy={event=>{event.preventDefault();event.clipboardData.setData('text/plain','component-owned-copy')}}/>
    <CopyButton text={${JSON.stringify(marker + "-button-中文")}} label="复制测试按钮"/>
  </ClipboardProvider></React.StrictMode>
);
`;
await writeFile(entry, fixture);
const html = `<!doctype html><html lang="zh-CN"><meta charset="utf-8"><title>Clipboard verification</title><style>body{font:16px system-ui;margin:24px}input,textarea{display:block;margin:10px 0;width:95%;padding:8px}#rich,#editable{margin:12px;padding:10px;border:1px solid #888}</style><div id="root"></div><script type="module" src=${JSON.stringify(sourceUrl(entry))}></script></html>`;

async function capture(name, text, { history = true, html = false } = {}) {
  fixtureTexts.add(text);
  const expectedPath = resolve(run, "expected.txt");
  await writeFile(expectedPath, text);
  let result;
  for (let attempt = 0; attempt < 12; attempt++) {
    const { stdout } = await execute(
      "C:/Windows/System32/WindowsPowerShell/v1.0/powershell.exe",
      [
        "-NoProfile",
        "-NonInteractive",
        "-File",
        resolve(root, "tooling/clipboard-history.ps1"),
        "-ExpectedPath",
        expectedPath,
        "-CaseName",
        name,
      ],
      { windowsHide: true },
    );
    result = JSON.parse(stdout.trim());
    assert.equal(result.status, "Success");
    assert.equal(
      result.isHistoryEnabled,
      true,
      "Enable clipboard history to run this optional test",
    );
    if (result.currentMatches && (!history || result.historyMatches.length))
      break;
    await sleep(250);
  }
  assert.ok(result.currentMatches, name + ": current clipboard must match");
  if (history)
    assert.ok(
      result.historyMatches.length,
      name + ": must enter Win+V history",
    );
  if (html)
    assert.ok(
      result.historyMatches.some((item) =>
        item.formats.includes("HTML Format"),
      ),
      name + ": preserve HTML",
    );
  checks.push(result);
}
const callCount = () =>
  page.evaluate(() => globalThis.clipboardSmoke.calls.length);
async function selectNode(selector) {
  await page.locator(selector).evaluate((element) => {
    const document = element.ownerDocument;
    document.activeElement?.blur?.();
    const range = document.createRange();
    range.selectNodeContents(element);
    const selection = document.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
  });
  return page.evaluate(() => globalThis.getSelection().toString());
}
async function copyInput(selector, text, name) {
  await page.locator(selector).fill(text);
  await page.locator(selector).press("Control+a");
  const before = await callCount();
  await page.locator(selector).press("Control+c");
  await capture(name, text);
  assert.equal(
    await callCount(),
    before + 1,
    "One native write, including under React StrictMode",
  );
}
try {
  server = await createServer({
    configFile: false,
    root: resolve(root, "apps/desktop"),
    cacheDir: resolve(run, "vite-cache"),
    resolve: { dedupe: ["react", "react-dom"] },
    plugins: [
      {
        name: "clipboard-fixture",
        configureServer(vite) {
          vite.middlewares.use(async (req, res, next) => {
            if (req.url !== "/") return next();
            res.setHeader("Content-Type", "text/html; charset=utf-8");
            res.end(await vite.transformIndexHtml("/", html));
          });
        },
      },
      react(),
    ],
    optimizeDeps: { entries: [entry] },
    server: {
      host: "127.0.0.1",
      port: 1435,
      strictPort: true,
      fs: { allow: [root] },
    },
  });
  await server.listen();
  const log = await open(resolve(run, "native.log"), "a");
  child = spawn(
    resolve(root, "target/debug/examples/clipboard_probe.exe"),
    [],
    {
      cwd: root,
      env: {
        ...process.env,
        WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-port=9349",
        WEBVIEW2_USER_DATA_FOLDER: resolve(run, "webview"),
      },
      stdio: ["ignore", log.fd, log.fd],
      windowsHide: true,
    },
  );
  await log.close();
  child.on("error", (error) => errors.push(error.message));
  for (let attempt = 0; attempt < 80; attempt++) {
    try {
      browser = await chromium.connectOverCDP("http://127.0.0.1:9349", {
        timeout: 500,
      });
      break;
    } catch {
      if (child.exitCode !== null)
        throw new Error(
          `Native clipboard host exited (${child.exitCode}); see native.log`,
        );
      await sleep(150);
    }
  }
  assert.ok(browser, "Native WebView2 debugger must start");
  page = browser.contexts()[0].pages()[0];
  page.on("pageerror", (error) => errors.push(error.message));
  await expect(page.locator("#plain")).toBeVisible({ timeout: 30000 });
  await copyInput("#plain", marker + "-plain-中文", "input-ctrl-c");
  await copyInput(
    "#multiline",
    marker + "-多行\n第二行 😀 & <text>",
    "textarea-multiline-unicode",
  );
  const rich = await selectNode("#rich");
  await page.keyboard.press("Control+c");
  await capture("rich-selection", rich, { html: true });
  const writtenHtml = await page.evaluate(
    () => globalThis.clipboardSmoke.calls.at(-1).html,
  );
  assert.match(writtenHtml, /<strong>中文粗体<\/strong>/);
  assert.match(writtenHtml, /<em>italic<\/em>/);
  await page.locator("#editable").fill(marker + "-editable-中文");
  const editable = await selectNode("#editable");
  await page.keyboard.press("Control+c");
  await capture("contenteditable-selection", editable, { html: true });
  await page.getByRole("button", { name: "复制测试按钮", exact: true }).click();
  await capture("shared-copy-button", marker + "-button-中文");

  await copyInput("#number", String(Date.now()), "number-input-ctrl-c");
  await copyInput(
    "#email",
    "clip." + Date.now() + "@example.com",
    "email-input-ctrl-c",
  );

  let before = await callCount();
  await page.locator("#password").press("Control+a");
  await page.locator("#password").press("Control+c");
  assert.equal(
    await callCount(),
    before,
    "Password copy keeps browser protection",
  );
  await selectNode("#rich");
  await page.locator("#empty").focus();
  await page.locator("#empty").press("Control+c");
  assert.equal(
    await callCount(),
    before,
    "Empty focused input does not copy stale page selection",
  );
  for (const selector of ["#number", "#email"]) {
    await selectNode("#rich");
    await page.locator(selector).focus();
    await page.locator(selector).press("End");
    await page.locator(selector).press("Control+c");
    assert.equal(
      await callCount(),
      before,
      selector + ": no selection, no native write",
    );
  }
  await page.evaluate(() =>
    globalThis.document.dispatchEvent(
      new globalThis.ClipboardEvent("copy", {
        bubbles: true,
        cancelable: true,
      }),
    ),
  );
  assert.equal(
    await callCount(),
    before,
    "Synthetic copy does not write the clipboard",
  );
  await page.locator("#custom").press("Control+a");
  await page.locator("#custom").press("Control+c");
  assert.equal(
    await callCount(),
    before,
    "Component copy handler keeps precedence",
  );
  await capture("component-copy-handler", "component-owned-copy", {
    history: false,
  });
  checks.push({ case: "protected-empty-synthetic-custom", passed: true });

  await page.evaluate(() => {
    globalThis.clipboardSmoke.failure = true;
  });
  await page.locator("#plain").press("Control+a");
  await page.locator("#plain").press("Control+c");
  await expect(page.getByRole("alert")).toContainText("复制失败");
  await page.evaluate(() => {
    globalThis.clipboardSmoke.failure = false;
  });
  await copyInput("#plain", marker + "-retry-恢复", "failed-write-retry");
  await expect(page.getByRole("alert")).toHaveCount(0);
  checks.push({ case: "failure-visible-and-recoverable", passed: true });
  assert.deepEqual(errors, []);
  await page.screenshot({ path: resolve(run, "verified.png") });
} catch (error) {
  errors.push(String(error.stack ?? error));
  if (page)
    await page
      .screenshot({ path: resolve(run, "failure.png") })
      .catch(() => {});
  throw error;
} finally {
  // Delete only the exact synthetic values from this run, leaving user history
  // and the current clipboard untouched. No global ClearHistory/Clear calls.
  const expectedPath = resolve(run, "expected.txt");
  for (const text of fixtureTexts) {
    await writeFile(expectedPath, text);
    try {
      await execute(
        "C:/Windows/System32/WindowsPowerShell/v1.0/powershell.exe",
        [
          "-NoProfile",
          "-NonInteractive",
          "-File",
          resolve(root, "tooling/clipboard-history.ps1"),
          "-ExpectedPath",
          expectedPath,
          "-CaseName",
          "fixture-cleanup",
          "-Remove",
        ],
        { windowsHide: true },
      );
    } catch (error) {
      errors.push("Fixture clipboard cleanup: " + error.message);
    }
  }
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify({ checks, errors }, null, 2),
  );
  if (browser) await browser.close();
  if (child && child.exitCode === null) child.kill();
  if (server) await server.close();
}
assert.deepEqual(errors, []);
console.log(JSON.stringify({ status: "passed", checks: checks.length, run }));
