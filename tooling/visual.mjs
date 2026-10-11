import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { venvPython } from "./platform.mjs";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const args = process.argv.slice(2).filter((a) => a !== "--");
const option = (name) =>
  args.find((a) => a.startsWith(`--${name}=`))?.slice(name.length + 3);
const [command, id, action] = args.filter((a) => !a.startsWith("--"));
if (!command || command === "help") {
  console.log(`用法：pnpm visual <命令> [--data-dir=应用目录]
runtime | models | recipes | sources | runs | coverage | snapshots
configure --materials-root=目录 [--models-root=目录] [--python=解释器] [--device=cuda|cpu]
create --sources=来源ID,... [--models=all|siglip2_naflex,embeddinggemma2,kaloscope3|none] [--limit=每湖对象数] [--batch=256]
run 任务ID | action 任务ID pause|resume|cancel | failures 任务ID | metrics 任务ID
create | snapshot | features | export | search | duplicates | index --input=请求JSON文件
create 也可使用 --scope-run=已有任务ID，复用其全部图片范围。
不设置 create --limit 时，任务覆盖所选来源固定版本的全部对象。`);
} else {
  const directory = resolve(option("data-dir") ?? resolve(root, ".local/dev"));
  const connection = JSON.parse(
    await readFile(resolve(directory, "engine.json"), "utf8"),
  );
  const endpoint = new URL(connection.endpoint);
  if (endpoint.protocol !== "http:" || endpoint.hostname !== "127.0.0.1")
    throw new Error("引擎须为本机 HTTP 服务");
  async function request(path, body, method = "POST") {
    const response = await fetch(`${connection.endpoint}/v1/visual/${path}`, {
      method: body === undefined ? "GET" : method,
      headers: {
        Authorization: `Bearer ${connection.token}`,
        "Content-Type": "application/json",
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const value = await response.json();
    if (!response.ok) throw new Error(`${value.code}: ${value.message}`);
    return value;
  }
  let result;
  if (
    [
      "runtime",
      "models",
      "recipes",
      "sources",
      "runs",
      "coverage",
      "snapshots",
    ].includes(command)
  )
    result = await request(command);
  else if (command === "configure") {
    if (!option("materials-root"))
      throw new Error("configure 需要 --materials-root");
    result = await request(
      "runtime",
      {
        python: resolve(
          option("python") ??
            venvPython(resolve(root, ".local/runtime/visual-worker")),
        ),
        materials_root: resolve(option("materials-root")),
        model_root: resolve(option("models-root") ?? resolve(root, "Models")),
        device: option("device") ?? "cuda",
      },
      "PUT",
    );
  } else if (command === "create" && option("input")) {
    result = await request(
      "runs",
      JSON.parse(await readFile(resolve(option("input")), "utf8")),
    );
  } else if (command === "create") {
    if (!option("sources")) throw new Error("create 需要 --sources");
    const models = await request("models");
    const names = (option("models") ?? "all").split(",");
    const recipes = models.recipes.filter(
      (r) => names.includes("all") || names.includes(r.model),
    );
    if (!recipes.length && !names.includes("none"))
      throw new Error("未匹配到视觉模型");
    result = await request("runs", {
      source_ids: option("sources").split(","),
      recipe_ids: recipes.map((r) => r.id),
      max_objects_per_source: option("limit") ? Number(option("limit")) : null,
      batch_size: Number(option("batch") ?? 256),
      scope_run_id: option("scope-run") ?? null,
    });
  } else if (command === "run" && id)
    result = await request(`runs/${encodeURIComponent(id)}`);
  else if (command === "action" && id && action)
    result = await request(`runs/${encodeURIComponent(id)}/actions`, {
      action,
    });
  else if (command === "metrics" && id)
    result = await request(`runs/${encodeURIComponent(id)}/metrics`);
  else if (command === "failures" && id)
    result = await request(
      `runs/${encodeURIComponent(id)}/failures?after=${option("after") ?? 0}`,
    );
  else if (
    [
      "search",
      "duplicates",
      "index",
      "snapshot",
      "features",
      "export",
    ].includes(command) &&
    option("input")
  )
    result = await request(
      { index: "indexes", snapshot: "snapshots", export: "exports" }[command] ??
        command,
      JSON.parse(await readFile(resolve(option("input")), "utf8")),
    );
  else throw new Error("未知命令或缺少参数；运行 pnpm visual help 查看用法");
  console.log(JSON.stringify(result, null, 2));
}
