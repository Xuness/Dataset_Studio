import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { copyFile, mkdir, readFile, writeFile, unlink } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

if (process.platform !== "win32") {
  console.log("Windows launcher smoke test skipped on this platform.");
  process.exit(0);
}
const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(root, ".local", "launcher-test-" + Date.now());
const fixture = resolve(run, "工作 副本");
const native = resolve(fixture, "target/debug/studio-desktop.exe");
const execute = promisify(execFile);
const sleep = (ms) => new Promise((done) => setTimeout(done, ms));
const alive = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
};
for (const path of ["tooling", "bin", "target/debug", ".local"]) {
  await mkdir(resolve(fixture, path), { recursive: true });
}
for (const path of [
  "启动开发版.bat",
  "tooling/start-dev.ps1",
  "tooling/dev.mjs",
  "tooling/cargo.mjs",
]) {
  await copyFile(resolve(root, path), resolve(fixture, path));
}
await writeFile(resolve(fixture, "package.json"), '{"type":"module"}\n');
await writeFile(
  resolve(fixture, "bin/pnpm.cmd"),
  "@echo fixture dependency check\r\n@exit /b 0\r\n",
);
await writeFile(resolve(fixture, "bin/cargo.cmd"), "@exit /b 0\r\n");
await writeFile(
  resolve(fixture, "tooling/setup-duckdb.ps1"),
  "Write-Output 'fixture runtime ready'\n",
);
await writeFile(
  resolve(fixture, ".local/development-session.json"),
  JSON.stringify({ workspace: fixture, pid: process.pid }),
);
await writeFile(resolve(run, "empty-input.txt"), "");
await copyFile(process.execPath, native);
const workerModule = resolve(fixture, "fake-desktop.mjs");
await writeFile(
  workerModule,
  `
import {readFileSync,writeFileSync,appendFileSync} from 'node:fs';
import {basename,dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
if(basename(process.execPath)==='studio-desktop.exe') {
  const root=dirname(fileURLToPath(import.meta.url));
  const marker=resolve(root,'worker.json');
  appendFileSync(resolve(root,'launches.log'),String(process.pid)+'\\n');
  try { const old=JSON.parse(readFileSync(marker,'utf8'));process.kill(old.pid,0);process.exit(0); } catch {}
  writeFileSync(marker,JSON.stringify({pid:process.pid,cwd:process.cwd(),data:process.env.STUDIO_DATA_DIR,development:process.env.STUDIO_DEVELOPMENT}));
  setInterval(()=>{},1000);
}
`,
);
const helper = resolve(run, "invoke-wrapper.ps1");
await writeFile(
  helper,
  String.raw`
param([string]$Fixture,[string]$Outside,[string]$Label,[switch]$Web)
$ErrorActionPreference='Stop'
$command='call "'+(Join-Path $Fixture '启动开发版.bat')+'"'
if($Web){$command+=' --web'}
$process=Start-Process -FilePath $env:ComSpec -ArgumentList @('/d','/c',$command) -WorkingDirectory $Outside -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $Outside ($Label+'.stdout.log')) -RedirectStandardError (Join-Path $Outside ($Label+'.stderr.log')) -RedirectStandardInput (Join-Path $Outside 'empty-input.txt')
if(-not $process.WaitForExit(15000)){$process.Kill();throw 'Wrapper timed out.'}
Write-Output $process.ExitCode
`,
);
const env = {
  ...process.env,
  PATH: resolve(fixture, "bin") + ";" + process.env.PATH,
  NODE_OPTIONS: [
    process.env.NODE_OPTIONS,
    "--import=" + pathToFileURL(workerModule).href,
  ]
    .filter(Boolean)
    .join(" "),
};
delete env.STUDIO_DATA_DIR;
const checks = [];
let worker;
async function wrapper(label, web = false) {
  const result = await execute(
    "pwsh",
    [
      "-NoLogo",
      "-NoProfile",
      "-File",
      helper,
      "-Fixture",
      fixture,
      "-Outside",
      run,
      "-Label",
      label,
      ...(web ? ["-Web"] : []),
    ],
    { cwd: run, env, windowsHide: true, timeout: 20000 },
  );
  return Number(result.stdout.trim());
}
async function stopWorker() {
  if (worker && alive(worker.pid)) process.kill(worker.pid);
  for (let i = 0; worker && alive(worker.pid) && i < 100; i++) await sleep(50);
  if (worker) assert.equal(alive(worker.pid), false, "Fixture desktop stopped");
  worker = undefined;
}
async function removeNative() {
  // Windows can release a terminated process's image mapping after its PID
  // has disappeared. Keep this fixture cleanup separate from launch readiness.
  for (let attempt = 0; ; attempt++) {
    try {
      await unlink(native);
      return;
    } catch (error) {
      if (error.code === "ENOENT") return;
      if (!["EPERM", "EBUSY"].includes(error.code) || attempt >= 100)
        throw error;
      await sleep(50);
    }
  }
}
try {
  assert.equal(await wrapper("first"), 0);
  for (let i = 0; i < 100; i++) {
    try {
      worker = JSON.parse(
        await readFile(resolve(fixture, "worker.json"), "utf8"),
      );
      break;
    } catch {
      await sleep(50);
    }
  }
  assert.ok(worker, "Native process started from wrapper");
  await sleep(1000);
  assert.ok(
    alive(worker.pid),
    "Desktop outlives the short-lived launcher console",
  );
  assert.equal(worker.cwd, fixture);
  assert.equal(worker.data, resolve(fixture, ".local/dev"));
  assert.equal(worker.development, "1");
  checks.push(
    "Real cmd wrapper works from an external directory with spaces/Chinese and preserves development paths",
  );
  assert.equal(await wrapper("repeat"), 0);
  await sleep(500);
  assert.equal(
    JSON.parse(await readFile(resolve(fixture, "worker.json"), "utf8")).pid,
    worker.pid,
  );
  const launches = (await readFile(resolve(fixture, "launches.log"), "utf8"))
    .trim()
    .split("\n").length;
  assert.equal(launches, 2);
  checks.push(
    "Repeated invocation reaches the existing desktop instance without terminating it",
  );
  assert.equal(await wrapper("web", true), 0);
  assert.equal(
    (await readFile(resolve(fixture, "launches.log"), "utf8"))
      .trim()
      .split("\n").length,
    launches,
  );
  assert.match(
    await readFile(resolve(run, "web.stdout.log"), "utf8"),
    /浏览器入口/,
  );
  checks.push(
    "--web passes through both wrappers and does not open a desktop process",
  );
  await stopWorker();
  await removeNative();
  assert.notEqual(await wrapper("missing"), 0);
  assert.match(
    await readFile(resolve(fixture, ".local/logs/startup-dev.log"), "utf8"),
    /未找到桌面程序/,
  );
  checks.push(
    "Missing desktop binary produces an actionable persisted error and a nonzero batch exit",
  );
  const report = resolve(run, "report.json");
  await writeFile(
    report,
    JSON.stringify({ passed: true, checks, fixture }, null, 2),
  );
  console.log(JSON.stringify({ passed: true, checks, report }, null, 2));
} finally {
  await stopWorker();
  await removeNative();
}
