/* global window, document */
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";
import { createServer } from "../apps/desktop/node_modules/vite/dist/node/index.js";
import { browserOptions } from "./platform.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const run = resolve(root, ".local/test-runs", "image-viewer-" + Date.now());
await mkdir(run, { recursive: true });
const moduleUrl = (path) => "/@fs/" + resolve(root, path).replaceAll("\\", "/");
// Render the real viewer components with deterministic originals. Native file
// and clipboard capabilities are spies, so this test never touches the desktop.
const fixture = `
import React, { useMemo, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { PlatformFilesProvider } from ${JSON.stringify(moduleUrl("packages/ui/src/index.ts"))};
import { AssetViewer } from ${JSON.stringify(moduleUrl("apps/desktop/src/features/aesthetic/AssetViewer.tsx"))};
import { ImageStage, ImageTools, useImageViewer } from ${JSON.stringify(moduleUrl("apps/desktop/src/features/browser/ImageStage.tsx"))};
import ${JSON.stringify(moduleUrl("apps/desktop/src/app/studio.css"))};
import ${JSON.stringify(moduleUrl("apps/desktop/src/features/aesthetic/aesthetic.css"))};
const metrics = { reads: 0, saves: 0, copies: 0 };
Object.assign(window, { viewerTestMetrics: metrics });
const files = {
  chooseSaveFile: async () => { metrics.saves++; return null; },
  chooseDirectory: async () => null,
  copyImage: async () => { metrics.copies++; },
};
function BrowseStage({ client, asset }) {
  const viewer = useImageViewer(client, 'fixture', asset);
  return <div style={{ display: 'flex', flexDirection: 'column', flex: 1 }}>
    <div className="zoom-controls"><ImageTools viewer={viewer} /></div>
    <div className="image-canvas"><ImageStage viewer={viewer} client={client} projectId="fixture" edge={1600} /></div>
  </div>;
}
function Fixture() {
  const [size, setSize] = useState([128, 160]);
  const [mode, setMode] = useState('ranking');
  const client = useMemo(() => {
    const canvas = document.createElement('canvas');
    canvas.width = size[0]; canvas.height = size[1];
    const ctx = canvas.getContext('2d');
    ctx.fillStyle = '#267eaf'; ctx.fillRect(0, 0, size[0], size[1]);
    const blob = new Promise(resolve => canvas.toBlob(resolve, 'image/png'));
    return {
      original: async () => { metrics.reads++; return await blob; },
      acquireMedia: async () => {
        const url = URL.createObjectURL(await blob);
        return { url, release: () => URL.revokeObjectURL(url), verifiedMs: Date.now(), offline: false };
      },
    };
  }, [size]);
  const identity = size.join('x');
  const asset = { key: { source_id: 'fixture', asset_id: identity }, name: identity + '.png', extension: 'png', bytes: 1024, source_name: 'Fixture' };
  return <PlatformFilesProvider value={files}>
    <div>
      <button onClick={() => setMode('ranking')}>ranking</button>
      <button onClick={() => setMode('browse')}>browse</button>
      <button onClick={() => setSize([128, 160])}>normal</button>
      <button onClick={() => setSize([32, 40])}>tiny</button>
      <button onClick={() => setSize([1, 1])}>pixel</button>
      <button onClick={() => setSize([32, 30000])}>tall</button>
    </div>
    <div style={{ display: 'flex', width: 1200, height: 1200 }}>
      {mode === 'ranking' ? <AssetViewer context={{ client, projectId: 'fixture' }} asset={asset} title="Viewer fixture" disabled={false} previous={false} next={false} onNavigate={() => {}} onGrid={() => {}} /> : <BrowseStage client={client} asset={asset} />}
    </div>
  </PlatformFilesProvider>;
}
createRoot(document.getElementById('root')).render(<Fixture />);
`;
const fixturePath = resolve(run, "fixture.tsx");
await writeFile(fixturePath, fixture);
const html = `<!doctype html><html><head><title>Viewer regression</title></head><body><div id="root"></div><script type="module" src="${moduleUrl(fixturePath)}"></script></body></html>`;
let browser, server;
const checks = [],
  errors = [];
