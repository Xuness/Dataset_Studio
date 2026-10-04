import { useRef, useState } from "react";
import { aestheticTime } from "./analysisPresentation.js";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button, DraftStatus, ErrorDetails, useDraft } from "@studio/ui";
import type { ModuleContext } from "@studio/ui";
import type { RankingRow } from "./analysisPresentation.js";

type Entry = {
  snapshot: string;
  ordinal: number;
  decision: string;
  reason: string;
  key: string;
};
const initial = { reviewer: "", entries: [] as Entry[] };
const decisions: Record<string, string> = {
  protect: "保留保护",
  confirm_elite: "确认精选",
  release: "解除保护",
  defer: "暂缓决定",
};
function decode(value: unknown): typeof initial | null {
  if (!value || typeof value !== "object") return null;
  const v = value as typeof initial;
  if (
    typeof v.reviewer !== "string" ||
    v.reviewer.length > 120 ||
    !Array.isArray(v.entries) ||
    v.entries.length > 32 ||
    v.entries.some(
      (e) =>
        !e ||
        typeof e.snapshot !== "string" ||
        e.snapshot.length > 120 ||
        !Number.isSafeInteger(e.ordinal) ||
        e.ordinal < 0 ||
        !Object.hasOwn(decisions, e.decision) ||
        typeof e.reason !== "string" ||
        e.reason.length > 2000 ||
        typeof e.key !== "string" ||
        e.key.length > 120,
    )
  )
    return null;
  return v;
}
export const reviewQueryKey = (
  project: string,
  snapshot: string,
  ordinal: number,
  after = "",
) =>
  [
    "project",
    project,
    "aesthetic",
    "candidate-reviews",
    snapshot,
    ordinal,
    after,
  ] as const;
