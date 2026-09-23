import { useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { StudioError } from "@studio/client";
import type { Schema } from "@studio/contracts";
import {
  DraftStatus,
  ErrorDetails,
  useDraft,
  WorkbenchPanelPortal,
} from "@studio/ui";
import type { ModuleContext } from "@studio/ui";
import { CandidateCard } from "./Evidence.js";

const labels: Record<Schema["AestheticDisposition"], string> = {
  active: "正常参与",
  needs_review: "待处理",
  rejudge: "已安排重评",
  excluded: "已明确排除",
};
const reasons: Record<string, string> = {
  rating_unresolved: "Rating 未知或冲突",
  no_comparison_peer: "缺少同 Rating 的比较对手",
};
type Entry = {
  ordinal: number;
  action: "" | Schema["AestheticDispositionAction"];
  reason: string;
  key: string;
};
const initial = {
  filter: "needs_review" as "" | Schema["AestheticDisposition"],
  after: "",
  past: [] as string[],
  selected: null as number | null,
  entries: [] as Entry[],
};
function decode(value: unknown): typeof initial | null {
  if (!value || typeof value !== "object") return null;
  const v = value as typeof initial;
  return ["", ...Object.keys(labels)].includes(v.filter) &&
    typeof v.after === "string" &&
    (v.selected === null ||
      (Number.isSafeInteger(v.selected) && v.selected >= 0)) &&
    Array.isArray(v.past) &&
    v.past.length <= 64 &&
    v.past.every((s) => typeof s === "string") &&
    Array.isArray(v.entries) &&
    v.entries.length <= 32 &&
    v.entries.every(
      (e) =>
        Number.isSafeInteger(e.ordinal) &&
        e.ordinal >= 0 &&
        ["", "rejudge", "exclude"].includes(e.action) &&
        typeof e.reason === "string" &&
        e.reason.length <= 1000 &&
        typeof e.key === "string",
    )
    ? v
    : null;
}
export function CandidateQueue({
  context,
  stage,
  onInspect,
  onBusy,
  onChanged,
}: {
  context: ModuleContext;
  stage: Schema["AestheticStage"];
  onInspect: () => void;
  onBusy: (value: boolean) => void;
  onChanged: () => void;
}) {
  const { client, projectId } = context;
  const cache = useQueryClient();
  const draft = useDraft(
    client,
    projectId,
    "core.aesthetic",
    initial,
    decode,
    "candidate-queue-" + stage.id,
  );
  const saved = draft.value;
  const prefix = [
    "project",
    projectId,
    "aesthetic",
    "candidate-queue",
    stage.id,
  ];
  const rows = useQuery({
    queryKey: [...prefix, "page", saved.filter, saved.after],
    queryFn: ({ signal }) =>
      client.aesthetic.candidates(
        projectId,
        stage.id,
        false,
        saved.after || undefined,
        signal,
        saved.filter || undefined,
      ),
    enabled: draft.editable,
    refetchInterval: ["running", "preparing", "pausing", "cancelling"].includes(
      stage.state,
    )
      ? 2000
      : false,
  });
  const ordinal = saved.selected ?? rows.data?.items[0]?.ordinal;
  const row = useQuery({
    queryKey: [...prefix, "candidate", ordinal],
    queryFn: ({ signal }) =>
      client.aesthetic.candidate(projectId, stage.id, ordinal!, signal),
    enabled: ordinal !== undefined,
  });
  const entry = saved.entries.find((e) => e.ordinal === ordinal);
  const value: Entry = entry ?? {
    ordinal: ordinal ?? -1,
    action: "",
    reason: "",
    key: "",
  };
  const full = !entry && saved.entries.length >= 32;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [message, setMessage] = useState("");
  const lock = useRef(false);
  const paused = ["ready", "paused", "needs_attention", "failed"].includes(
    stage.state,
  );
  const canDecide =
    !!value.key || (paused && row.data?.disposition === "needs_review");
  const canRejudge =
    row.data &&
    ["g", "s", "q", "e"].includes(row.data.rating) &&
    stage.attempts < stage.config.request.max_calls;
  const editable = draft.editable && !busy && !full && canDecide && !value.key;
  const bytes = new TextEncoder().encode(value.reason.trim()).length;
  function edit(patch: Partial<Entry>) {
    if (!editable || ordinal === undefined) return;
    setMessage("");
    setError(null);
    draft.controller.set((old) => ({
      ...old,
      entries: [
        ...old.entries.filter((e) => e.ordinal !== ordinal),
        { ...value, ...patch, ordinal },
      ],
    }));
  }
  async function save() {
    if (
      lock.current ||
      !draft.editable ||
      !canDecide ||
      !value.action ||
      !bytes ||
      bytes > 1000 ||
      full
    )
      return;
    lock.current = true;
    setBusy(true);
    onBusy(true);
    setError(null);
    setMessage("");
    const submitted = {
      ...value,
      reason: value.reason.trim(),
      key: value.key || crypto.randomUUID(),
    };
    try {
      draft.controller.set((old) => ({
        ...old,
        selected: submitted.ordinal,
        entries: [
          ...old.entries.filter((e) => e.ordinal !== submitted.ordinal),
          submitted,
        ],
      }));
      await client.edits.flush(projectId);
      await client.aesthetic.decideCandidate(
        projectId,
        stage.id,
        submitted.ordinal,
        {
          idempotency_key: submitted.key,
          action: submitted.action as Schema["AestheticDispositionAction"],
          reason: submitted.reason,
        },
      );
      draft.controller.set((old) => ({
        ...old,
        entries: old.entries.filter((e) => e.key !== submitted.key),
      }));
      await draft.controller.flush();
      await cache.invalidateQueries({ queryKey: prefix });
      setMessage(
        submitted.action === "rejudge"
          ? "已安排重评；点击阶段的“开始评审”后派发。"
          : "已明确排除；历史评审证据保留。",
      );
      onChanged();
    } catch (e) {
      if (
        e instanceof StudioError &&
        [
          "REVISION_CONFLICT",
          "EVALUATION_RATING_UNRESOLVED",
          "INVALID_INPUT",
        ].includes(e.code)
      )
        draft.controller.set((old) => ({
          ...old,
          entries: old.entries.map((item) =>
            item.key === submitted.key ? { ...item, key: "" } : item,
          ),
        }));
      setError(e);
    } finally {
      lock.current = false;
      setBusy(false);
      onBusy(false);
    }
  }
  return (
    <>
      <div className="evaluation-queue">
        <div className="aesthetic-actions">
          <label>
            候选状态
            <select
              aria-label="候选状态"
              value={saved.filter}
              disabled={!draft.editable || busy}
              onChange={(e) =>
                draft.controller.set((old) => ({
                  ...old,
                  filter: e.target.value as typeof saved.filter,
                  after: "",
                  past: [],
                  selected: null,
                }))
              }
            >
              <option value="">全部状态</option>
              {Object.entries(labels).map(([id, label]) => (
                <option key={id} value={id}>
                  {label}
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            disabled={busy}
            onClick={() => void cache.invalidateQueries({ queryKey: prefix })}
          >
            刷新候选
          </button>
        </div>
        <p className="aesthetic-help">
          选择候选，在右侧处理原因与后续安排。此处置用于评审执行，保护提名仍在“保护复核”中处理。
        </p>
        {rows.error && <ErrorDetails error={rows.error} />}
        <div className="aesthetic-images">
          {rows.data?.items.map((candidate) => (
            <button
              type="button"
              className="evaluation-candidate"
              key={candidate.ordinal}
              aria-label={"处置候选 " + (candidate.ordinal + 1)}
              aria-pressed={ordinal === candidate.ordinal}
              disabled={!draft.editable || busy}
              onClick={() => {
                draft.controller.set((old) => ({
                  ...old,
                  selected: candidate.ordinal,
                }));
                setMessage("");
                setError(null);
                onInspect();
              }}
            >
              <CandidateCard context={context} candidate={candidate} />
              <span>{labels[candidate.disposition]}</span>
              {candidate.disposition_reason && (
                <small>
                  {reasons[candidate.disposition_reason] ??
                    candidate.disposition_reason}
                </small>
              )}
            </button>
          ))}
        </div>
        {!rows.isPending && rows.data?.items.length === 0 && (
          <p>本页没有符合该状态的候选。</p>
        )}
        <div className="aesthetic-actions">
          <button
            type="button"
            disabled={busy || !saved.after}
            onClick={() =>
              draft.controller.set((old) => ({
                ...old,
                after: "",
                past: [],
                selected: null,
              }))
            }
          >
            候选首页
          </button>
          <button
            type="button"
            disabled={busy || !saved.past.length}
            onClick={() =>
              draft.controller.set((old) => ({
                ...old,
                after: old.past.at(-1) ?? "",
                past: old.past.slice(0, -1),
                selected: null,
              }))
            }
          >
            上一页候选
          </button>
          <button
            type="button"
            disabled={busy || !rows.data?.next_cursor}
            onClick={() =>
              draft.controller.set((old) => ({
                ...old,
                after: rows.data?.next_cursor ?? "",
                past: [...old.past, old.after].slice(-64),
                selected: null,
              }))
            }
          >
            下一页候选
          </button>
        </div>
      </div>
      <WorkbenchPanelPortal id="candidate-decision">
        <div className="evaluation-decision wb-field-list">
          {row.error && <ErrorDetails error={row.error} />}
          {row.data ? (
            <>
              <h4>
                候选 {row.data.ordinal + 1} · {labels[row.data.disposition]}
              </h4>
              <p>
                {row.data.disposition_reason
                  ? (reasons[row.data.disposition_reason] ??
                    row.data.disposition_reason)
                  : "没有待处理原因"}
              </p>
              <p className="aesthetic-help">
                {row.data.rating.toUpperCase()} · 有效曝光 {row.data.exposures}
              </p>
              {["running", "preparing", "pausing", "cancelling"].includes(
                stage.state,
              ) && <p>暂停阶段并等待在途批次结束后，才能提交新的处置。</p>}
              {["completed", "completed_with_exclusions", "cancelled"].includes(
                stage.state,
              ) && (
                <p className="aesthetic-help">
                  阶段已结束，保留候选及处置记录供查看。
                </p>
              )}
              {error !== null && <ErrorDetails error={error} />}
              {message && <p role="status">{message}</p>}
              <form
                className="wb-field-list"
                onSubmit={(e) => {
                  e.preventDefault();
                  void save();
                }}
              >
                <label>
                  后续安排
                  <select
                    aria-label="候选后续安排"
                    value={value.action}
                    disabled={!editable}
                    required
                    onChange={(e) =>
                      edit({ action: e.target.value as Entry["action"] })
                    }
                  >
                    <option value="">选择处置方式</option>
                    <option value="rejudge" disabled={!canRejudge}>
                      安排重新评审
                    </option>
                    <option value="exclude">明确排除</option>
                  </select>
                </label>
                <label>
                  处置原因
                  <textarea
                    aria-label="候选处置原因"
                    rows={5}
                    value={value.reason}
                    maxLength={1000}
                    disabled={!editable}
                    required
                    onChange={(e) => edit({ reason: e.target.value })}
                  />
                </label>
                <small>{bytes} / 1000 字节</small>
                {value.action === "rejudge" && (
                  <p className="aesthetic-help">
                    后端将安排同 Rating
                    的比较对手。此操作不立即调用模型，启动阶段后消耗剩余调用预算。
                  </p>
                )}
                {value.action === "exclude" && (
                  <p className="aesthetic-help">
                    将候选从本阶段后续评审中明确排除，保留已有证据和处置原因；当前接口不提供撤销。
                  </p>
                )}
                {full && (
                  <p role="alert">
                    已有 32 份未完成处置草稿，请先处理已有草稿。
                  </p>
                )}
                {value.key && (
                  <p>正在确认上一份处置；重试沿用原请求，内容暂时锁定。</p>
                )}
                <button
                  type="submit"
                  disabled={
                    !draft.editable ||
                    busy ||
                    !canDecide ||
                    full ||
                    !value.action ||
                    !bytes ||
                    bytes > 1000
                  }
                >
                  {value.key ? "重试确认处置" : "保存候选处置"}
                </button>
              </form>
            </>
          ) : (
            <p>选择一张候选，查看处理原因。</p>
          )}
          <DraftStatus controller={draft.controller} quiet />
        </div>
      </WorkbenchPanelPortal>
    </>
  );
}
