import type { Schema } from "@studio/contracts";
type Request = <T>(path: string, init?: RequestInit) => Promise<T>;
export type ObjectTarget = { kind: Schema["ObjectKind"]; id: string };
export type ObjectListOptions = {
  search?: string;
  order?:
    "name_asc" | "name_desc" | "created_asc" | "created_desc" | "count_desc";
  state?: string;
  subtype?: string;
  include_archived?: boolean;
  cursor?: string | null;
  limit?: number;
  signal?: AbortSignal;
};
const project = (id: string) => "/v1/projects/" + encodeURIComponent(id);
const object = (pid: string, target: ObjectTarget) =>
  project(pid) +
  "/objects/" +
  encodeURIComponent(target.kind) +
  "/" +
  encodeURIComponent(target.id);
function params(
  options: Record<string, string | number | boolean | null | undefined>,
) {
  const value = new URLSearchParams();
  for (const [key, item] of Object.entries(options))
    if (item !== undefined && item !== null) value.set(key, String(item));
  return value.toString();
}
export class ManagementClient {
  constructor(private readonly request: Request) {}
  list(
    pid: string,
    kind: Schema["ObjectKind"],
    options: ObjectListOptions = {},
  ) {
    const { signal, ...query } = options;
    return this.request<Schema["ObjectPage"]>(
      project(pid) +
        "/objects/" +
        encodeURIComponent(kind) +
        "?" +
        params(query),
      { signal: signal ?? null },
    );
  }
  details(pid: string, target: ObjectTarget, signal?: AbortSignal) {
    return this.request<Schema["ObjectDetails"]>(object(pid, target), {
      signal: signal ?? null,
    });
  }
  jobs(pid: string, options: ObjectListOptions = {}) {
    const { signal, ...query } = options;
    return this.request<Schema["ManagedJobPage"]>(
      project(pid) + "/job-history?" + params(query),
      { signal: signal ?? null },
    );
  }
  edit(pid: string, target: ObjectTarget, body: Schema["EditObject"]) {
    return this.request<Schema["ManagedObject"]>(object(pid, target), {
      method: "PATCH",
      body: JSON.stringify(body),
    });
  }
  links(
    pid: string,
    target: ObjectTarget,
    incoming: boolean,
    cursor?: string | null,
    signal?: AbortSignal,
  ) {
    return this.request<Schema["ObjectLinkPage"]>(
      object(pid, target) + "/links?" + params({ incoming, cursor, limit: 32 }),
      { signal: signal ?? null },
    );
  }
  action(pid: string, target: ObjectTarget, body: Schema["ObjectAction"]) {
    return this.request<Schema["OkResponse"]>(
      object(pid, target) + "/actions",
      { method: "POST", body: JSON.stringify(body) },
    );
  }
  reveal(pid: string, target: ObjectTarget, fileIndex = 0, open = true) {
    return this.request<Schema["RevealedLocation"]>(
      object(pid, target) + "/reveal",
      { method: "POST", body: JSON.stringify({ file_index: fileIndex, open }) },
    );
  }
  history(pid: string, signal?: AbortSignal) {
    return this.request<Schema["HistoryStatus"]>(
      project(pid) + "/selection/history",
      { signal: signal ?? null },
    );
  }
  restore(
    pid: string,
    action: Schema["HistoryActionKind"],
    expectedRevision: number,
  ) {
    return this.request<Schema["HistoryStatus"]>(
      project(pid) + "/selection/history",
      {
        method: "POST",
        body: JSON.stringify({ action, expected_revision: expectedRevision }),
      },
    );
  }
  editing(signal?: AbortSignal) {
    return this.request<Schema["EditingSettings"]>("/v1/settings/editing", {
      signal: signal ?? null,
    });
  }
  configureEditing(body: Schema["ConfigureEditing"]) {
    return this.request<Schema["EditingSettings"]>("/v1/settings/editing", {
      method: "PUT",
      body: JSON.stringify(body),
    });
  }
  presets(
    pid: string,
    operatorId: string,
    cursor?: string | null,
    signal?: AbortSignal,
  ) {
    return this.request<Schema["PresetPage"]>(
      project(pid) +
        "/presets?" +
        params({ operator_id: operatorId, cursor, limit: 32 }),
      { signal: signal ?? null },
    );
  }
  savePreset(pid: string, body: Schema["SaveToolPreset"]) {
    return this.request<Schema["ToolPreset"]>(project(pid) + "/presets", {
      method: "POST",
      body: JSON.stringify(body),
    });
  }
  deletePreset(pid: string, id: string, expectedRevision: number) {
    return this.request<Schema["OkResponse"]>(
      project(pid) + "/presets/" + encodeURIComponent(id) + "/delete",
      {
        method: "POST",
        body: JSON.stringify({ expected_revision: expectedRevision }),
      },
    );
  }
}