export function useCandidateReviews(
  context: ModuleContext,
  snapshot: string,
  ordinal: number | undefined,
  after = "",
) {
  return useQuery({
    queryKey: reviewQueryKey(context.projectId, snapshot, ordinal ?? -1, after),
    queryFn: ({ signal }) =>
      context.client.aesthetic.analysis.candidateReviews(
        context.projectId,
        snapshot,
        ordinal!,
        after || undefined,
        signal,
      ),
    enabled: !!snapshot && ordinal !== undefined,
  });
}
export function protectionLabel(
  decision: string | undefined,
  nominated: boolean,
) {
  return decision
    ? (decisions[decision] ?? decision)
    : nominated
      ? "已提名，待复核"
      : "未列入保护池";
}
export function RankingReviewPanel({
  context,
  snapshotId,
  row,
  next,
  onBusy,
  onSaved,
}: {
  context: ModuleContext;
  snapshotId: string;
  row: RankingRow;
  next: boolean;
  onBusy: (busy: boolean) => void;
  onSaved: (advance: boolean) => void;
}) {
  const cache = useQueryClient();
  const draft = useDraft(
    context.client,
    context.projectId,
    "core.aesthetic",
    initial,
    decode,
    "protection-review",
  );
  const entry = draft.value.entries.find(
    (e) => e.snapshot === snapshotId && e.ordinal === row.ordinal,
  );
  const empty: Entry = {
    snapshot: snapshotId,
    ordinal: row.ordinal,
    decision: "protect",
    reason: "",
    key: "",
  };
  const value = entry ?? empty;
  const full = !entry && draft.value.entries.length >= 32;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [message, setMessage] = useState("");
  const [after, setAfter] = useState("");
  const lock = useRef(false);
  const current = useCandidateReviews(context, snapshotId, row.ordinal);
  const history = useCandidateReviews(context, snapshotId, row.ordinal, after);
  const latest = current.data?.items[0];
  function edit(patch: Partial<Entry>) {
    if (!draft.editable || full || busy) return;
    setMessage("");
    setError(null);
    draft.controller.set((old) => ({
      ...old,
      entries: [
        ...old.entries.filter(
          (e) => e.snapshot !== snapshotId || e.ordinal !== row.ordinal,
        ),
        { ...value, ...patch, key: "" },
      ],
    }));
  }
  async function save(advance: boolean) {
    if (
      lock.current ||
      !draft.editable ||
      full ||
      !value.reason.trim() ||
      !draft.value.reviewer.trim()
    )
      return;
    lock.current = true;
    setBusy(true);
    onBusy(true);
    setError(null);
    setMessage("");
    const submitted = { ...value, key: value.key || crypto.randomUUID() };
    try {
      draft.controller.set((old) => ({
        ...old,
        entries: [
          ...old.entries.filter(
            (e) => e.snapshot !== snapshotId || e.ordinal !== row.ordinal,
          ),
          submitted,
        ],
      }));
      // Persist the exact retry key before an append. A lost HTTP response can be retried safely.
      await draft.controller.flush();
      await context.client.aesthetic.analysis.review(context.projectId, {
        snapshot_id: snapshotId,
        ordinal: row.ordinal,
        decision: submitted.decision,
        reviewer: draft.value.reviewer.trim(),
        reason: submitted.reason.trim(),
        idempotency_key: submitted.key,
      });
      draft.controller.set((old) => ({
        ...old,
        entries: old.entries.filter((e) => e.key !== submitted.key),
      }));
      await cache.invalidateQueries({
        queryKey: reviewQueryKey(
          context.projectId,
          snapshotId,
          row.ordinal,
        ).slice(0, -1),
      });
      setAfter("");
      setMessage("复核已保存");
      onSaved(advance);
    } catch (e) {
      setError(e);
    } finally {
      lock.current = false;
      setBusy(false);
      onBusy(false);
    }
  }
  return (
    <div className="ranking-review-panel">
      <div className="ranking-panel-identity">
        <strong>候选 {row.ordinal + 1}</strong>
        <span>
          {current.isPending
            ? "正在读取保护状态…"
            : current.isError
              ? "保护状态读取失败"
              : protectionLabel(latest?.request.decision, row.protected)}
        </span>
      </div>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void save(false);
        }}
      >
        <fieldset disabled={!draft.editable || busy || full}>
          <label>
            保护决定
            <select
              aria-label="保护决定"
              value={value.decision}
              onChange={(event) => edit({ decision: event.target.value })}
            >
              {Object.entries(decisions).map(([id, label]) => (
                <option key={id} value={id}>
                  {label}
                </option>
              ))}
            </select>
          </label>
          <label>
            复核人
            <input
              aria-label="复核人"
              required
              maxLength={120}
              value={draft.value.reviewer}
              onChange={(event) => {
                const reviewer = event.target.value;
                draft.controller.set((old) => ({
                  ...old,
                  reviewer,
                  entries: old.entries.map((e) => ({ ...e, key: "" })),
                }));
              }}
            />
          </label>
          <label>
            复核理由
            <textarea
              aria-label="复核理由"
              required
              maxLength={2000}
              rows={5}
              placeholder="记录这张图片的判断依据…"
              value={value.reason}
              onChange={(event) => edit({ reason: event.target.value })}
            />
          </label>
        </fieldset>
        {full && (
          <p className="aesthetic-help">
            已有 32 张未完成复核，请先处理已保存的草稿。
          </p>
        )}
        <div className="ranking-review-actions">
          <Button
            type="submit"
            disabled={
              busy ||
              full ||
              !draft.editable ||
              !value.reason.trim() ||
              !draft.value.reviewer.trim()
            }
          >
            保存复核决定
          </Button>
          <Button
            type="button"
            className="primary"
            disabled={
              busy ||
              full ||
              !draft.editable ||
              !next ||
              !value.reason.trim() ||
              !draft.value.reviewer.trim()
            }
            onClick={() => void save(true)}
          >
            保存并下一张
          </Button>
        </div>
        {entry && !busy && (
          <button
            type="button"
            className="ranking-discard"
            onClick={() => {
              draft.controller.set((old) => ({
                ...old,
                entries: old.entries.filter(
                  (e) => e.snapshot !== snapshotId || e.ordinal !== row.ordinal,
                ),
              }));
              setError(null);
            }}
          >
            清除此图草稿
          </button>
        )}
      </form>
      <DraftStatus controller={draft.controller} quiet />
      {message && (
        <p className="ranking-save-message" role="status">
          {message}
        </p>
      )}
      {error != null && <ErrorDetails error={error} />}
      <p className="aesthetic-help">
        草稿随图片保存。复核会追加记录；快照名次保持不变。
      </p>
      <details className="wb-fold" open>
        <summary>
          复核记录 <small>最新在前</small>
        </summary>
        <div className="ranking-review-history">
          {history.error && <ErrorDetails error={history.error} />}
          {history.isPending && <p>正在读取记录…</p>}
          {history.data?.items.length === 0 && (
            <p>这张图片还没有人工复核记录。</p>
          )}
          {history.data?.items.map((review) => (
            <article key={review.sequence}>
              <strong>
                {decisions[review.request.decision] ?? review.request.decision}
              </strong>
              <span>
                {review.request.reviewer} · {aestheticTime(review.created_at)}
              </span>
              <p>{review.request.reason}</p>
            </article>
          ))}
          <div className="aesthetic-actions">
            {after && (
              <button type="button" onClick={() => setAfter("")}>
                最新记录
              </button>
            )}
            {history.data?.next_cursor && (
              <button
                type="button"
                disabled={history.isFetching}
                onClick={() => setAfter(history.data!.next_cursor!)}
              >
                更早记录
              </button>
            )}
          </div>
        </div>
      </details>
    </div>
  );
}
