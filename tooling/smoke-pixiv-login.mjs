// Real native bridge + WebView2 + settings component. The loopback server and
// all cookies are synthetic; no Pixiv request or user profile is involved.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createServer as httpServer } from "node:http";
import { mkdir, open, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { chromium, expect } from "@playwright/test";
import { sleep } from "./engine-fixture.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(
  root,
  ".local/test-runs",
  "pixiv-login-native-" + Date.now(),
);
const require = createRequire(resolve(root, "apps/desktop/package.json"));
const { createServer } = await import(pathToFileURL(require.resolve("vite")));
const { default: react } = await import(
  pathToFileURL(require.resolve("@vitejs/plugin-react"))
);
const source = (file) => "/@fs/" + resolve(root, file).replaceAll("\\", "/");
const origin = "http://127.0.0.1:1458";
const livePage = process.argv.includes("--live-login-page");
const checks = [],
  errors = [],
  requests = [];
const accounts = new Map(),
  ledger = new Map();
let mode = "success",
  server,
  api,
  child,
  browser,
  fallbackBrowser,
  page;
await mkdir(run, { recursive: true });
const connection = {
  api_version: 1,
  endpoint: "",
  token: crypto.randomUUID(),
  instance_id: crypto.randomUUID(),
  pid: 0,
};
const status = {
  configured: true,
  protocol_version: 1,
  collection_contract_version: 2,
  runtime: {
    state: "ready",
    python: "fixture",
    state_root: resolve(run, "control"),
    failures: 0,
    error_code: null,
    message: null,
    next_retry_ms: null,
  },
  counts: {},
  active: [],
  fixture_padding: "x".repeat(96 * 1024),
};
const send = (res, value, code = 200) => {
  res.writeHead(code, { "content-type": "application/json" });
  res.end(JSON.stringify(value));
};
async function invoke(command, args) {
  return page.evaluate(
    ({ command, args }) => globalThis.loginSmoke.invoke(command, args),
    { command, args },
  );
}
async function session() {
  return (await invoke("pixiv_login_status")).session;
}
async function start() {
  await expect(
    page.getByRole("button", { name: "通过浏览器登录", exact: true }),
  ).toBeEnabled();
  await page
    .getByRole("button", { name: "通过浏览器登录", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "已登录，验证并保存", exact: true }),
  ).toBeVisible();
  const value = await session();
  assert.equal(value.phase, "waiting");
  return value;
}
async function cancel() {
  await page.getByRole("button", { name: "取消此次登录", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "通过浏览器登录", exact: true }),
  ).toBeEnabled();
  assert.equal((await session()).phase, "cancelled");
}
function pages() {
  return browser.contexts().flatMap((c) => c.pages());
}
async function loginPage(path = "/login") {
  await expect
    .poll(() => pages().some((p) => p.url() === origin + path))
    .toBe(true);
  return pages().find((p) => p.url() === origin + path);
}
try {
  api = httpServer((req, res) => {
    void (async () => {
      res.setHeader("access-control-allow-origin", origin);
      res.setHeader(
        "access-control-allow-headers",
        "authorization,content-type,x-studio-session",
      );
      res.setHeader("access-control-allow-methods", "GET,POST,PUT,OPTIONS");
      if (req.method === "OPTIONS") {
        res.writeHead(204);
        res.end();
        return;
      }
      if (req.headers.authorization !== "Bearer " + connection.token)
        return send(
          res,
          { code: "UNAUTHORIZED", message: "Fixture auth" },
          401,
        );
      const path = new URL(req.url, connection.endpoint).pathname;
      if (path === "/v1/health")
        return send(res, {
          api_version: 1,
          instance_id: connection.instance_id,
        });
      if (path === "/v1/source-collections/status") return send(res, status);
      if (path === "/v1/source-collections/accounts")
        return send(res, { items: [...accounts.values()], next_cursor: null });
      if (
        /\/accounts\/[^/]+\/authenticate$/.test(path) &&
        req.method === "POST"
      ) {
        let body = "";
        for await (const chunk of req) {
          body += chunk;
          if (body.length > 65536)
            throw new Error("Oversized native candidate");
        }
        const input = JSON.parse(body);
        requests.push({
          key: input.request_key,
          account_id: input.account_id,
          cookie_count: input.cookies.length,
        });
        assert.equal(
          input.cookies.length,
          1,
          "Foreign and regular-profile cookies never leave the owned login profile",
        );
        assert.deepEqual(input.cookies[0], {
          name: "PHPSESSID",
          value: "4242_NATIVE_LOGIN_FIXTURE",
          domain: "www.pixiv.net",
          path: "/",
          secure: true,
          http_only: true,
          expires_unix: null,
        });
        if (ledger.has(input.request_key)) {
          const saved = ledger.get(input.request_key);
          assert.equal(saved.body, body);
          return send(res, saved.result);
        }
        if (mode === "reject" || mode === "different-account")
          return send(
            res,
            {
              code:
                mode === "reject"
                  ? "COLLECTION_CREDENTIAL_REQUIRED"
                  : "COLLECTION_SCOPE_CHANGED",
              message: "Synthetic rejection",
            },
            401,
          );
        const previous = accounts.get(input.account_id);
        if ((previous?.revision ?? null) !== input.expected_revision)
          return send(
            res,
            { code: "REVISION_CONFLICT", message: "Fixture revision changed" },
            409,
          );
        const account = {
          id: input.account_id,
          site: "pixiv",
          label: input.label,
          mode: "session",
          state: "valid",
          revision: (previous?.revision ?? 0) + 1,
          credential_set: true,
          bound_user_id: "4242",
          last_probe_at: new Date().toISOString(),
        };
        const result = {
          account,
          visibility: {
            context_id: crypto.randomUUID(),
            observed_at: account.last_probe_at,
            login: "authenticated",
            r18: "unknown",
            r18g: "unknown",
            ai_display: "unknown",
            coverage_verified: false,
          },
        };
        accounts.set(account.id, account);
        ledger.set(input.request_key, { body, result });
        if (mode === "drop-once") {
          mode = "success";
          res.writeHead(200, {
            "content-type": "application/json",
            "content-length": "5000",
          });
          res.write('{"account":');
          setTimeout(() => res.destroy(), 30);
          return;
        }
        return send(res, result);
      }
      return send(
        res,
        { code: "NOT_FOUND", message: "Unknown acceptance route" },
        404,
      );
    })().catch((error) => {
      errors.push(error.message);
      if (!res.headersSent)
        send(
          res,
          {
            code: "FIXTURE_FAILURE",
            message: "Native acceptance assertion failed",
          },
          500,
        );
      else res.destroy();
    });
  });
  await new Promise((done) => api.listen(0, "127.0.0.1", done));
  connection.endpoint = "http://127.0.0.1:" + api.address().port;
  await writeFile(resolve(run, "connection.json"), JSON.stringify(connection));
  const entry = resolve(run, "fixture.tsx");
  await writeFile(
    entry,
    `
import React from 'react';
import {createRoot} from 'react-dom/client';
import {QueryClient,QueryClientProvider} from '@tanstack/react-query';
import {StudioClient} from '@studio/client';
import {invoke,isTauri} from '@tauri-apps/api/core';
import {collectionLogin} from ${JSON.stringify(source("apps/desktop/src/platform/collectionLogin.ts"))};
import {CollectionApiSettings} from ${JSON.stringify(source("apps/desktop/src/features/lake-updates/CollectionApiSettings.tsx"))};
import '@studio/ui/styles.css';
import ${JSON.stringify(source("apps/desktop/src/app/studio.css"))};
import ${JSON.stringify(source("apps/desktop/src/app/workbench-theme.css"))};
import ${JSON.stringify(source("apps/desktop/src/features/lake-updates/lake-updates.css"))};
globalThis.loginSmoke={invoke,responses:[]};
const bridge=collectionLogin && Object.fromEntries(Object.entries(collectionLogin).map(([key,fn])=>[key,async(...args)=>{const result=await fn(...args);globalThis.loginSmoke.responses.push(result);return result;}]));
const connection=isTauri()?await invoke('engine_connection'):await fetch('/__studio/connection').then(r=>r.json());
const client=new StudioClient(connection,bridge);
const cache=new QueryClient({defaultOptions:{queries:{retry:false,refetchOnWindowFocus:false}}});
createRoot(document.getElementById('root')).render(<React.StrictMode><QueryClientProvider client={cache}><main className="lake-settings" style={{padding:24,overflow:'auto',height:'100vh'}}><h2>Pixiv 登录助手验收</h2><CollectionApiSettings client={client}/></main></QueryClientProvider></React.StrictMode>);
`,
  );
  const html = `<!doctype html><html><head><meta charset="utf-8"></head><body><div id="root"></div><script type="module" src=${JSON.stringify(source(entry))}></script></body></html>`;
  server = await createServer({
    configFile: false,
    root: resolve(root, "apps/desktop"),
    cacheDir: resolve(run, "vite-cache"),
    resolve: {
      dedupe: [
        "react",
        "react-dom",
        "@tanstack/react-query",
        "@studio/client",
        "@studio/ui",
        "@studio/contracts",
        "@tauri-apps/api",
      ],
    },
    plugins: [
      {
        name: "pixiv-login-fixture",
        configureServer(vite) {
          vite.middlewares.use(async (req, res, next) => {
            if (req.url === "/__studio/connection")
              return send(res, connection);
            if (req.url === "/login" || req.url === "/login-child") {
              res.setHeader("content-type", "text/html; charset=utf-8");
              res.end(
                "<html><body><h1>Offline login fixture</h1></body></html>",
              );
              return;
            }
            if (req.url !== "/") return next();
            res.setHeader("content-type", "text/html; charset=utf-8");
            res.end(await vite.transformIndexHtml("/", html));
          });
        },
      },
      react(),
    ],
    optimizeDeps: { entries: [entry] },
    server: {
      host: "127.0.0.1",
      port: 1458,
      strictPort: true,
      fs: { allow: [root] },
    },
  });
  await server.listen();
  const log = await open(resolve(run, "native.log"), "a");
  child = spawn(
    resolve(root, "target/debug/examples/pixiv_login_probe.exe"),
    [],
    {
      cwd: root,
      windowsHide: true,
      env: {
        ...process.env,
        STUDIO_PIXIV_LOGIN_PROBE: run,
        STUDIO_PIXIV_LOGIN_LIVE_PAGE: livePage ? "1" : "0",
        WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-port=9351",
        WEBVIEW2_USER_DATA_FOLDER: resolve(run, "host/pixiv-login-webview"),
      },
      stdio: ["ignore", log.fd, log.fd],
    },
  );
  await log.close();
  for (let i = 0; i < 100; i++) {
    try {
      browser = await chromium.connectOverCDP("http://127.0.0.1:9351", {
        timeout: 500,
      });
      break;
    } catch {
      if (child.exitCode !== null)
        throw new Error("Native login host exited; inspect native.log");
      await sleep(100);
    }
  }
  assert.ok(browser);
  await expect
    .poll(() => pages().some((p) => p.url().startsWith(origin)))
    .toBe(true);
  page = pages().find((p) => p.url().startsWith(origin));
  page.on("pageerror", (e) => errors.push(e.message));
  await expect(page.getByLabel("Pixiv PHPSESSID")).toBeVisible({
    timeout: 30000,
  });
  if (livePage) {
    await start();
    await expect
      .poll(
        () =>
          pages().some((p) =>
            p.url().startsWith("https://accounts.pixiv.net/"),
          ),
        { timeout: 45000 },
      )
      .toBe(true);
    const login = pages().find((p) =>
      p.url().startsWith("https://accounts.pixiv.net/"),
    );
    await expect(login.locator('input[type="password"]')).toBeVisible({
      timeout: 45000,
    });
    assert.ok(
      await login.locator('input[type="text"],input[type="email"]').count(),
    );
    await login.screenshot({ path: resolve(run, "official-login-page.png") });
    await cancel();
    assert.equal(requests.length, 0);
    checks.push(
      "official Pixiv login page renders the email/password form in the owned native window; no credentials or authentication request were submitted",
    );
  } else {
    await invoke("probe_seed", { id: null });
    await expect
      .poll(() => invoke("probe_regular_intact"), { timeout: 4000 })
      .toBe(true);

    let current = await start();
    const login = await loginPage();
    const denied = await login.evaluate(async () => {
      try {
        return {
          allowed: true,
          result:
            await globalThis.__TAURI_INTERNALS__.invoke("engine_connection"),
        };
      } catch {
        return { allowed: false };
      }
    });
    assert.equal(
      denied.allowed,
      false,
      "Remote login pages cannot obtain engine credentials or use host commands",
    );
    await expect(page.getByLabel("Pixiv PHPSESSID")).toBeDisabled();
    await page
      .getByRole("button", { name: "已登录，验证并保存", exact: true })
      .click();
    await expect(
      page.getByText("尚未取得 Pixiv 登录会话", { exact: false }).first(),
    ).toBeVisible();
    assert.equal(requests.length, 0);
    await cancel();
    assert.equal(await invoke("probe_regular_intact"), true);
    checks.push(
      "native login is owned and isolated; an empty session cannot be saved; remote pages cannot access host IPC; cancellation preserves the normal profile",
    );

    current = await start();
    await invoke("probe_seed", { id: current.id });
    const popupParent = await loginPage();
    await popupParent.evaluate(() => {
      const button = globalThis.document.createElement("button");
      button.id = "open-child";
      button.textContent = "Open login child";
      button.onclick = () => {
        globalThis.open("/login-child", "_blank");
      };
      globalThis.document.body.appendChild(button);
    });
    await popupParent.locator("#open-child").click();
    await loginPage("/login-child");
    await invoke("probe_close_login", { id: current.id });
    await expect.poll(async () => (await session()).phase).toBe("cancelled");
    await expect
      .poll(() => pages().filter((p) => p.url().includes("/login")).length)
      .toBe(0);
    current = await start();
    await page
      .getByRole("button", { name: "已登录，验证并保存", exact: true })
      .click();
    await expect(
      page.getByText("尚未取得 Pixiv 登录会话", { exact: false }).first(),
    ).toBeVisible();
    assert.equal(
      requests.length,
      0,
      "A new login never inherits the previous InPrivate session",
    );
    checks.push(
      "login child windows share the owned lifetime; closing the parent closes children; reopening starts with an empty private cookie store",
    );

    await invoke("probe_seed", { id: current.id });
    await page.reload();
    await expect(
      page.getByRole("button", { name: "已登录，验证并保存", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText("尚未取得 Pixiv 登录会话", { exact: false }).first(),
    ).toBeVisible();
    await expect(
      page.getByText("[object Object]", { exact: true }),
    ).toHaveCount(0);
    await page.screenshot({ path: resolve(run, "login-waiting.png") });
    await page
      .getByRole("button", { name: "已登录，验证并保存", exact: true })
      .click();
    await expect(
      page.getByText("浏览器登录已验证", { exact: false }),
    ).toBeVisible();
    const saved = (await session()).result.account;
    assert.equal(saved.bound_user_id, "4242");
    assert.equal(requests.length, 1);
    await expect(page.getByLabel("Pixiv PHPSESSID")).toHaveValue("");
    assert.equal(await invoke("probe_regular_intact"), true);
    await page.screenshot({ path: resolve(run, "login-saved.png") });
    checks.push(
      "settings reload recovers the native session; HttpOnly Pixiv cookies reach the backend directly; only the verified account returns to the renderer",
    );

    const before = JSON.stringify(accounts.get(saved.id));
    for (const failure of ["reject", "different-account"]) {
      current = await start();
      await invoke("probe_seed", { id: current.id });
      mode = failure;
      await page
        .getByRole("button", { name: "已登录，验证并保存", exact: true })
        .click();
      await expect(
        page
          .getByText(
            failure === "reject"
              ? "网站尚未确认登录"
              : "浏览器登录的是另一个账号",
            { exact: false },
          )
          .first(),
      ).toBeVisible();
      assert.equal(JSON.stringify(accounts.get(saved.id)), before);
      await cancel();
    }
    checks.push(
      "failed authentication and account mismatch preserve the prior valid credential and leave a recoverable login window",
    );

    current = await start();
    await invoke("probe_seed", { id: current.id });
    mode = "drop-once";
    await page
      .getByRole("button", { name: "已登录，验证并保存", exact: true })
      .click();
    await expect(
      page.getByRole("button", { name: "重新确认保存结果", exact: true }),
    ).toBeVisible();
    const attempt = requests.at(-1).key;
    await invoke("probe_close_login", { id: current.id });
    await page.reload();
    await expect(
      page.getByRole("button", { name: "重新确认保存结果", exact: true }),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "重新确认保存结果", exact: true })
      .click();
    await expect(
      page.getByText("浏览器登录已验证", { exact: false }),
    ).toBeVisible();
    assert.equal(requests.at(-1).key, attempt);
    assert.equal(accounts.get(saved.id).revision, 2);
    const returned = await page.evaluate(() =>
      JSON.stringify(globalThis.loginSmoke.responses),
    );
    assert.ok(
      !returned.includes("NATIVE_LOGIN_FIXTURE") &&
        !returned.includes("MUST_STAY_IN_BROWSER") &&
        !returned.includes('"cookies"'),
    );
    checks.push(
      "a committed response lost in transit is replayed with the same request after window close and UI reload, without a second account revision or secret readback",
    );

    fallbackBrowser = await chromium.launch({
      channel: "msedge",
      headless: true,
    });
    const web = await fallbackBrowser.newPage();
    await web.goto(origin);
    await expect(
      web.getByText("浏览器登录助手在 Windows 桌面版中提供", { exact: false }),
    ).toBeVisible();
    await expect(web.getByLabel("Pixiv PHPSESSID")).toBeEnabled();
    checks.push("web mode keeps an explicit manual-import fallback");
  }
  assert.deepEqual(errors, []);
  await writeFile(
    resolve(run, "summary.json"),
    JSON.stringify(
      { ok: true, checks, requests: requests.length, accounts: accounts.size },
      null,
      2,
    ),
  );
  console.log(JSON.stringify({ ok: true, checks, run }));
} catch (error) {
  await page?.screenshot({ path: resolve(run, "failure.png") }).catch(() => {});
  await writeFile(
    resolve(run, "failure.json"),
    JSON.stringify(
      {
        message: error.message,
        checks,
        errors,
        pages: browser ? pages().map((p) => p.url()) : [],
      },
      null,
      2,
    ),
  );
  throw error;
} finally {
  await fallbackBrowser?.close();
  if (page)
    await Promise.race([invoke("probe_shutdown").catch(() => {}), sleep(1500)]);
  if (child && child.exitCode === null) {
    const exited = new Promise((done) => child.once("exit", done));
    child.kill();
    await exited;
  }
  await browser?.close().catch(() => {});
  await server?.close();
  if (api) {
    api.closeAllConnections();
    await new Promise((done) => api.close(done));
  }
}
