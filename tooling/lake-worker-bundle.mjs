import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readdir, readFile } from "node:fs/promises";
import { resolve } from "node:path";

// Match build.rs's ordered file identity and byte lengths. Checking the unpacked
// runtime catches a stale build-script revision as well as stale included bytes.
export async function verifyLakeWorkerBundle(root, applicationRoot) {
  const worker = resolve(root, "services/lake-worker");
  const files = ["worker.py", "pyproject.toml"];
  async function collect(relative) {
    const entries = await readdir(resolve(worker, relative), {
      withFileTypes: true,
    });
    entries.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
    for (const entry of entries) {
      const name = `${relative}/${entry.name}`;
      if (entry.isDirectory() && entry.name !== "__pycache__")
        await collect(name);
      else if (entry.isFile() && /\.(py|sql)$/.test(entry.name))
        files.push(name);
    }
  }
  await collect("src");
  const digest = createHash("sha256");
  const contents = [];
  for (const name of files) {
    const bytes = await readFile(resolve(worker, name));
    const length = Buffer.alloc(8);
    length.writeBigUInt64LE(BigInt(bytes.length));
    digest
      .update(name)
      .update(Buffer.from([0]))
      .update(length)
      .update(bytes);
    contents.push(bytes);
  }
  const revision = digest.digest("hex");
  const installed = resolve(applicationRoot, "lake-worker", revision);
  for (let i = 0; i < files.length; i++)
    assert.deepEqual(
      await readFile(resolve(installed, files[i])),
      contents[i],
      `Engine worker differs from current source: ${files[i]}`,
    );
  return { revision, files: files.length };
}
