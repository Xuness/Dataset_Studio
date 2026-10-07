import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, readFile, open } from "node:fs/promises";
import { resolve, sep } from "node:path";
import { engineExecutable, engineProfile } from "./engine-profile.mjs";
import { duckdbLibrary } from "./platform.mjs";

export const sleep = (ms) => new Promise((done) => setTimeout(done, ms));
export function within(root, path) {
  const plain = (value) => {
    const path = resolve(value.startsWith("\\\\?\\") ? value.slice(4) : value);
    return process.platform === "win32" ? path.toLowerCase() : path;
  };
  assert.ok(
    plain(path).startsWith(plain(root) + sep),
    "Fixture path escapes run directory",
  );
  return path;
}
export class EngineFixture {
  constructor(root, dataDir, logDir = dataDir, options = {}) {
    this.root = root;
    this.dataDir = dataDir;
    this.logDir = logDir;
    this.options = options;
  }
  async start() {
    await mkdir(this.dataDir, { recursive: true });
    await mkdir(this.logDir, { recursive: true });
    const log = await open(resolve(this.logDir, "engine.log"), "a");
    this.child = spawn(
      this.options.executable ??
        engineExecutable(
          this.root,
          this.options.profile ?? engineProfile([], process.env, "debug"),
        ),
      [
        "serve",
        "--data-dir",
        this.dataDir,
        "--cache-dir",
        resolve(this.dataDir, "preview-cache"),
      ],
      {
        stdio: ["ignore", log.fd, log.fd],
        windowsHide: true,
        env: {
          ...process.env,
          STUDIO_DUCKDB_LIBRARY: duckdbLibrary(this.root),
          ...this.options.env,
        },
      },
    );
    await log.close();
    for (let attempt = 0; attempt < 150; attempt++) {
      try {
        const connection = JSON.parse(
          await readFile(resolve(this.dataDir, "engine.json"), "utf8"),
        );
        if (connection.pid === this.child.pid) {
          const health = await fetch(connection.endpoint + "/v1/health", {
            headers: { Authorization: "Bearer " + connection.token },
            signal: AbortSignal.timeout(700),
          });
          if (health.ok) {
            this.connection = connection;
            return;
          }
        }
      } catch {
        /* Await this fixture process, never reuse another instance. */
      }
      if (this.child.exitCode !== null)
        throw new Error(
          "Fixture engine exited: " +
            (await readFile(resolve(this.logDir, "engine.log"), "utf8")),
        );
      await sleep(100);
    }
    throw new Error("Fixture engine startup timed out");
  }
  async response(path, method = "GET", body) {
    return fetch(this.connection.endpoint + path, {
      method,
      headers: {
        Authorization: "Bearer " + this.connection.token,
        ...(body === undefined ? {} : { "Content-Type": "application/json" }),
      },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      signal: AbortSignal.timeout(30000),
    });
  }
  async api(path, method = "GET", body) {
    const response = await this.response(path, method, body);
    const value = await response.json();
    if (!response.ok)
      throw new Error(response.status + ": " + JSON.stringify(value));
    return value;
  }
  async expectError(path, method, body, code) {
    const response = await this.response(path, method, body);
    const value = await response.json();
    assert.equal(response.ok, false);
    assert.equal(value.code, code);
    return value;
  }
  async wait(path, predicate, timeout = 30000) {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) {
      const value = await this.api(path);
      if (predicate(value)) return value;
      await sleep(60);
    }
    throw new Error("Wait timed out: " + path);
  }
  async stop(crash = false) {
    const child = this.child;
    if (!child || child.exitCode !== null) return;
    const exited = new Promise((done) => child.once("exit", done));
    if (crash) child.kill();
    else await this.api("/v1/shutdown", "POST").catch(() => child.kill());
    await exited;
    this.child = undefined;
  }
}
