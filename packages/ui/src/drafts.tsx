import { useMemo, useState, useSyncExternalStore } from "react";
import type { StudioClient, DraftController } from "@studio/client";
export function useDraft<T>(
  client: StudioClient,
  projectId: string,
  moduleId: string,
  initial: T,
  decode: (value: unknown) => T | null,
  instanceId = "default",
) {
  const controller = useMemo(
    () =>
      client.edits.open(
        { projectId, moduleId, instanceId, schemaVersion: 1 },
        initial,
        decode,
      ),
    [client, projectId, moduleId, initial, decode, instanceId],
  );
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
  );
  return {
    controller,
    ...state,
    editable:
      state.status !== "loading" &&
      state.status !== "unsupported" &&
      !(state.status === "error" && !state.dirty),
  };
}
export function DraftStatus<T>({
  controller,
  quiet = false,
}: {
  controller: DraftController<T>;
  quiet?: boolean;
}) {
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
  );
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  async function act(action: () => Promise<void>) {
    setPending(true);
    setError("");
    try {
      await action();
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setPending(false);
    }
  }
  const names = {
    loading: "正在恢复草稿…",
    saved: "草稿已保存",
    dirty: "草稿待保存",
    saving: "草稿保存中…",
    error: "草稿保存失败",
    conflict: "草稿存在并发冲突",
    unsupported: "原草稿已保留",
  };
  if (
    quiet &&
    !error &&
    !["error", "conflict", "unsupported"].includes(state.status)
  )
    return null;
  return (
    <div
      className={"draft-status " + (state.error ? "draft-error" : "")}
      role="status"
    >
      <span>{names[state.status]}</span>
      {state.error && <span>{state.error}</span>}
      {state.status === "conflict" && (
        <>
          <button
            type="button"
            disabled={pending}
            onClick={() => void act(() => controller.keepLocal())}
          >
            保留本地并保存
          </button>
          <button
            type="button"
            disabled={pending}
            onClick={() => void act(() => controller.reload())}
          >
            重新载入草稿
          </button>
        </>
      )}
      {state.status === "error" && (
        <button
          type="button"
          disabled={pending}
          onClick={() =>
            void act(() =>
              state.dirty ? controller.flush() : controller.reload(),
            )
          }
        >
          重试草稿保存
        </button>
      )}
      {error && <span>{error}</span>}
    </div>
  );
}
