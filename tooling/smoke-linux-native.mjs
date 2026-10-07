// WebKitGTK acceptance uses the real Tauri host and IPC. Fixture pages report
// results over loopback; no Chromium/CDP substitute, real account, or real lake.
import assert from "node:assert/strict";
import { spawn, execFile } from "node:child_process";
import { promisify } from "node:util";
import { createServer as httpServer } from "node:http";
import { mkdir, open, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { sleep } from "./engine-fixture.mjs";

assert.equal(
  process.platform,
  "linux",
  "This probe uses WebKitGTK and an X11 test display",
);
assert.equal(
  process.env.STUDIO_LINUX_TEST_SESSION,
  "1",
  "Use tooling/linux-test-session.sh",
);
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(root, ".local/test-runs", "linux-native-" + Date.now());
await mkdir(run, { recursive: true });
const chosenFolder = resolve(run, "selected-folder");
await mkdir(chosenFolder);
const execute = promisify(execFile);
const require = createRequire(resolve(root, "apps/desktop/package.json"));
const { createServer } = await import(pathToFileURL(require.resolve("vite")));
const source = (path) => "/@fs/" + resolve(root, path);
const connection = {
  api_version: 1,
  endpoint: "",
  token: crypto.randomUUID(),
  instance_id: crypto.randomUUID(),
  pid: 0,
};
let report,
  dialogOpened = false,
  child,
  server;
const recorded = [];
const loadedPages = new Set();
const blockedPages = new Set();
const api = httpServer(async (req, res) => {
  const send = (value, status = 200) => {
    res.writeHead(status, { "content-type": "application/json" });
    res.end(JSON.stringify(value));
  };
  if (req.headers.authorization !== "Bearer " + connection.token)
    return send({}, 401);
  if (req.url === "/v1/source-collections/status")
    return send({
      configured: true,
      runtime: { state_root: resolve(run, "control") },
    });
  const parts = [];
  for await (const part of req) parts.push(part);
  const body = parts.length ? JSON.parse(Buffer.concat(parts)) : {};
  recorded.push({ path: req.url, body });
  if (req.url?.endsWith("/authenticate"))
    return send({
      account: {
        id: body.account_id,
        site: "pixiv",
        revision: 1,
        state: "valid",
        mode: "session",
        label: body.label,
        bound_user_id: "4242",
        credential_set: true,
        last_probe_at: null,
      },
      visibility: {
        context_id: "fixture",
        observed_at: new Date().toISOString(),
        login: "session",
        r18: "unknown",
        r18g: "unknown",
        ai_display: "unknown",
        coverage_verified: false,
      },
    });
  return send({}, 404);
});
await new Promise((done) => api.listen(0, "127.0.0.1", done));
connection.endpoint = `http://127.0.0.1:${api.address().port}`;
await writeFile(resolve(run, "connection.json"), JSON.stringify(connection));
const entry = resolve(run, "probe.js");
await writeFile(
  entry,
  `
import { invoke } from '@tauri-apps/api/core';
import '@studio/ui/styles.css';
const checks=[];
const check=(condition, name)=>{if(!condition)throw new Error(name);checks.push(name);document.querySelector('#results').textContent=checks.join('\\n');};
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const start=()=>invoke('pixiv_login_start',{input:{account_id:crypto.randomUUID(),request_key:crypto.randomUUID(),label:'Linux 离线验证',expected_revision:null}});
try {
  check(await invoke('probe_native_desktop'), '原生窗口最大化、恢复与中文剪贴板');
  const dialog=invoke('probe_folder_dialog');
  await fetch('/dialog-opened',{method:'POST'});
  const selected=await dialog;
  check(selected===${JSON.stringify(chosenFolder)} || selected===${JSON.stringify(chosenFolder + "/")}, 'GTK 原生目录选择器: '+selected);
  await invoke('probe_seed',{id:null});
  check((await invoke('pixiv_login_status')).available, 'Linux 登录助手可用');
  const first=await start();
  check(first.window_open && first.phase==='waiting','独立隐私登录窗口');
  for(let i=0;i<50;i++){
    const loaded=await fetch('/loaded-pages').then(r=>r.json());
    if(loaded.includes('/login'))break;
    await sleep(100);
  }
  await invoke('probe_seed',{id:first.id});
  await invoke('probe_open_child',{id:first.id});
  let shared=false;
  for(let i=0;i<50;i++){shared=await invoke('probe_child_shared',{id:first.id});if(shared)break;await sleep(100);}
  check(shared,'关联弹窗共享登录会话');
  check((await fetch('/popup-ready',{method:'POST'})).ok,'登录子窗口页面完成加载');
  checks.push('登录页及子窗口不能调用主窗口命令');
  await invoke('pixiv_login_finish',{id:first.id});
  const saved=(await invoke('pixiv_login_status')).session;
  check(saved.phase==='succeeded','Cookie 从原生窗口传到后端并保存');
  check(await invoke('probe_regular_intact'),'主窗口 Cookie 保持隔离');
  const second=await start();
  await invoke('pixiv_login_finish',{id:second.id}).catch(()=>{});
  const empty=(await invoke('pixiv_login_status')).session;
  check(empty.phase!=='succeeded','新登录窗口没有沿用上次 Cookie');
  await invoke('pixiv_login_cancel',{id:second.id});
  const third=await start();
  await invoke('probe_close_login',{id:third.id});
  check((await invoke('pixiv_login_status')).session.phase==='cancelled','关闭窗口取消会话');
  check(await invoke('probe_regular_intact'),'多次会话后主窗口 Cookie 仍完整');
  const text=document.querySelector('h1');
  check(getComputedStyle(text).fontFamily.includes('Noto Sans'),'中文字体回退已加载');
  await fetch('/probe-report',{method:'POST',body:JSON.stringify({checks})});
}catch(error){await fetch('/probe-report',{method:'POST',body:JSON.stringify({checks,error:String(error?.message||error)+'\\n'+String(error?.stack||'')})});}
`,
);
try {
  server = await createServer({
    configFile: false,
    root: resolve(root, "apps/desktop"),
    cacheDir: resolve(run, "vite-cache"),
    resolve: { dedupe: ["@studio/ui", "@tauri-apps/api"] },
    plugins: [
      {
        name: "linux-native-probe",
        configureServer(vite) {
          vite.middlewares.use(async (req, res, next) => {
            if (req.url === "/probe-report") {
              const chunks = [];
              for await (const part of req) chunks.push(part);
              report = JSON.parse(Buffer.concat(chunks));
              res.end("ok");
              return;
            }
            if (req.url === "/dialog-opened") {
              dialogOpened = true;
              res.end("ok");
              return;
            }
            if (req.url === "/popup-ready") {
              for (let i = 0; i < 50 && !loadedPages.has("/login-child"); i++)
                await sleep(100);
              if (!loadedPages.has("/login-child")) {
                res.statusCode = 500;
                res.end("Child page did not load");
                return;
              }
              if (
                !blockedPages.has("/login") ||
                !blockedPages.has("/login-child")
              ) {
                res.statusCode = 500;
                res.end("Remote page reached a main-window command");
                return;
              }
              await sleep(500);
              await execute("import", [
                "-window",
                "root",
                resolve(run, "login-window.png"),
              ]);
              res.end("ok");
              return;
            }
            if (req.url === "/loaded-pages") {
              res.setHeader("content-type", "application/json");
              res.end(JSON.stringify([...loadedPages]));
              return;
            }
            if (req.url === "/page-loaded") {
              const chunks = [];
              for await (const part of req) chunks.push(part);
              const page = JSON.parse(Buffer.concat(chunks));
              loadedPages.add(page.path);
              if (page.denied) blockedPages.add(page.path);
              res.end("ok");
              return;
            }
            if (req.url === "/login" || req.url === "/login-child") {
              res.setHeader("content-type", "text/html; charset=utf-8");
              res.end(
                "<!doctype html><html><head><meta charset='utf-8'></head><body><h1>Pixiv 登录窗口 · 离线测试</h1><p>此窗口使用原生 WebKitGTK；测试只使用合成 Cookie。</p><script>Promise.resolve().then(()=>window.__TAURI_INTERNALS__.invoke('engine_connection')).then(()=>false,()=>true).then(denied=>fetch('/page-loaded',{method:'POST',body:JSON.stringify({path:location.pathname,denied})}));</script></body></html>",
              );
              return;
            }
            if (req.url !== "/") return next();
            res.setHeader("content-type", "text/html; charset=utf-8");
            res.end(
              await vite.transformIndexHtml(
                "/",
                `<!doctype html><html><head><meta charset="utf-8"></head><body style="padding:32px"><h1>Dataset Studio · Linux 原生桌面验证</h1><pre id="results"></pre><script type="module" src=${JSON.stringify(source(entry))}></script></body></html>`,
              ),
            );
          });
        },
      },
    ],
    server: {
      host: "127.0.0.1",
      port: 1458,
      strictPort: true,
      fs: { allow: [root] },
    },
  });
  await server.listen();
  const log = await open(resolve(run, "native.log"), "a");
  child = spawn(resolve(root, "target/debug/examples/pixiv_login_probe"), [], {
    cwd: root,
    stdio: ["ignore", log.fd, log.fd],
    env: {
      ...process.env,
      STUDIO_PIXIV_LOGIN_PROBE: run,
      STUDIO_NATIVE_VISIBLE: "1",
    },
  });
  await log.close();
  let dialogHandled = false;
  for (let i = 0; i < 1200 && !report; i++) {
    if (child.exitCode !== null)
      throw new Error("Native host exited; inspect native.log");
    if (dialogOpened && !dialogHandled) {
      try {
        const { stdout } = await execute("xdotool", [
          "search",
          "--onlyvisible",
          "--name",
          "Studio Linux folder probe",
        ]);
        const window = stdout.trim().split("\n").at(-1);
        await sleep(800);
        await execute("import", [
          "-window",
          "root",
          resolve(run, "folder-dialog.png"),
        ]);
        await execute("xdotool", ["windowactivate", "--sync", window]);
        await execute("xdotool", ["key", "--clearmodifiers", "ctrl+l"]);
        await execute("xdotool", [
          "type",
          "--clearmodifiers",
          "--delay",
          "0",
          chosenFolder,
        ]);
        await execute("xdotool", ["key", "Return"]);
        await sleep(400);
        await execute("xdotool", ["key", "Return"]);
        dialogHandled = true;
      } catch {
        /* The native dialog has not been mapped yet. */
      }
    }
    await sleep(100);
  }
  assert.ok(report, "Native page did not report within 120 seconds");
  await writeFile(resolve(run, "report.json"), JSON.stringify(report, null, 2));
  await execute("import", ["-window", "root", resolve(run, "desktop.png")]);
  assert.ok(!report.error, report.error);
  const auth = recorded.find((r) => r.path.endsWith("/authenticate"));
  assert.ok(auth, "Native bridge never reached the backend");
  assert.equal(auth.body.cookies.length, 1);
  assert.equal(auth.body.cookies[0].name, "PHPSESSID");
  console.log(
    JSON.stringify({
      run,
      checks: report.checks,
      filtered_cookie_bridge: true,
    }),
  );
} finally {
  child?.kill();
  if (server) await server.close();
  await new Promise((done) => api.close(done));
}
