import { useMemo, useSyncExternalStore } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { StudioClient } from "@studio/client";
export const lakeKey = (client: StudioClient) =>
  ["lake-updates", client.connection.instance_id] as const;
export function useLakeStatus(client: StudioClient, visible = false) {
  return useQuery({
    queryKey: [...lakeKey(client), "status"],
    queryFn: ({ signal }) => client.lakeUpdates.status(signal),
    retry: false,
    staleTime: 1500,
    refetchInterval: (query) =>
      visible
        ? 2500
        : query.state.data?.activity?.counts.some((c) =>
              ["running", "queued", "waiting_retry", "waiting_space"].includes(
                c.state,
              ),
            )
          ? 5000
          : 15000,
  });
}
export function useLakeRefresh(client: StudioClient) {
  const cache = useQueryClient();
  return () => cache.invalidateQueries({ queryKey: lakeKey(client) });
}
export function useCollectionStatus(client: StudioClient, enabled: boolean) {
  return useQuery({
    queryKey: [...lakeKey(client), "collection-status"],
    queryFn: ({ signal }) => client.sourceCollections.status(signal),
    enabled,
    retry: false,
    refetchInterval: 5000,
  });
}
export function usePinterestStatus(client: StudioClient, enabled: boolean) {
  return useQuery({
    queryKey: [...lakeKey(client), "pinterest-status"],
    queryFn: ({ signal }) => client.pinterestCollections.status(signal),
    enabled,
    retry: false,
    refetchInterval: 5000,
  });
}
export function useLakePreference<T>(
  client: StudioClient,
  key: string,
  initial: T,
  decode: (value: unknown) => T | null,
) {
  const controller = useMemo(
    () => client.edits.preference(key, initial, decode),
    [client, key, initial, decode],
  );
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
  );
  return {
    ...state,
    controller,
    editable:
      !["loading", "unsupported", "conflict"].includes(state.status) &&
      !(state.status === "error" && !state.dirty),
  };
}
