import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
const directory = resolve(process.env.STUDIO_DATA_DIR ?? ".local/dev");
const connection = JSON.parse(
  await readFile(resolve(directory, "engine.json"), "utf8"),
);
if (!/^http:\/\/127\.0\.0\.1:\d+$/.test(connection.endpoint))
  throw new Error("Invalid engine endpoint");
const health = await fetch(connection.endpoint + "/v1/health", {
  headers: { Authorization: "Bearer " + connection.token },
});
if ((await health.json()).instance_id !== connection.instance_id)
  throw new Error("Engine instance mismatch");
const result = await fetch(connection.endpoint + "/v1/shutdown", {
  method: "POST",
  headers: { Authorization: "Bearer " + connection.token },
});
if (!result.ok) throw new Error("Engine shutdown failed");
console.log("引擎正在停止；未完成任务会在下次启动时恢复。");
