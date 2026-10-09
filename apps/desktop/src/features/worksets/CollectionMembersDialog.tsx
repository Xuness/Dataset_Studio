import { useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button, Dialog, ErrorDetails, Field } from "@studio/ui";
import type { StudioClient } from "@studio/client";
import type { AssetKey, Schema } from "@studio/contracts";
import { ScopePicker } from "../scopes/ScopePicker.js";
import type { ScopeOption } from "../scopes/scopes.js";

export type CollectionMembersIntent = {
  operation: "add" | "remove";
  keys?: AssetKey[];
  collectionId?: string;
  scopeValue: string;
};
export function CollectionMembersDialog({
  client,
  projectId,
  intent,
  options,
  onClose,
  onApplied,
}: {
  client: StudioClient;
  projectId: string;
  intent: CollectionMembersIntent;
  options: ScopeOption[];
  onClose: () => void;
  onApplied: (result: Schema["CollectionEditResult"]) => void;
}) {
  const cache = useQueryClient();
  const collections = useQuery({
    queryKey: ["project", projectId, "collections"],
    queryFn: () => client.collections(projectId),
  });
  const [targetId, setTargetId] = useState(intent.collectionId ?? "");
  const [scopeId, setScopeId] = useState(
    options.find((o) => o.value === intent.scopeValue)?.value ??
      options[0]?.value ??
      "",
  );
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [result, setResult] = useState<Schema["CollectionEditResult"] | null>(
    null,
  );
  const [undone, setUndone] = useState(false);
  const [operationId, setOperationId] = useState<string | null>(null);
  const [captureProgress, setCaptureProgress] = useState<
    Schema["QueryResult"] | null
  >(null);
  const capture = useRef<AbortController | null>(null);
  const request = useRef<Schema["CollectionEdit"] | null>(null);
  const target = collections.data?.items.find((c) => c.id === targetId);
  const input = options.find((o) => o.value === scopeId);
  const adding = intent.operation === "add";
  const progress = useQuery({
    queryKey: ["project", projectId, "member-edit", operationId],
    queryFn: ({ signal }) =>
      client.ranking.saveProgress(projectId, operationId!, signal),
    enabled: pending && !!operationId,
    refetchInterval: pending ? 1000 : false,
  });
  function resetRequest() {
    request.current = null;
    setError(null);
  }
  async function publish(body: Schema["CollectionEdit"]) {
    setOperationId(body.request_id);
    const receipt = await client.editCollectionMembers(
      projectId,
      targetId,
      body,
    );
    setResult(receipt);
    request.current = null;
    await cache.invalidateQueries({ queryKey: ["project", projectId] });
    onApplied(receipt);
  }
  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (!target || pending) return;
    setPending(true);
    setError(null);
    try {
      if (!request.current) {
        let members: Schema["CollectionMemberInput"];
        if (intent.keys) members = { kind: "keys", keys: intent.keys };
        else {
          if (!input) throw new Error("请选择图片范围。");
          capture.current = new AbortController();
          const scope = await client.queries.fixedScope(
            projectId,
            input.scope,
            setCaptureProgress,
            capture.current.signal,
          );
          members = { kind: "scope", scope };
          capture.current = null;
        }
        request.current = {
          request_id: crypto.randomUUID(),
          expected_revision: target.revision,
          change: { kind: intent.operation, input: members },
        };
      }
      await publish(request.current);
    } catch (failure) {
      setError(failure);
      if (
        failure &&
        typeof failure === "object" &&
        "code" in failure &&
        ["REVISION_CONFLICT", "CANCELLED"].includes(String(failure.code))
      ) {
        request.current = null;
        void cache.invalidateQueries({ queryKey: ["project", projectId] });
      }
    } finally {
      capture.current = null;
      setPending(false);
      setOperationId(null);
      setCaptureProgress(null);
    }
  }
  async function undo() {
    if (!result || pending) return;
    setPending(true);
    setError(null);
    try {
      request.current ??= {
        request_id: crypto.randomUUID(),
        expected_revision: result.collection.revision,
        change: { kind: "restore", revision: result.previous_revision },
      };
      await publish(request.current);
      setUndone(true);
    } catch (failure) {
      setError(failure);
    } finally {
      setPending(false);
      setOperationId(null);
    }
  }
  async function cancel() {
    if (capture.current) capture.current.abort();
    if (operationId) {
      try {
        await client.ranking.cancelSave(projectId, operationId);
      } catch (failure) {
        setError(failure);
      }
    }
  }
  const count = intent.keys?.length ?? input?.count;
  return (
    <Dialog
      title={adding ? "加入已有工作集" : "从工作集移除"}
      onClose={() => {
        if (!pending) onClose();
      }}
    >
      {result ? (
        <div>
          <p role="status">
            {undone
              ? "已撤销本次操作。"
              : result.changed === 0
                ? "成员没有变化。"
                : `${adding ? "已加入" : "已移除"} ${result.changed.toLocaleString()} 项。`}
          </p>
          <p>
            本次操作后，“{result.collection.name}”有{" "}
            {result.collection.count.toLocaleString()} 项。
          </p>
          {!undone && result.requested > result.changed && (
            <p className="tool-hint">
              {adding ? "已在工作集中的图片" : "不在工作集中的图片"}已自动跳过。
            </p>
          )}
          {error ? <ErrorDetails error={error} /> : null}
          <div className="dialog-actions">
            {!undone && result.changed > 0 && (
              <Button disabled={pending} onClick={() => void undo()}>
                {pending ? "正在撤销…" : "撤销本次操作"}
              </Button>
            )}
            <Button disabled={pending} onClick={onClose}>
              完成
            </Button>
          </div>
        </div>
      ) : (
        <form onSubmit={(event) => void submit(event)}>
          {intent.keys ? (
            <p>已指定 {intent.keys.length.toLocaleString()} 张图片。</p>
          ) : (
            <fieldset
              disabled={pending}
              style={{ border: 0, padding: 0, margin: 0 }}
            >
              <ScopePicker
                options={options}
                value={scopeId}
                onChange={(value) => {
                  setScopeId(value);
                  resetRequest();
                }}
                label="图片范围"
              />
            </fieldset>
          )}
          <Field label="目标工作集">
            <select
              aria-label="目标工作集"
              value={targetId}
              disabled={pending}
              required
              onChange={(event) => {
                setTargetId(event.target.value);
                resetRequest();
              }}
            >
              <option value="">请选择工作集</option>
              {collections.data?.items.map((collection) => (
                <option key={collection.id} value={collection.id}>
                  {collection.name} · {collection.count.toLocaleString()} 项
                </option>
              ))}
            </select>
          </Field>
          {!collections.isPending &&
            !collections.data?.items.length &&
            !collections.error && (
              <p className="tool-hint">
                还没有工作集，请先将一个图片范围保存为工作集。
              </p>
            )}
          <p className="tool-hint">
            {adding
              ? "自动跳过已加入的图片。"
              : "只移除工作集成员，原图和项目选择会保留。"}
            完成后可以撤销本次操作。
          </p>
          {pending && (
            <p role="status">
              {captureProgress
                ? `正在固定图片范围 · 已处理 ${captureProgress.processed.toLocaleString()} 项`
                : progress.data?.total
                  ? `正在更新成员 · ${progress.data.completed.toLocaleString()} / ${progress.data.total.toLocaleString()}`
                  : "正在更新成员…"}
            </p>
          )}
          {error || collections.error ? (
            <ErrorDetails error={error ?? collections.error} />
          ) : null}
          <div className="dialog-actions">
            <Button
              type="button"
              onClick={() => (pending ? void cancel() : onClose())}
            >
              {pending ? "取消操作" : "取消"}
            </Button>
            <Button
              type="submit"
              disabled={
                pending || !target || (!intent.keys && !input) || count === 0
              }
            >
              {pending ? "处理中…" : adding ? "加入工作集" : "从工作集移除"}
            </Button>
          </div>
        </form>
      )}
    </Dialog>
  );
}
