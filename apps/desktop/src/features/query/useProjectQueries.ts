import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import type { StudioClient } from "@studio/client";

export function useProjectQueries(client: StudioClient, projectId: string) {
  const [cursor, setCursor] = useState<string | undefined>();
  const [definitionCursor, setDefinitionCursor] = useState<
    string | undefined
  >();
  useEffect(() => {
    setCursor(undefined);
    setDefinitionCursor(undefined);
  }, [projectId, client]);
  const definitions = useQuery({
    queryKey: ["project", projectId, "queries", definitionCursor],
    queryFn: ({ signal }) =>
      client.queries.definitions(projectId, {
        ...(definitionCursor ? { cursor: definitionCursor } : {}),
        limit: 100,
        signal,
      }),
    enabled: !!projectId,
    gcTime: 0,
  });
  const results = useQuery({
    queryKey: ["project", projectId, "query-results", cursor],
    queryFn: ({ signal }) =>
      client.queries.results(projectId, {
        ...(cursor ? { cursor } : {}),
        limit: 30,
        signal,
      }),
    enabled: !!projectId,
    gcTime: 0,
    refetchInterval: (query) =>
      query.state.data?.items.some(
        (r) => r.state === "queued" || r.state === "running",
      )
        ? 1500
        : false,
  });
  return {
    definitions,
    results,
    cursor,
    definitionCursor,
    firstDefinitions: () => setDefinitionCursor(undefined),
    nextDefinitions: () =>
      setDefinitionCursor(definitions.data?.next_cursor ?? undefined),
    firstPage: () => setCursor(undefined),
    nextPage: () => setCursor(results.data?.next_cursor ?? undefined),
  };
}
