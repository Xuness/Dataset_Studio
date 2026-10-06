import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  ErrorDetails,
  MoreMenu,
  startObjectDrag,
  WorkbenchDialog,
} from "@studio/ui";
import type { DragObject, ModuleContext, MoreMenuItem } from "@studio/ui";
import type { Schema } from "@studio/contracts";
import { analysisActive } from "./analysisPresentation.js";

type Job = Schema["AestheticAnalysisJob"];
const SNAPSHOT = "aesthetic.snapshot";
export const isSnapshot = (job: Job) =>
  job.state === "completed" && job.result?.kind === "fit";
export const acceptsSnapshot = (object: DragObject) => object.kind === SNAPSHOT;
const noun = (job: Job) =>
  isSnapshot(job)
    ? "排名快照"
    : job.request.spec.kind === "compare"
      ? "对照记录"
      : "离线任务";
const snapshotDrag = (job: Job): DragObject => ({
  kind: SNAPSHOT,
  id: job.id,
  label: job.request.name,
});
/** Single-job queries whose display name or listing a rename/removal changes. */
const jobQueries = [
  "analysis-jobs",
  "experiment-jobs",
  "snapshot",
  "active-analysis",
  "comparison-job",
];

export type AnalysisJobEditing = ReturnType<typeof useAnalysisJobEditing>;
/** Rename and removal state shared by every job row in one workspace. */
export function useAnalysisJobEditing(
  { client, projectId }: ModuleContext,
  onRemoved?: (job: Job) => void,
) {
  const cache = useQueryClient();
  // A snapshot can be listed twice (snapshots and jobs); edit only one row.
  const [renaming, setRenaming] = useState("");
  const [removing, setRemoving] = useState<Job | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  // Hide removals at once; a stale page must not reselect a removed snapshot.
  const [removed, setRemoved] = useState<ReadonlySet<string>>(new Set());
  async function refresh() {
    await Promise.all(
      jobQueries.map((key) =>
        cache.invalidateQueries({
          queryKey: ["project", projectId, "aesthetic", key],
        }),
      ),
    );
  }
  async function rename(job: Job, value: string) {
    setRenaming("");
    const name = value.trim();
    if (!name || name === job.request.name) return;
    setError(null);
    try {
      await client.aesthetic.analysis.rename(projectId, job.id, name);
      await refresh();
    } catch (failure) {
      setError(failure);
    }
  }
  async function remove() {
    if (!removing || busy) return;
    setBusy(true);
    setError(null);
    try {
      await client.aesthetic.analysis.remove(projectId, removing.id);
      const id = removing.id;
      setRemoved((old) => new Set(old).add(id));
      onRemoved?.(removing);
      setRemoving(null);
      await refresh();
    } catch (failure) {
      setError(failure);
    } finally {
      setBusy(false);
    }
  }
  const snapshot = removing && isSnapshot(removing);
  const dialog = removing && (
    <WorkbenchDialog
      title={"删除" + noun(removing)}
      onClose={() => {
        if (!busy) setRemoving(null);
      }}
    >
      <p>
        “{removing.request.name}”将从列表中移除
        {snapshot && "，之后不能再作为对照或工作集的输入"}。
      </p>
      <p className="aesthetic-help">
        已生成的对照、工作集与复核记录保持可读；评审账本保留其冻结数据，删除不会释放快照存储。
      </p>
      {error != null && <ErrorDetails error={error} />}
      <div className="wb-dialog-actions">
        <button disabled={busy} onClick={() => setRemoving(null)}>
          返回
        </button>
        <button
          className="danger-text"
          disabled={busy}
          onClick={() => void remove()}
        >
          删除
        </button>
      </div>
    </WorkbenchDialog>
  );
  return {
    renaming,
    error: removing ? null : error,
    dialog,
    startRename: (list: string, job: Job) => setRenaming(list + ":" + job.id),
    cancelRename: () => setRenaming(""),
    rename,
    askRemove: (job: Job) => {
      setError(null);
      setRemoving(job);
    },
    removable: (job: Job) => !analysisActive(job.state),
    visible: (job: Job) => !removed.has(job.id),
  };
}

