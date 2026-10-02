import { useState } from "react";
import { Database, X } from "lucide-react";
import type { Schema } from "@studio/contracts";
import { rangeLabel, states } from "./model.js";
import { collectionRange, collectionStates } from "./collectionModel.js";
import "./lake-updates.css";
export function LakeActivity({
  status,
  collections,
  disconnected,
  onOpen,
  onProjectTasks,
  onPreparations,
}: {
  status: Schema["LakeUpdateServiceStatus"] | undefined;
  collections?: Schema["CollectionServiceStatus"] | undefined;
  disconnected: boolean;
  onOpen: (id?: string, family?: "update" | "collection") => void;
  onProjectTasks?: (() => void) | undefined;
  onPreparations: () => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const counts = status?.activity?.counts ?? [],
    running =
      counts
        .filter((c) => ["running", "queued", "waiting_retry"].includes(c.state))
        .reduce((n, c) => n + c.n, 0) +
      Object.entries(collections?.counts ?? {})
        .filter(([s]) =>
          [
            "running",
            "queued",
            "waiting_retry",
            "waiting_resources",
            "pausing",
            "publishing",
            "cancelling",
          ].includes(s),
        )
        .reduce((n, [, count]) => n + count, 0),
    attention =
      counts
        .filter((c) =>
          [
            "waiting_space",
            "waiting_credentials",
            "needs_review",
            "completed_with_exclusions",
          ].includes(c.state),
        )
        .reduce((n, c) => n + c.n, 0) +
      Object.entries(collections?.counts ?? {})
        .filter(([s]) =>
          [
            "waiting_credentials",
            "waiting_budget",
            "needs_review",
            "completed_with_gaps",
          ].includes(s),
        )
        .reduce((n, [, count]) => n + count, 0);
  const jobs = [
    ...(status?.activity?.active ?? []),
    ...(status?.activity?.attention ?? []),
  ];
  return (
    <>
      <button
        type="button"
        className="resource-drawer-trigger"
        aria-expanded={expanded}
        onClick={() => setExpanded((v) => !v)}
      >
        <Database size={14} />
        数据湖 ·{" "}
        {disconnected
          ? "连接中断"
          : `${running} 活动${attention ? ` / ${attention} 待处理` : ""}${status?.preparation_count ? ` / ${status.preparation_count} 范围准备` : ""}${status?.preparation_attention_count ? ` / ${status.preparation_attention_count} 准备异常` : ""}`}
      </button>
      {expanded && (
        <section className="lake-activity" aria-label="后台活动">
          <header>
            <strong>后台活动 · 数据湖更新</strong>
            <span className="grow" />
            {onProjectTasks && (
              <button
                onClick={() => {
                  setExpanded(false);
                  onProjectTasks();
                }}
              >
                项目任务
              </button>
            )}
            <button
              aria-label="关闭后台活动"
              onClick={() => setExpanded(false)}
            >
              <X size={14} />
            </button>
          </header>
          <div>
            {(status?.preparations ?? []).map((p) => (
              <button
                key={p.id}
                onClick={() => {
                  setExpanded(false);
                  onPreparations();
                }}
              >
                <span>
                  {p.label || "项目范围"} · {p.processed.toLocaleString()} /{" "}
                  {p.total?.toLocaleString() ?? "尚未知"}
                </span>
                <span>
                  {p.state === "needs_review"
                    ? "准备需要检查"
                    : p.state === "cancelling"
                      ? "正在取消"
                      : "正在准备"}
                </span>
              </button>
            ))}
            {collections?.active.map((j) => (
              <button
                key={j.id}
                onClick={() => {
                  setExpanded(false);
                  onOpen(j.id, "collection");
                }}
              >
                <span>Pixiv · {collectionRange(j.definition)}</span>
                <span>{collectionStates[j.state]}</span>
              </button>
            ))}
            {jobs.length ? (
              jobs.map((j) => (
                <button
                  key={j.id}
                  onClick={() => {
                    setExpanded(false);
                    onOpen(j.id);
                  }}
                >
                  <span>{rangeLabel(j.definition)}</span>
                  <span>{states[j.state]}</span>
                </button>
              ))
            ) : !collections?.active.length ? (
              <p>
                {attention
                  ? "有待处理的数据湖任务，可打开工作台查看。"
                  : "当前没有活动或待处理的数据湖任务。"}
              </p>
            ) : null}
          </div>
          <footer>
            <button
              onClick={() => {
                setExpanded(false);
                onOpen();
              }}
            >
              打开数据湖工作台
            </button>
            <span>关闭标签不会停止任务</span>
          </footer>
        </section>
      )}
    </>
  );
}