try {
  server = await createServer({
    configFile: resolve(root, "apps/desktop/vite.config.ts"),
    root: resolve(root, "apps/desktop"),
    cacheDir: resolve(run, "vite-cache"),
    server: { host: "127.0.0.1", port: 0, strictPort: false },
    plugins: [
      {
        name: "viewer-regression-fixture",
        configureServer(instance) {
          instance.middlewares.use(
            "/__viewer_test",
            async (_request, response) => {
              response.setHeader("Content-Type", "text/html");
              response.end(
                await instance.transformIndexHtml("/__viewer_test", html),
              );
            },
          );
        },
      },
    ],
  });
  await server.listen();
  browser = await chromium.launch({ ...browserOptions(), headless: true });
  const page = await browser.newPage({
    viewport: { width: 2560, height: 1440 },
    deviceScaleFactor: 1,
  });
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(
    `http://127.0.0.1:${server.httpServer.address().port}/__viewer_test`,
  );
  const region = page.getByRole("region", { name: "Viewer fixture" });
  await region.waitFor();
  const metrics = () => page.evaluate(() => ({ ...window.viewerTestMetrics }));
  await page.getByRole("button", { name: "另存原图", exact: true }).click();
  await expect.poll(async () => (await metrics()).saves).toBe(1);
  await region.focus();
  await page.keyboard.press("Control+s");
  await expect.poll(async () => (await metrics()).saves).toBe(2);
  await page.keyboard.press("Control+c");
  await expect.poll(async () => (await metrics()).copies).toBe(1);
  // Keyboard commands should also work while a viewer toolbar button has focus.
  await page.getByRole("button", { name: "原始尺寸", exact: true }).focus();
  await page.keyboard.press("Control+s");
  await expect.poll(async () => (await metrics()).saves).toBe(3);
  checks.push({
    check: "ranking toolbar and Ctrl+S/Ctrl+C dispatch to shared capabilities",
    metrics: await metrics(),
  });
  const measure = () =>
    page.evaluate(() => {
      const image = document.querySelector(".original-image img");
      if (!image?.naturalWidth) return null;
      const rect = image.getBoundingClientRect();
      return {
        width: image.naturalWidth,
        height: image.naturalHeight,
        pixelScale: Math.min(
          rect.width / image.naturalWidth,
          rect.height / image.naturalHeight,
        ),
        zoom: Number(document.querySelector(".zoom-viewport").dataset.zoom),
      };
    });
  for (const mode of ["ranking", "browse"]) {
    await page.getByRole("button", { name: mode, exact: true }).click();
    for (const [label, width, height] of [
      ["normal", 128, 160],
      ["tiny", 32, 40],
      ["pixel", 1, 1],
      ["tall", 32, 30000],
    ]) {
      await page.getByRole("button", { name: label, exact: true }).click();
      await page.getByRole("button", { name: "原始尺寸", exact: true }).click();
      await expect
        .poll(async () => {
          const value = await measure();
          return (
            value?.width === width &&
            value?.height === height &&
            Math.abs(value.pixelScale - 1) < 0.002
          );
        })
        .toBe(true);
      const actual = await measure();
      checks.push({ check: mode + " 1:1 " + label, ...actual });
      if (label === "tiny") {
        await page
          .getByRole("button", { name: "放大图片", exact: true })
          .click();
        await expect
          .poll(async () => (await measure()).pixelScale)
          .toBeCloseTo(1.25, 3);
      }
      if (label === "tall") {
        await page
          .getByRole("button", { name: "缩小图片", exact: true })
          .click();
        await expect
          .poll(async () => (await measure()).pixelScale)
          .toBeCloseTo(0.8, 3);
        await expect(
          page.getByRole("button", { name: "原始尺寸", exact: true }),
        ).toBeEnabled();
        await page
          .getByRole("button", { name: "原始尺寸", exact: true })
          .click();
        await expect
          .poll(async () => (await measure()).pixelScale)
          .toBeCloseTo(1, 3);
      }
    }
  }
  assert.deepEqual(errors, []);
  await writeFile(
    resolve(run, "report.json"),
    JSON.stringify({ passed: true, checks }, null, 2),
  );
  console.log(
    JSON.stringify(
      { passed: true, checks, report: resolve(run, "report.json") },
      null,
      2,
    ),
  );
} finally {
  await browser?.close();
  await server?.close();
}