/**
 * One offline job in an outliner: click opens, F2 or double-click renames,
 * Delete asks to remove, right-click opens the menu, and a published
 * snapshot can be dragged onto a drop zone.
 */
export function AnalysisJobRow({
  list,
  job,
  edit,
  icon,
  detail,
  badge,
  pressed,
  disabled = false,
  onOpen,
  actions = [],
}: {
  /** Distinguishes rows of the same job in different lists. */
  list: string;
  job: Job;
  edit: AnalysisJobEditing;
  icon?: ReactNode;
  detail?: ReactNode;
  /** Short always-visible marker, such as the comparison side. */
  badge?: string | undefined;
  pressed: boolean;
  disabled?: boolean;
  onOpen: () => void;
  actions?: MoreMenuItem[];
}) {
  const name = job.request.name;
  const editing = edit.renaming === list + ":" + job.id;
  const removable = edit.removable(job);
  const main = useRef<HTMLButtonElement>(null);
  const wasEditing = useRef(false);
  useEffect(() => {
    // Keep keyboard focus on the row once the inline editor closes, unless
    // the user already moved it elsewhere by clicking.
    const lost =
      !document.activeElement || document.activeElement === document.body;
    if (wasEditing.current && !editing && lost) main.current?.focus();
    wasEditing.current = editing;
  }, [editing]);
  return (
    <div
      className="analysis-row"
      data-pressed={pressed || undefined}
      title={name}
    >
      {editing ? (
        <span className="analysis-row-main">
          {icon}
          <RenameInput
            value={name}
            onCommit={(value) => void edit.rename(job, value)}
            onCancel={edit.cancelRename}
          />
        </span>
      ) : (
        <button
          ref={main}
          type="button"
          className="analysis-row-main"
          aria-pressed={pressed}
          disabled={disabled}
          onClick={onOpen}
          onDoubleClick={() => edit.startRename(list, job)}
          onPointerDown={(event) => {
            if (isSnapshot(job)) startObjectDrag(event, snapshotDrag(job));
          }}
          onKeyDown={(event) => {
            if (event.key === "F2") {
              event.preventDefault();
              edit.startRename(list, job);
            }
            if (event.key === "Delete" && removable) {
              event.preventDefault();
              edit.askRemove(job);
            }
          }}
        >
          {icon}
          <span>
            {name}
            {detail && <small>{detail}</small>}
          </span>
          {badge && <b className="analysis-row-badge">{badge}</b>}
        </button>
      )}
      <MoreMenu
        label={name}
        contextMenu
        disabled={editing}
        items={[
          ...actions,
          {
            label: "重命名",
            shortcut: "F2",
            separator: actions.length > 0,
            action: () => edit.startRename(list, job),
          },
          {
            label: "删除" + noun(job) + "…",
            shortcut: "Delete",
            danger: true,
            disabled: !removable,
            action: () => edit.askRemove(job),
          },
        ]}
      />
    </div>
  );
}

function RenameInput({
  value,
  onCommit,
  onCancel,
}: {
  value: string;
  onCommit: (value: string) => void;
  onCancel: () => void;
}) {
  const input = useRef<HTMLInputElement>(null);
  const done = useRef(false);
  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, []);
  function finish(commit: boolean) {
    if (done.current) return;
    done.current = true;
    if (commit) onCommit(input.current?.value ?? value);
    else onCancel();
  }
  return (
    <input
      ref={input}
      className="analysis-row-rename"
      aria-label="新名称"
      defaultValue={value}
      maxLength={120}
      onBlur={() => finish(true)}
      onKeyDown={(event) => {
        event.stopPropagation();
        if (event.key === "Enter") {
          event.preventDefault();
          finish(true);
        }
        if (event.key === "Escape") {
          event.preventDefault();
          finish(false);
        }
      }}
    />
  );
}
