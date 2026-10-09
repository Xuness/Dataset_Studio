import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { useQueries, useQueryClient } from "@tanstack/react-query";
import {
  ErrorDetails,
  MoreMenu,
  startObjectDrag,
  useDropZone,
  useObjectDragging,
  WorkbenchDialog,
} from "@studio/ui";
import type {
  DragObject,
  DropLocate,
  ModuleContext,
  MoreMenuItem,
} from "@studio/ui";
import type { Schema } from "@studio/contracts";
import { analysisActive } from "./analysisPresentation.js";

type Job = Schema["AestheticAnalysisJob"];
type Place = Schema["AestheticAnalysisPlace"];
type Move = { id: string; place: Place; target?: string };
const SNAPSHOT = "aesthetic.snapshot";
const JOB = "aesthetic.job";
export const isSnapshot = (job: Job) =>
  job.state === "completed" && job.result?.kind === "fit";
export const acceptsSnapshot = (object: DragObject) => object.kind === SNAPSHOT;
/** Names of the source stages of one page of jobs, read stage by stage. */
export function useStageNames(context: ModuleContext, jobs: Job[]) {
  const ids = [...new Set(jobs.map((job) => job.input.stage_id))].filter(
    Boolean,
  );
  const stages = useQueries({
    queries: ids.map((id) => ({
      queryKey: ["project", context.projectId, "aesthetic", "stage-name", id],
      queryFn: ({ signal }: { signal: AbortSignal }) =>
        context.client.aesthetic.stage(context.projectId, id, signal),
      staleTime: 60_000,
      retry: false,
    })),
  });
  const names = new Map<string, string>();
  stages.forEach((stage, index) => {
    if (stage.data)
      names.set(
        ids[index]!,
        stage.data.name + (stage.data.archived ? "（已归档）" : ""),
      );
  });
  return names;
}
const noun = (job: Job) =>
  isSnapshot(job)
    ? "排名快照"
    : job.request.spec.kind === "compare"
      ? "对照记录"
      : "离线任务";
// Snapshots keep their own kind so canvases and comparison slots accept them.
const jobDrag = (job: Job): DragObject => ({
  kind: isSnapshot(job) ? SNAPSHOT : JOB,
  id: job.id,
  label: job.request.name,
});
const rowHalf: DropLocate = (rect, _x, y) =>
  y < rect.top + rect.height / 2 ? "before" : "after";
/** Applies a pending move locally so a dropped row lands before the refetch. */
function arranged(items: Job[], move: Move | null) {
  if (!move) return items;
  const job = items.find((j) => j.id === move.id);
  const rest = items.filter((j) => j.id !== move.id);
  if (!job) return items;
  const target = rest.findIndex((j) => j.id === move.target);
  const at =
    move.place === "first"
      ? 0
      : move.place === "last"
        ? rest.length
        : target < 0
          ? -1
          : target + (move.place === "after" ? 1 : 0);
  if (at < 0) return items;
  return [...rest.slice(0, at), job, ...rest.slice(at)];
}
/** Single-job queries whose display name or listing a rename/removal changes. */
const jobQueries = [
  "analysis-jobs",
  "experiment-jobs",
  "snapshot",
  "active-analysis",
  "comparison-job",
];

export type AnalysisJobEditing = ReturnType<typeof useAnalysisJobEditing>;
/** Rename, removal and ordering state shared by every job row in one workspace. */
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
  const [moving, setMoving] = useState<Move | null>(null);
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
  async function move(next: Move) {
    if (moving) return;
    setMoving(next);
    setError(null);
    try {
      await client.aesthetic.analysis.move(
        projectId,
        next.id,
        next.place,
        next.target,
      );
      await refresh();
    } catch (failure) {
      setError(failure);
    } finally {
      setMoving(null);
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
    /** Listing order, including a move that is still being saved. */
    arrange: (items: Job[]) => arranged(items, moving),
    moving: moving !== null,
    move: (job: Job, place: Place, target?: Job) =>
      void move({
        id: job.id,
        place,
        ...(target ? { target: target.id } : {}),
      }),
  };
}

/**
 * One offline job in an outliner: click opens, F2 or double-click renames,
 * Delete asks to remove, right-click opens the menu. Dragging a row onto
 * another row of the same list, or Alt+Up/Down, rearranges the order; a
 * published snapshot can also be dragged onto a drop zone.
 */
export function AnalysisJobRow({
  list,
  job,
  siblings,
  edit,
  icon,
  detail,
  hint,
  badge,
  pressed,
  disabled = false,
  onOpen,
  actions = [],
}: {
  /** Distinguishes rows of the same job in different lists. */
  list: string;
  job: Job;
  /** The rows of this list in display order; drags reorder among them. */
  siblings: Job[];
  edit: AnalysisJobEditing;
  icon?: ReactNode;
  detail?: ReactNode;
  /** Hover text; defaults to the name. */
  hint?: string | undefined;
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
  const index = siblings.findIndex((j) => j.id === job.id);
  const previous = index > 0 ? siblings[index - 1] : undefined;
  const next = index >= 0 ? siblings[index + 1] : undefined;
  const dragged = useObjectDragging();
  // Dropping a row beside its current neighbour leaves the order unchanged.
  const unchanged = (id: string, place: string | null) => {
    const from = siblings.findIndex((j) => j.id === id);
    return place === "before" ? from === index - 1 : from === index + 1;
  };
  const drop = useDropZone(
    (object) =>
      (object.kind === SNAPSHOT || object.kind === JOB) &&
      object.id !== job.id &&
      siblings.some((j) => j.id === object.id),
    (object, place) => {
      const source = siblings.find((j) => j.id === object.id);
      if (!source || unchanged(object.id, place)) return;
      edit.move(source, place === "before" ? "before" : "after", job);
    },
    editing || edit.moving,
    rowHalf,
  );
  const insertion =
    drop.dragging && !unchanged(drop.dragging.id, drop.place)
      ? drop.place
      : null;
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
      {...drop.props}
      className="analysis-row"
      data-pressed={pressed || undefined}
      data-drop={insertion ?? undefined}
      data-dragged={dragged?.id === job.id || undefined}
      title={hint ?? name}
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
          onPointerDown={(event) => startObjectDrag(event, jobDrag(job))}
          onKeyDown={(event) => {
            if (event.altKey && event.key === "ArrowUp" && previous) {
              event.preventDefault();
              edit.move(job, "before", previous);
            }
            if (event.altKey && event.key === "ArrowDown" && next) {
              event.preventDefault();
              edit.move(job, "after", next);
            }
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
            label: "上移",
            shortcut: "Alt+↑",
            separator: actions.length > 0,
            disabled: !previous || edit.moving,
            action: () => previous && edit.move(job, "before", previous),
          },
          {
            label: "下移",
            shortcut: "Alt+↓",
            disabled: !next || edit.moving,
            action: () => next && edit.move(job, "after", next),
          },
          {
            label: "移到最前",
            disabled: edit.moving,
            action: () => edit.move(job, "first"),
          },
          {
            label: "移到最后",
            disabled: edit.moving,
            action: () => edit.move(job, "last"),
          },
          {
            label: "重命名",
            shortcut: "F2",
            separator: true,
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
