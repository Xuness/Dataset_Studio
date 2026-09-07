import { useCallback, useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { Project } from "@studio/contracts";
import type { StudioClient } from "@studio/client";

function savedProject() {
  try {
    return localStorage.getItem("studio.last-project");
  } catch {
    return null;
  }
}
function rememberProject(id: string | null) {
  try {
    if (id) localStorage.setItem("studio.last-project", id);
    else localStorage.removeItem("studio.last-project");
  } catch {
    /* Optional local preference. */
  }
}

export function useProjectSession(
  client: StudioClient,
  onError: (message: string) => void,
) {
  const cache = useQueryClient();
  const [project, setProject] = useState<Project | null>(null);
  const [pending, setPending] = useState(false);
  const current = useRef<Project | null>(null);
  const restored = useRef(false);
  const previousClient = useRef(client);
  useEffect(() => {
    if (previousClient.current === client) return;
    previousClient.current = client;
    if (current.current)
      void client
        .openRecentProject(current.current.id)
        .catch((error) => onError(String(error)));
  }, [client, onError]);
  const projects = useQuery({
    queryKey: ["projects", client.connection.instance_id],
    queryFn: () => client.projects(),
    refetchInterval: (query) =>
      query.state.data?.items.some(
        (p) => p.state === "background" || p.state === "draining",
      )
        ? 2500
        : false,
  });
  const activate = useCallback(
    async (next: Project | null) => {
      restored.current = true;
      const old = current.current;
      if (old && old.id !== next?.id) {
        await client.closeProject(old.id);
        await cache.cancelQueries({ queryKey: ["project", old.id] });
      }
      current.current = next;
      setProject(next);
      rememberProject(next?.id ?? null);
      await cache.invalidateQueries({ queryKey: ["projects"] });
    },
    [client, cache],
  );
  const openRecent = useCallback(
    async (id: string) => {
      restored.current = true;
      setPending(true);
      onError("");
      try {
        await activate(await client.openRecentProject(id));
      } catch (error) {
        onError(error instanceof Error ? error.message : String(error));
      } finally {
        setPending(false);
        void cache.invalidateQueries({ queryKey: ["projects"] });
      }
    },
    [activate, client, onError, cache],
  );
  const close = useCallback(async () => {
    restored.current = true;
    setPending(true);
    onError("");
    try {
      await activate(null);
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    } finally {
      setPending(false);
    }
  }, [activate, onError]);
  useEffect(() => {
    if (restored.current || !projects.data) return;
    restored.current = true;
    const id = savedProject();
    if (id && projects.data.items.some((p) => p.id === id)) void openRecent(id);
  }, [projects.data, openRecent]);
  return { project, projects, pending, activate, openRecent, close };
}
