import { invoke, isTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { StudioClient, validateConnection } from "@studio/client";
export async function connectEngine() {
  const raw: unknown = isTauri()
    ? await invoke("engine_connection")
    : await fetch("/__studio/connection", { cache: "no-store" }).then((r) => {
        if (!r.ok) throw new Error("请使用 pnpm dev:web 启动本机引擎与前端。");
        return r.json() as Promise<unknown>;
      });
  const client = new StudioClient(validateConnection(raw));
  await client.health();
  return client;
}
export async function chooseDirectory(): Promise<string | null> {
  if (!isTauri()) return null;
  const result = await open({ directory: true, multiple: false });
  return typeof result === "string" ? result : null;
}
