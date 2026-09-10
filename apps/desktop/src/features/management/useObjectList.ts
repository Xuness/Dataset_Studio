import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { StudioClient, ObjectListOptions } from "@studio/client";
import type { ObjectKind } from "@studio/contracts";

export function useObjectList(
  client: StudioClient,
  projectId: string,
  kind: ObjectKind,
  options: ObjectListOptions = {},
  refetchInterval: number | false = false,
) {
  const [search, setSearch] = useState("");
  const [order, setOrder] =
    useState<NonNullable<ObjectListOptions["order"]>>("created_desc");
  const [cursor, setCursor] = useState<string | null>(null);
  const [history, setHistory] = useState<(string | null)[]>([]);
  const query = useQuery({
    queryKey: [
      "project",
      projectId,
      "managed-objects",
      kind,
      search,
      order,
      cursor,
      options,
    ],
    queryFn: ({ signal }) =>
      client.management.list(projectId, kind, {
        ...options,
        search,
        order,
        cursor,
        limit: options.limit ?? 32,
        signal,
      }),
    enabled: !!projectId,
    retry: false,
    refetchInterval,
  });
  function reset() {
    setCursor(null);
    setHistory([]);
  }
  return {
    query,
    search,
    order,
    cursor,
    history,
    searchFor: (value: string) => {
      setSearch(value);
      reset();
    },
    sortBy: (value: ObjectListOptions["order"]) => {
      setOrder(value ?? "created_desc");
      reset();
    },
    reset,
    next: () => {
      if (query.data?.next_cursor) {
        setHistory((old) => [...old, cursor].slice(-64));
        setCursor(query.data.next_cursor);
      }
    },
    previous: () => {
      setCursor(history.at(-1) ?? null);
      setHistory((old) => old.slice(0, -1));
    },
  };
}
