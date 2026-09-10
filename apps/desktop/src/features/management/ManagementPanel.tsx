import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  FolderOpen,
  Link2,
  RotateCw,
  Trash2,
  ArrowLeft,
  Play,
  Archive,
} from "lucide-react";
import {
  Button,
  CopyButton,
  DraftStatus,
  ErrorDetails,
  Field,
  useDraft,
} from "@studio/ui";
import type { StudioClient, ObjectTarget } from "@studio/client";
import type {
  ObjectDetails,
  ObjectLink,
  OperatorRun,
  Schema,
} from "@studio/contracts";
import {
  objectBytes,
  objectNames,
  objectStates,
  readableTime,
} from "./presentation.js";
import "./management.css";

type Editor = {
  baseRevision: number;
  baseName: string;
  baseNotes: string;
  name: string;
  notes: string;
};
const initial: Editor = {
  baseRevision: -1,
  baseName: "",
  baseNotes: "",
  name: "",
  notes: "",
};
function decode(value: unknown): Editor | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Record<string, unknown>;
  return typeof v.baseRevision === "number" &&
    [v.baseName, v.baseNotes, v.name, v.notes].every(
      (item) => typeof item === "string",
    )
    ? (value as Editor)
    : null;
}
function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}
export type ManagementMode = "details" | "rename" | "remove";
export function InspectorTabs({
  tab,
  onChange,
}: {
  tab: "properties" | "management";
  onChange: (tab: "properties" | "management") => void;
}) {
  return (
    <header
      className="panel-tabs inspector-tabs"
      role="tablist"
      aria-label="右侧面板"
    >
      <button
        type="button"
        role="tab"
        aria-selected={tab === "properties"}
        className={tab === "properties" ? "active" : ""}
        onClick={() => onChange("properties")}
      >
        属性
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={tab === "management"}
        className={tab === "management" ? "active" : ""}
        onClick={() => onChange("management")}
      >
        管理
      </button>
    </header>
  );
}
function Provenance({ data }: { data: ObjectDetails }) {
  const p = record(data.provenance);
  if (!p) return null;
  const filter = record(p.filter);
  const conditions = (
    Array.isArray(p.conditions)
      ? p.conditions
      : Array.isArray(p.queries)
        ? p.queries.flatMap((query) => {
            const spec = record(record(query)?.spec);
            return Array.isArray(spec?.conditions) ? spec.conditions : [];
          })
        : []
  ) as unknown[];
  const fieldNames: Record<string, string> = {
    rating: "分级",
    tags: "标签",
    "post.id": "帖子 ID",
    score: "评分",
    fav_count: "收藏数",
    "source.width": "来源宽度",
    "source.height": "来源高度",
    "stored.bytes": "存储大小",
  };
  const operators: Record<string, string> = {
    eq: "等于",
    ne: "不等于",
    gte: "不小于",
    lte: "不大于",
    contains: "包含",
    contains_any: "包含任一",
    contains_all: "包含全部",
    is_missing: "未记录",
    is_present: "已记录",
  };
  const eligibility: Record<string, string> = {
    eligible: "合格候选",
    metadata_missing: "元数据缺失",
    invalid: "数据无效",
    excluded: "已排除",
  };
  return (
    <section className="management-section">
      <h4>来源与筛选条件</h4>
      {p.ranking_artifact ? (
        <p>此工作集保存自元数据排名筛选。</p>
      ) : data.object.kind === "workset" ? (
        <p>成员已固定；保存之后的选择和查询修改不会改变它。</p>
      ) : null}
      {filter && (
        <dl className="management-facts">
          <dt>分级</dt>
          <dd>
            {typeof filter.rating === "string"
              ? filter.rating.toUpperCase()
              : "全部分级"}
          </dd>
          <dt>候选资格</dt>
          <dd>
            {typeof filter.eligibility === "string"
              ? (eligibility[filter.eligibility] ?? filter.eligibility)
              : "全部"}
          </dd>
          <dt>排序依据</dt>
          <dd>
            {filter.order === "rescue"
              ? "补救排名"
              : filter.order === "input"
                ? "输入顺序"
                : "主排名"}
          </dd>
          <dt>名次限制</dt>
          <dd>
            {filter.top
              ? "每分级前 " + Number(filter.top).toLocaleString("zh-CN") + " 名"
              : "不限名次"}
          </dd>
          <dt>入选条件</dt>
          <dd>
            {filter.selected_only ? "仅已入选" : "全部"}
            {filter.route
              ? " · " +
                (
                  {
                    main: "主通道",
                    rescue: "补救通道",
                    overlap: "重叠通道",
                    none: "未入选",
                  } as Record<string, string>
                )[String(filter.route)]
              : ""}
          </dd>
        </dl>
      )}
      {conditions.length > 0 && (
        <ul className="management-conditions">
          {conditions.map((raw, index) => {
            const condition = record(raw);
            const value = record(condition?.value)?.value;
            return condition ? (
              <li key={index}>
                {fieldNames[String(condition.field)] ?? String(condition.field)}{" "}
                {operators[String(condition.operator)] ??
                  String(condition.operator)}{" "}
                {Array.isArray(value)
                  ? value.join("、")
                  : value == null
                    ? ""
                    : String(value)}
              </li>
            ) : null;
          })}
        </ul>
      )}
      {data.object.kind === "project" && (
        <p>
          {Number(p.sources ?? 0)} 个数据湖 · {Number(p.worksets ?? 0)} 个工作集
          · {Number(p.artifacts ?? 0)} 份成果
        </p>
      )}
      <details>
        <summary>完整来源记录</summary>
        <pre>{JSON.stringify(data.provenance, null, 2)}</pre>
      </details>
    </section>
  );
}
function References({
  client,
  projectId,
  target,
  incoming,
  initialItems,
  total,
  nextCursor,
  onNavigate,
}: {
  client: StudioClient;
  projectId: string;
  target: ObjectTarget;
  incoming: boolean;
  initialItems: ObjectLink[];
  total: number;
  nextCursor?: string | null;
  onNavigate: (target: ObjectTarget) => void;
}) {
  const [cursor, setCursor] = useState<string | null>(null);
  const [back, setBack] = useState<(string | null)[]>([]);
  const query = useQuery({
    queryKey: ["project", projectId, "object-links", target, incoming, cursor],
    queryFn: ({ signal }) =>
      client.management.links(projectId, target, incoming, cursor, signal),
    enabled: cursor !== null,
    retry: false,
  });
  const items = cursor === null ? initialItems : (query.data?.items ?? []);
  const next = cursor === null ? nextCursor : query.data?.next_cursor;
  return (
    <section className="management-section management-references">
      <h4>
        {incoming ? "使用它的对象" : "它来自哪里"}
        <small>{total}</small>
      </h4>
      {!total && (
        <p className="subtle">
          {incoming ? "没有其他对象引用。" : "没有额外记录的来源关系。"}
        </p>
      )}
      {query.error && <ErrorDetails error={query.error} compact />}
      {query.isFetching && <p role="status">正在读取引用…</p>}
      {items.map((link) => (
        <button
          type="button"
          className="management-reference"
          key={link.kind + link.id + link.relation}
          onClick={() =>
            onNavigate({
              kind: link.kind,
              id: link.kind === "selection_history" ? "selection" : link.id,
            })
          }
        >
          <Link2 size={13} />
          <span>
            <strong>{link.name}</strong>
            <small>
              {objectNames[link.kind]} · {link.relation}
              {link.blocking ? " · 需先处理" : ""}
            </small>
          </span>
        </button>
      ))}
      {(cursor || next) && (
        <div className="management-pagination">
          <Button
            disabled={!cursor || query.isFetching}
            onClick={() => {
              setCursor(back.at(-1) ?? null);
              setBack((old) => old.slice(0, -1));
            }}
          >
            上一页
          </Button>
          <Button
            disabled={!next || query.isFetching}
            onClick={() => {
              setBack((old) => [...old, cursor].slice(-64));
              setCursor(next!);
            }}
          >
            更多引用
          </Button>
        </div>
      )}
    </section>
  );
}
const removalText: Partial<Record<ObjectTarget["kind"], string>> = {
  source:
    "取消与当前项目的关联。已保存的工作集、计算结果和来源记录会保留；需要原图的操作在重新关联后恢复。原始数据湖和其他项目不受影响。",
  workset:
    "删除此工作集和成员记录，并解除它对查询及成果的引用。已固定输入的计算任务、其他工作集和原始图片会保留。",
  artifact:
    "删除这份计算结果的文件与索引。生成任务保留历史记录；仍被使用的结果需要先处理引用。",
  query: "移除这份保存的查询条件。已经生成的结果保留当时的条件快照。",
  query_result: "清理这份查询结果的成员引用。保存的查询条件可以再次计算。",
  job: "清理已结束任务的固定输入和暂存，并移出任务列表。清理后不能重试；仍需保留成果时，可使用归档整理任务。",
};
export function ManagementPanel({
  client,
  projectId,
  target,
  mode,
  sequence,
  onNavigate,
  onBrowse,
  onReuse,
  onChanged,
  onSettings,
}: {
  client: StudioClient;
  projectId: string;
  target: ObjectTarget;
  mode: ManagementMode;
  sequence: number;
  onNavigate: (target: ObjectTarget, mode?: ManagementMode) => void;
  onBrowse: (target: ObjectTarget) => void;
  onReuse: (run: OperatorRun) => void;
  onChanged: (target: ObjectTarget, action: string) => void;
  onSettings: () => void;
}) {
  const cache = useQueryClient();
  const query = useQuery({
    queryKey: ["project", projectId, "object", target.kind, target.id],
    queryFn: ({ signal }) =>
      client.management.details(projectId, target, signal),
    retry: false,
  });
  const data = query.data;
  const item = data?.object;
  const draft = useDraft(
    client,
    projectId,
    "core.management",
    initial,
    decode,
    target.kind + "." + target.id,
  );
  const d = draft.value;
  const dirty = d.name !== d.baseName || d.notes !== d.baseNotes;
  const [pending, setPending] = useState("");
  const [error, setError] = useState<unknown>(null);
  const [notice, setNotice] = useState("");
  const [confirm, setConfirm] = useState(mode === "remove");
  const input = useRef<HTMLInputElement>(null);
  const removal = useRef<HTMLElement>(null);
  const references = useRef<HTMLDivElement>(null);
  const editable =
    item &&
    !["selection", "selection_history", "query_result"].includes(item.kind) &&
    item.state !== "deleted";
  useEffect(() => {
    setConfirm(mode === "remove");
    requestAnimationFrame(() => {
      if (mode === "rename") {
        input.current?.focus();
        input.current?.select();
      }
      if (mode === "remove")
        removal.current?.scrollIntoView({ block: "nearest" });
    });
  }, [mode, sequence, item?.id]);
  useEffect(() => {
    if (
      draft.editable &&
      item &&
      (d.baseRevision < 0 || (!dirty && item.revision > d.baseRevision))
    )
      draft.controller.set({
        baseRevision: item.revision,
        baseName: item.name,
        baseNotes: item.notes,
        name: item.name,
        notes: item.notes,
      });
  }, [draft.editable, draft.controller, item, d.baseRevision, dirty]);
  async function act(
    name: string,
    run: () => Promise<unknown>,
    message: string,
  ) {
    if (pending) return;
    setPending(name);
    setError(null);
    setNotice("");
    try {
      await run();
      await cache.invalidateQueries({ queryKey: ["project", projectId] });
      setNotice(message);
    } catch (failure) {
      setError(failure);
      await query.refetch();
    } finally {
      setPending("");
    }
  }
  async function save() {
    if (!item) return;
    await act(
      "save",
      async () => {
        const result = await client.management.edit(projectId, target, {
          expected_revision: d.baseRevision,
          name: d.name,
          notes: d.notes,
        });
        draft.controller.set({
          baseRevision: result.revision,
          baseName: result.name,
          baseNotes: result.notes,
          name: result.name,
          notes: result.notes,
        });
        await draft.controller.flush();
        onChanged(target, "edit");
      },
      "名称和备注已更新。",
    );
  }
  function action(action: Schema["ObjectActionKind"]) {
    if (!item) return;
    void act(
      action,
      async () => {
        await client.management.action(projectId, target, {
          action,
          expected_revision: item.revision,
        });
        setConfirm(false);
        onChanged(target, action);
      },
      action === "remove"
        ? item.kind === "source"
          ? "已取消与此项目的关联。"
          : "已完成清理。"
        : action === "reconnect"
          ? "已重新关联到当前项目。"
          : action === "archive"
            ? "已归档任务，成果仍保留。"
            : "已恢复到任务列表。",
    );
  }
  const conflict =
    item && dirty && d.baseRevision >= 0 && d.baseRevision !== item.revision;
  const history = useQuery({
    queryKey: ["project", projectId, "selection-history"],
    queryFn: ({ signal }) => client.management.history(projectId, signal),
    enabled: target.kind === "selection_history" || target.kind === "selection",
  });
  if (query.isPending)
    return (
      <div className="management-empty" role="status">
        正在读取对象详情…
      </div>
    );
  if (!data || !item)
    return (
      <div className="management-empty">
        <p>对象已移除或暂时不可用。</p>
        <ErrorDetails error={query.error} compact />
        <Button onClick={() => onNavigate({ kind: "project", id: projectId })}>
          <ArrowLeft size={13} />
          返回项目管理
        </Button>
      </div>
    );
  return (
    <section className="management-panel" aria-label="对象管理">
      <div className="management-heading">
        <span>{objectNames[item.kind]}</span>
        <strong title={item.name}>{item.name}</strong>
        <Button
          disabled={!!pending || query.isFetching}
          title="刷新管理详情"
          onClick={() => void query.refetch()}
        >
          <RotateCw size={12} />
        </Button>
      </div>
      <div className="management-scroll">
        <DraftStatus controller={draft.controller} quiet />
        {(error || query.error) && (
          <ErrorDetails error={error || query.error} />
        )}
        {notice && (
          <p className="management-notice" role="status">
            {notice}
          </p>
        )}
        {editable && (
          <form
            className="management-section"
            onSubmit={(event) => {
              event.preventDefault();
              void save();
            }}
          >
            <Field label="名称">
              <input
                ref={input}
                aria-label="对象名称"
                value={d.name}
                maxLength={120}
                disabled={!draft.editable || !!pending}
                onChange={(event) =>
                  draft.controller.set((old) => ({
                    ...old,
                    name: event.target.value,
                  }))
                }
              />
            </Field>
            <Field label="备注">
              <textarea
                aria-label="对象备注"
                value={d.notes}
                rows={3}
                maxLength={4000}
                disabled={!draft.editable || !!pending}
                placeholder="记录用途、筛选思路或这次计算的区别"
                onChange={(event) =>
                  draft.controller.set((old) => ({
                    ...old,
                    notes: event.target.value,
                  }))
                }
              />
            </Field>
            {conflict && (
              <div className="management-conflict">
                <p>对象已被其他操作修改，当前编辑已保留。</p>
                <Button
                  type="button"
                  onClick={() =>
                    draft.controller.set({
                      baseRevision: item.revision,
                      baseName: item.name,
                      baseNotes: item.notes,
                      name: item.name,
                      notes: item.notes,
                    })
                  }
                >
                  重新载入
                </Button>
                <Button
                  type="button"
                  onClick={() =>
                    draft.controller.set((old) => ({
                      ...old,
                      baseRevision: item.revision,
                      baseName: item.name,
                      baseNotes: item.notes,
                    }))
                  }
                >
                  保留编辑，使用新版本
                </Button>
              </div>
            )}
            <div className="management-actions">
              <Button
                className="primary"
                type="submit"
                disabled={
                  !dirty ||
                  !d.name.trim() ||
                  !draft.editable ||
                  !!pending ||
                  !!conflict
                }
              >
                {pending === "save" ? "保存中…" : "保存修改"}
              </Button>
              {dirty && (
                <Button
                  type="button"
                  disabled={!!pending}
                  onClick={() =>
                    draft.controller.set({
                      baseRevision: item.revision,
                      baseName: item.name,
                      baseNotes: item.notes,
                      name: item.name,
                      notes: item.notes,
                    })
                  }
                >
                  放弃修改
                </Button>
              )}
            </div>
          </form>
        )}
        <section className="management-section">
          <dl className="management-facts">
            <dt>状态</dt>
            <dd>
              {objectStates[item.state] ?? item.state}
              {item.archived ? " · 已归档" : ""}
            </dd>
            {item.count != null && (
              <>
                <dt>成员数量</dt>
                <dd>{item.count.toLocaleString("zh-CN")}</dd>
              </>
            )}
            <dt>创建时间</dt>
            <dd>{readableTime(item.created_at)}</dd>
            {item.updated_at && (
              <>
                <dt>管理信息更新</dt>
                <dd>{readableTime(item.updated_at)}</dd>
              </>
            )}
            {item.bytes != null && (
              <>
                <dt>结果文件占用</dt>
                <dd>{objectBytes(item.bytes)}</dd>
              </>
            )}
          </dl>
          {[
            "workset",
            "source",
            "artifact",
            "query",
            "query_result",
            "job",
            "selection",
          ].includes(item.kind) && (
            <Button
              disabled={
                item.state === "detached" ||
                item.state === "deleted" ||
                item.state === "released"
              }
              onClick={() => onBrowse(target)}
            >
              打开此{objectNames[item.kind]}
            </Button>
          )}
          {data.run && (
            <Button disabled={!!pending} onClick={() => onReuse(data.run!)}>
              <Play size={12} />
              用这些参数配置新任务
            </Button>
          )}
        </section>
        {item.kind === "source" && item.state === "detached" && (
          <section className="management-section">
            <p>
              历史工作集和成果仍在项目内。重新关联后，可继续读取这份数据湖的图片。
            </p>
            <Button disabled={!!pending} onClick={() => action("reconnect")}>
              <Link2 size={13} />
              重新关联到项目
            </Button>
          </section>
        )}
        {item.kind === "job" &&
          ["succeeded", "failed", "cancelled"].includes(item.state) && (
            <section className="management-section">
              <Button
                disabled={!!pending}
                onClick={() => action(item.archived ? "unarchive" : "archive")}
              >
                <Archive size={13} />
                {item.archived ? "恢复到任务列表" : "归档任务记录"}
              </Button>
              <p className="subtle">归档只整理列表，输入与计算结果保持可用。</p>
            </section>
          )}
        {(item.kind === "selection_history" || item.kind === "selection") && (
          <section className="management-section">
            <h4>选择撤销</h4>
            <p>
              可撤销 {history.data?.undo_steps ?? 0} 步 · 可重做{" "}
              {history.data?.redo_steps ?? 0} 步 · 上限{" "}
              {history.data?.limit ?? 50} 步
            </p>
            <Button onClick={onSettings}>设置撤销步数</Button>
            <Button
              disabled={
                !!pending ||
                !history.data ||
                !(history.data.undo_steps || history.data.redo_steps)
              }
              onClick={() =>
                void act(
                  "clear-history",
                  async () => {
                    await client.management.restore(
                      projectId,
                      "clear",
                      history.data!.selection.revision,
                    );
                  },
                  "选择历史已清空，当前选择保持不变。",
                )
              }
            >
              清空选择历史
            </Button>
            <p className="subtle">
              清空后解除历史对查询和成果的保护，当前选择保持不变。
            </p>
          </section>
        )}
        <Provenance data={data} />
        <References
          key={target.kind + target.id + "out" + data.object.revision}
          client={client}
          projectId={projectId}
          target={target}
          incoming={false}
          initialItems={data.outgoing}
          total={data.outgoing_total}
          nextCursor={data.outgoing_cursor ?? null}
          onNavigate={onNavigate}
        />
        <div ref={references}>
          <References
            key={target.kind + target.id + "in" + data.object.revision}
            client={client}
            projectId={projectId}
            target={target}
            incoming
            initialItems={data.incoming}
            total={data.incoming_total}
            nextCursor={data.incoming_cursor ?? null}
            onNavigate={onNavigate}
          />
        </div>
        {data.paths.length > 0 && (
          <section className="management-section">
            <h4>文件位置</h4>
            {data.paths.map((path, index) => (
              <div className="management-path" key={path}>
                <strong>
                  {item.kind === "artifact"
                    ? path.endsWith(".ranking-input.sqlite")
                      ? "固定输入表"
                      : path.endsWith(".ranking.sqlite")
                        ? "排名结果表"
                        : path.endsWith(".ranking.json")
                          ? "排名摘要"
                          : path.split(/[\\/]/).at(-1)
                    : item.kind === "source"
                      ? index === 0
                        ? "索引位置"
                        : "图片湖位置"
                      : "项目目录"}
                </strong>
                <div>
                  <CopyButton text={path} label="复制路径" />
                  <Button
                    disabled={!!pending}
                    onClick={() =>
                      void act(
                        "reveal",
                        () =>
                          client.management.reveal(projectId, target, index),
                        "已打开文件所在位置。",
                      )
                    }
                  >
                    <FolderOpen size={13} />
                    打开位置
                  </Button>
                </div>
                <details>
                  <summary>完整路径</summary>
                  <code>{path}</code>
                </details>
              </div>
            ))}
          </section>
        )}
        {removalText[item.kind] &&
          item.state !== "detached" &&
          (item.state !== "deleted" || item.kind === "job") && (
            <section
              ref={removal}
              className="management-section management-removal"
            >
              <h4>{item.kind === "source" ? "取消项目关联" : "删除与清理"}</h4>
              {data.remove_reason && (
                <>
                  <p>{data.remove_reason}</p>
                  {data.incoming.some((link) => link.blocking) && (
                    <Button
                      onClick={() =>
                        references.current?.scrollIntoView({ block: "start" })
                      }
                    >
                      查看占用对象
                    </Button>
                  )}
                </>
              )}
              {confirm ? (
                <>
                  <p>{removalText[item.kind]}</p>
                  {item.kind === "artifact" && (
                    <p>本次清理结果文件 {objectBytes(item.bytes)}。</p>
                  )}
                  <p className="subtle">
                    Ctrl+Z 仅用于选择操作，不能恢复已删除对象。
                  </p>
                  <div className="management-actions">
                    <Button
                      className="danger-text"
                      disabled={!!pending || !data.can_remove}
                      onClick={() => action("remove")}
                    >
                      {pending === "remove"
                        ? "正在清理…"
                        : item.kind === "source"
                          ? "确认取消关联"
                          : item.state === "released"
                            ? "重试清理残留"
                            : "确认删除"}
                    </Button>
                    <Button
                      disabled={!!pending}
                      onClick={() => setConfirm(false)}
                    >
                      返回
                    </Button>
                  </div>
                </>
              ) : (
                <Button
                  className="danger-text"
                  disabled={
                    !!pending ||
                    (item.state === "released" && item.bytes === "0")
                  }
                  onClick={() => setConfirm(true)}
                >
                  <Trash2 size={13} />
                  {item.kind === "source"
                    ? "取消与项目关联…"
                    : item.state === "released"
                      ? "重试清理残留…"
                      : "删除此" + objectNames[item.kind] + "…"}
                </Button>
              )}
            </section>
          )}
        <details className="management-section">
          <summary>对象身份</summary>
          <code>{item.id}</code>
          <CopyButton text={item.id} label="复制对象身份" />
        </details>
      </div>
    </section>
  );
}
