import { useQueries } from "@tanstack/react-query";
import { commonQueryFields, sourceSupports } from "@studio/client";
import type { StudioClient } from "@studio/client";
import type { Source } from "@studio/contracts";

/** Use every selected source's field contract, and refresh it with its revision. */
export function useQueryFields(
  client: StudioClient,
  projectId: string,
  sources: Source[],
  ids: string[],
) {
  const selected = ids.map((id) => sources.find((s) => s.id === id));
  const unavailable = selected.findIndex(
    (s) => !s?.available || !sourceSupports(s, "query"),
  );
  const queries = useQueries({
    queries: ids.slice(0, 8).map((id, index) => ({
      queryKey: [
        "project",
        projectId,
        "fields",
        id,
        selected[index]?.revision,
        selected[index]?.descriptor?.semantics_version,
      ],
      queryFn: ({ signal }: { signal: AbortSignal }) =>
        client.queries.fields(projectId, id, signal),
      enabled:
        !!selected[index]?.available &&
        sourceSupports(selected[index]!, "query"),
    })),
  });
  const pending = unavailable < 0 && queries.some((q) => q.isPending);
  const error =
    ids.length > 8
      ? new Error("一次查询最多选择 8 个数据湖。")
      : unavailable >= 0
        ? new Error(
            `查询来源不可用或不支持查询：${selected[unavailable]?.name ?? ids[unavailable]}`,
          )
        : queries.find((q) => q.error)?.error;
  const directories = queries.flatMap((q) => (q.data ? [q.data] : []));
  return {
    ...commonQueryFields(
      !error && directories.length === ids.length ? directories : [],
    ),
    pending,
    error,
  };
}
