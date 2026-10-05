import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Archive, RotateCw, Calculator } from "lucide-react";
import { Button, useDraft, DraftStatus, MoreMenu } from "@studio/ui";
import type { ObjectListOptions } from "@studio/client";
import { useObjectList } from "../management/useObjectList.js";
import { objectStates, readableTime } from "../management/presentation.js";
import type { ModuleContext } from "@studio/ui";
import type { ScalarValue } from "@studio/contracts";
import "./artifacts.css";
const initial = {
  selectedId: "",
  listCursor: null as string | null,
  rowCursor: null as string | null,
};
function decode(value: unknown): typeof initial | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Record<string, unknown>;
  return typeof v.selectedId === "string" &&
    (typeof v.listCursor === "string" || v.listCursor === null) &&
    (typeof v.rowCursor === "string" || v.rowCursor === null)
    ? (value as typeof initial)
    : null;
}
function scalar(value: ScalarValue | null | undefined) {
  if (!value) return "清单行";
  switch (value.status) {
    case "available":
      return value.value;
    case "missing":
      return "字段缺失 · " + value.reason;
    case "failed":
      return "计算失败 · " + value.code;
    case "uncomputed":
      return "未计算 · 范围外或未覆盖";
  }
}
const names = {
  legacy: "旧成果待校验",
  publishing: "发布中",
  ready: "已发布",
  released: "已删除",
  unavailable: "不可用",
};
const kinds: Record<string, string> = {
  manifest: "数据清单",
  scalar_columns: "标量字段",
  item_failures: "单项失败",
  ranking_table: "元数据排名",
};
export default function ArtifactPanel(context: ModuleContext) {
  const { client, projectId } = context;
  const cache = useQueryClient();
  const draft = useDraft(client, projectId, "core.artifacts", initial, decode);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [state, setState] = useState("");
  const [kind, setKind] = useState("");
  const listing = useObjectList(client, projectId, "artifact", {
    state,
    ...(kind ? { subtype: kind } : {}),
  });
  const list = listing.query;
  const applied = useRef<number | null>(null);
  useEffect(() => {
    if (
      !draft.editable ||
      !context.invocation ||
      applied.current === context.invocation.sequence
    )
      return;
    if (context.invocation.args.artifactId)
      draft.controller.set((old) => ({
        ...old,
        selectedId: context.invocation!.args.artifactId!,
        rowCursor: null,
      }));
    applied.current = context.invocation.sequence;
  }, [context.invocation, draft.editable, draft.controller]);
  const selectedId =
    draft.value.selectedId ||
    list.data?.items.find((a) => a.subtype !== "item_failures")?.id ||
    list.data?.items[0]?.id ||
    "";
  const artifact = useQuery({
    queryKey: ["project", projectId, "artifact", selectedId],
    queryFn: ({ signal }) =>
      client.tools.artifact(projectId, selectedId, signal),
    enabled: !!selectedId && draft.editable,
  });
  const rows = useQuery({
    queryKey: [
      "project",
      projectId,
      "artifact-rows",
      selectedId,
      draft.value.rowCursor,
    ],
    queryFn: ({ signal }) =>
      client.tools.rows(projectId, selectedId, {
        ...(draft.value.rowCursor ? { cursor: draft.value.rowCursor } : {}),
        limit: 32,
        signal,
      }),
    enabled:
      artifact.data?.state === "ready" &&
      artifact.data.kind !== "ranking_table",
    gcTime: 0,
  });
  async function act(action: () => Promise<unknown>) {
    setBusy(true);
    setError("");
    try {
      await action();
      await cache.invalidateQueries({ queryKey: ["project", projectId] });
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  }
  const item = artifact.data;
  useEffect(() => {
    if (
      draft.editable &&
      item?.state === "released" &&
      state !== "released" &&
      state !== "all"
    ) {
      draft.controller.set((old) => ({
        ...old,
        selectedId:
          list.data?.items.find(
            (next) => next.id !== item.id && next.state !== "released",
          )?.id ?? "",
        rowCursor: null,
      }));
    }
  }, [
    draft.editable,
    draft.controller,
    item?.id,
    item?.state,
    state,
    list.data,
  ]);
  return (
    <section className="artifact-view" aria-label="项目成果">
      <div className="content-bar">
        <Archive size={16} />
        <strong>项目成果</strong>
        <span className="grow" />
        <Button
          onClick={() =>
            void cache.invalidateQueries({ queryKey: ["project", projectId] })
          }
        >
          <RotateCw size={12} />
          刷新
        </Button>
      </div>
      <DraftStatus controller={draft.controller} />
      <div className="artifact-columns">
        <aside>
          <div className="object-list-tools">
            <input
              type="search"
              aria-label="搜索计算成果"
              placeholder="搜索成果名称或备注"
              maxLength={120}
              value={listing.search}
              onChange={(event) => listing.searchFor(event.target.value)}
            />
            <select
              aria-label="成果排序"
              value={listing.order}
              onChange={(event) =>
                listing.sortBy(event.target.value as ObjectListOptions["order"])
              }
            >
              <option value="created_desc">最近创建</option>
              <option value="created_asc">最早创建</option>
              <option value="name_asc">名称升序</option>
              <option value="name_desc">名称降序</option>
              <option value="count_desc">成员最多</option>
            </select>
            <select
              aria-label="成果类型"
              value={kind}
              onChange={(event) => {
                setKind(event.target.value);
                listing.reset();
              }}
            >
              <option value="">全部类型</option>
              {Object.entries(kinds).map(([id, name]) => (
                <option key={id} value={id}>
                  {name}
                </option>
              ))}
            </select>
            <select
              aria-label="成果状态"
              value={state}
              onChange={(event) => {
                setState(event.target.value);
                listing.reset();
              }}
            >
              <option value="">保留的成果</option>
              <option value="ready">可用成果</option>
              <option value="released">已删除记录</option>
              <option value="all">全部记录</option>
            </select>
          </div>
          <div className="artifact-list">
            {list.data?.items.map((a) => (
              <div className="managed-list-row" key={a.id}>
                <button
                  className={
                    "artifact-item " + (a.id === selectedId ? "active" : "")
                  }
                  onClick={() =>
                    draft.controller.set((v) => ({
                      ...v,
                      selectedId: a.id,
                      rowCursor: null,
                    }))
                  }
                >
                  <strong>{a.name}</strong>
                  <span>
                    {kinds[a.subtype ?? ""] ?? a.subtype} ·{" "}
                    {a.count?.toLocaleString("zh-CN") ?? "待确认"} 项
                  </span>
                  <small>
                    {objectStates[a.state] ?? a.state} ·{" "}
                    {readableTime(a.created_at)}
                  </small>
                </button>
                <MoreMenu
                  label={a.name}
                  contextMenu
                  items={[
                    {
                      label: "管理与引用关系",
                      action: () => context.management?.open(a),
                    },
                    {
                      label: "重命名与备注…",
                      action: () => context.management?.open(a, "rename"),
                    },
                    {
                      label:
                        a.state === "released"
                          ? "检查清理状态…"
                          : "删除计算结果…",
                      danger: true,
                      action: () => context.management?.open(a, "remove"),
                    },
                  ]}
                />
              </div>
            ))}
          </div>
          <div className="artifact-paging">
            {listing.cursor && (
              <Button onClick={() => listing.previous()}>上一页</Button>
            )}
            {list.data?.next_cursor && (
              <Button onClick={() => listing.next()}>更多成果</Button>
            )}
          </div>
        </aside>
        <main>
          {!selectedId && (
            <p className="artifact-empty">运行工具后，成果会保存在这里。</p>
          )}
          {item && (
            <>
              <header className="artifact-heading">
                <div>
                  <h2>{item.name}</h2>
                  <p>
                    {names[item.state]} · {kinds[item.kind] ?? item.kind} · 结构
                    v{item.schema_version} · {item.count ?? "待确认"} 项
                  </p>
                </div>
                <span className="grow" />
                <Button
                  disabled={
                    busy ||
                    item.state === "released" ||
                    item.state === "publishing"
                  }
                  onClick={() =>
                    void act(() => client.tools.verify(projectId, item.id))
                  }
                >
                  校验文件
                </Button>
                <Button
                  disabled={
                    busy ||
                    item.state === "released" ||
                    item.state === "publishing"
                  }
                  onClick={() =>
                    context.management?.open({ kind: "artifact", id: item.id })
                  }
                >
                  管理成果
                </Button>
              </header>
              <div className="artifact-quick-actions">
                <Button
                  onClick={() =>
                    void act(() =>
                      client.management.reveal(projectId, {
                        kind: "artifact",
                        id: item.id,
                      }),
                    )
                  }
                >
                  打开文件所在位置
                </Button>
                {item.provenance.run && (
                  <Button
                    onClick={() =>
                      context.activateView("core.tools", {
                        operatorId: item.provenance.run!.operator_id,
                        reuseRun: JSON.stringify(item.provenance.run),
                      })
                    }
                  >
                    用这些参数配置新任务
                  </Button>
                )}
              </div>
              {item.issue && <p className="tool-notice">{item.issue}</p>}
              {item.kind === "ranking_table" && (
                <div className="derived-field">
                  <strong>元数据排名与筛选依据</strong>
                  <p>
                    包括固定输入、主排名、补救分、入选通道与诊断。可以将过滤后的结果保存为工作集。
                  </p>
                  <Button
                    disabled={item.state !== "ready"}
                    onClick={() =>
                      context.activateView("core.tools", {
                        operatorId: "danbooru.metarecall",
                        artifactId: item.id,
                      })
                    }
                  >
                    <Calculator size={12} />
                    查看排名榜单
                  </Button>
                </div>
              )}
              {item.kind === "scalar_columns" && (
                <div className="derived-field">
                  <strong>派生字段</strong>
                  <code>project.{item.id}.value</code>
                  <span>
                    按资产身份关联 · 有符号整数 · 覆盖 {item.count} 项
                  </span>
                  <p>
                    范围外为未计算；缺失、单项失败与实际零值分别保留。查询“未记录”仅匹配覆盖范围内的字段缺失。
                  </p>
                  <Button
                    disabled={item.state !== "ready"}
                    onClick={() =>
                      context.activateView("core.tools", {
                        operatorId: "core.scalar",
                        artifactId: item.id,
                      })
                    }
                  >
                    <Calculator size={12} />
                    用此成果计算
                  </Button>
                </div>
              )}
              <details className="artifact-provenance">
                <summary>来源与完整性依据</summary>
                <pre>
                  {JSON.stringify(
                    {
                      id: item.id,
                      job_id: item.job_id,
                      provenance: item.provenance,
                      files: item.files,
                    },
                    null,
                    2,
                  )}
                </pre>
              </details>
              {item.state === "ready" && item.kind !== "ranking_table" && (
                <>
                  <table className="artifact-table">
                    <thead>
                      <tr>
                        <th>输入序号</th>
                        <th>资产身份</th>
                        <th>值或状态</th>
                      </tr>
                    </thead>
                    <tbody>
                      {rows.data?.items.map((row) => (
                        <tr key={row.key.source_id + row.key.asset_id}>
                          <td>{row.ordinal + 1}</td>
                          <td>
                            <details>
                              <summary title={row.key.asset_id}>
                                {row.key.asset_id.slice(0, 18)}…
                              </summary>
                              <pre>{JSON.stringify(row.data, null, 2)}</pre>
                            </details>
                          </td>
                          <td>{scalar(row.scalar)}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                  <div className="artifact-paging">
                    {draft.value.rowCursor && (
                      <Button
                        onClick={() =>
                          draft.controller.set((v) => ({
                            ...v,
                            rowCursor: null,
                          }))
                        }
                      >
                        返回首批
                      </Button>
                    )}
                    {rows.data?.next_cursor && (
                      <Button
                        onClick={() =>
                          draft.controller.set((v) => ({
                            ...v,
                            rowCursor: rows.data!.next_cursor ?? null,
                          }))
                        }
                      >
                        下一批
                      </Button>
                    )}
                    <span>每批最多 32 行</span>
                  </div>
                </>
              )}
            </>
          )}
          {(error || list.error || artifact.error || rows.error) && (
            <div className="tool-notice" role="alert">
              {error ||
                list.error?.message ||
                artifact.error?.message ||
                rows.error?.message}
              {artifact.error && (
                <Button
                  onClick={() =>
                    draft.controller.set((v) => ({
                      ...v,
                      selectedId: "",
                      rowCursor: null,
                    }))
                  }
                >
                  返回可用成果
                </Button>
              )}
            </div>
          )}
        </main>
      </div>
    </section>
  );
}
